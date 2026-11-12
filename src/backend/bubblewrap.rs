//! Linux namespace and mount planning for bubblewrap, backed by a seccomp filter.
//! Missing denies are ignored at startup; existing exclusions are masked last.

use super::{Backend, SandboxCommand};
use crate::{
    config::{Mode, Network},
    policy::{Policy, canonicalize_future},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File},
    io::{Seek, Write},
    os::fd::{AsRawFd, FromRawFd},
    path::{Path, PathBuf},
    process::Command,
};

mod seccomp;

/// Stateless Linux backend; per-launch resources live in SandboxCommand.
pub struct Bubblewrap;
/// Fixed launcher location; do not resolve the enforcement tool through child PATH.
pub const EXECUTABLE: &str = "/usr/bin/bwrap";

/// Collapse lexical aliases to mount targets, including nonexistent descendants.
fn canonical(paths: &BTreeSet<PathBuf>) -> Result<BTreeSet<PathBuf>> {
    paths.iter().map(|p| canonicalize_future(p)).collect()
}

/// Test subtree membership by path components, not by a string prefix.
fn covered(path: &Path, roots: &BTreeSet<PathBuf>) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// Sort parents before descendants, with lexical ordering to break depth ties.
fn ordered(paths: &BTreeSet<PathBuf>) -> Vec<&PathBuf> {
    let mut paths: Vec<_> = paths.iter().collect();
    paths.sort_by_key(|p| (p.components().count(), *p));
    paths
}

/// Canonical grants and existing exclusions for this launch.
struct Mounts {
    /// Host subtrees exposed read-only unless also covered by a write grant.
    read: BTreeSet<PathBuf>,
    /// Host subtrees eligible for writable binds before exclusions are applied.
    write: BTreeSet<PathBuf>,
    /// Paths masked last; descendants inherit the same exclusion.
    deny: BTreeSet<PathBuf>,
}

impl Mounts {
    /// Resolve mount targets and omit absent denies without modifying their parents.
    fn new(policy: &Policy) -> Result<Self> {
        let mut deny = BTreeSet::new();
        for path in canonical(&policy.deny)? {
            if path == Path::new("/") {
                bail!("bubblewrap cannot deny the filesystem root");
            }
            if path.try_exists()? {
                deny.insert(path);
            }
        }
        let mut write = canonical(&policy.write)?;
        if let Some(path) = &policy.temporary_tmpfs {
            write.remove(&canonicalize_future(path)?);
        }
        Ok(Self {
            read: canonical(&policy.read)?,
            write,
            deny,
        })
    }

    /// A mount over a parent grant hides that host subtree. Only explicit grants
    /// below the private tmpfs can reintroduce host data, including ancestor pins.
    fn host_covered(&self, policy: &Policy, path: &Path, grants: &BTreeSet<PathBuf>) -> bool {
        grants.iter().any(|grant| {
            path.starts_with(grant)
                && !policy.temporary_tmpfs.as_ref().is_some_and(|root| {
                    path.starts_with(root) && (grant == root || !grant.starts_with(root))
                })
        })
    }

    /// Check baseline/grant write access; callers apply denies and ancestor pins.
    fn writable(&self, policy: &Policy, path: &Path) -> bool {
        (policy.mode == Mode::Open
            && !policy
                .temporary_tmpfs
                .as_ref()
                .is_some_and(|root| path.starts_with(root)))
            || self.host_covered(policy, path, &self.write)
    }

    /// Check baseline/grant visibility before masking excluded subtrees.
    fn visible(&self, policy: &Policy, path: &Path) -> bool {
        self.writable(policy, path) || self.host_covered(policy, path, &self.read)
    }
}

/// Bind a host path at the same sandbox path with the requested write permission.
fn bind(args: &mut Vec<OsString>, path: &Path, write: bool) {
    args.push(if write { "--bind" } else { "--ro-bind" }.into());
    args.push(path.into());
    args.push(path.into());
}

/// Build the ordered launcher arguments shared by inspection and execution.
/// Descriptor suppliers allow rendering placeholders without allocating memfds.
fn arguments(
    policy: &Policy,
    mut empty_fd: impl FnMut() -> Result<String>,
    seccomp_fd: &str,
) -> Result<Vec<OsString>> {
    seccomp::filter(policy.network, policy.service_ipc)?; // Also reject unsupported syscall ABIs.
    if policy.mode == Mode::Closed && !policy.basic_runtime {
        bail!(
            "bubblewrap requires runtime.basic=true in closed mode; refusing to grant implicit execution"
        );
    }
    if !policy.keychain {
        bail!(
            "runtime.keychain=false is macOS-only; Linux service IPC is controlled by runtime.service_ipc"
        );
    }
    let mounts = Mounts::new(policy)?;
    // These trees must never expose host processes, devices, or kernel controls.
    for grant in mounts.read.iter().chain(&mounts.write).chain(&mounts.deny) {
        if ["/proc", "/dev", "/sys"]
            .iter()
            .any(|root| grant.starts_with(root))
        {
            bail!(
                "bubblewrap reserves /proc, /dev and /sys for its private runtime; unsupported policy path {}",
                grant.display()
            );
        }
    }
    let mut args: Vec<OsString> = [
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        // The host's terminal process group is outside the PID namespace.
        // Detach its controlling session while retaining the stdio descriptors;
        // otherwise interactive dash can fail restoring an invisible group.
        "--new-session",
        "--cap-drop",
        "ALL",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if policy.network == Network::Deny {
        args.push("--unshare-net".into());
    }
    if policy.mode == Mode::Open {
        bind(&mut args, Path::new("/"), true);
    } else {
        // Retain symlinks such as /lib -> usr/lib. Binding both spellings as
        // independent trees could expose a denied descendant via the other one.
        let mut links = BTreeSet::new();
        let mut pending: Vec<_> = policy.read.iter().chain(&policy.write).cloned().collect();
        while let Some(path) = pending.pop() {
            for prefix in path.ancestors() {
                if fs::symlink_metadata(prefix).is_ok_and(|m| m.file_type().is_symlink())
                    && links.insert(prefix.to_path_buf())
                {
                    let target = fs::read_link(prefix)?;
                    pending.push(
                        prefix
                            .parent()
                            .context("symlink without parent")?
                            .join(&target),
                    );
                }
            }
        }
        for link in ordered(&links) {
            if covered(link, &mounts.deny) {
                continue;
            }
            args.push("--symlink".into());
            args.push(fs::read_link(link)?.into());
            args.push(link.into());
        }
    }
    // Pins prevent ancestor renames. Only bind already-granted host trees;
    // binding a synthetic ancestor in closed mode would leak ungranted files.
    let mut bindings: BTreeSet<_> = mounts.read.union(&mounts.write).cloned().collect();
    if let Some(path) = &policy.temporary_tmpfs {
        bindings.insert(path.clone());
    }
    for deny in &mounts.deny {
        bindings.extend(
            deny.ancestors()
                .skip(1)
                .filter(|p| p.parent().is_some() && p.is_dir() && mounts.visible(policy, p))
                .map(Path::to_path_buf),
        );
    }
    // Parent-first bindings prevent a later parent mount from undoing a child grant.
    for path in ordered(&bindings) {
        if covered(path, &mounts.deny) {
            continue;
        }
        if policy.temporary_tmpfs.as_ref() == Some(path) {
            args.extend([
                "--perms".into(),
                "700".into(),
                "--tmpfs".into(),
                path.into(),
            ]);
            continue;
        }
        let writable = mounts.writable(policy, path);
        if path.try_exists()? {
            bind(&mut args, path, writable);
        } else if writable {
            args.extend(["--bind-try".into(), path.into(), path.into()]);
        }
    }
    args.extend(
        [
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/sys",
            "--remount-ro",
            "/sys",
        ]
        .map(Into::into),
    );
    // Denies are last, and descendants of a denied root need no further mounts.
    let mut masked = BTreeSet::new();
    for deny in ordered(&mounts.deny) {
        if covered(deny, &masked)
            || !deny.try_exists()?
            || !(mounts.visible(policy, deny)
                || policy
                    .temporary_tmpfs
                    .as_ref()
                    .is_some_and(|path| deny.starts_with(path)))
        {
            continue;
        }
        if deny.is_dir() {
            args.extend([
                "--perms".into(),
                "000".into(),
                "--tmpfs".into(),
                deny.into(),
                "--remount-ro".into(),
                deny.into(),
            ]);
        } else {
            args.extend([
                "--perms".into(),
                "000".into(),
                "--ro-bind-data".into(),
                empty_fd()?.into(),
                deny.into(),
            ]);
        }
        masked.insert(deny.clone());
    }
    if policy.mode == Mode::Closed {
        args.extend(["--remount-ro".into(), "/".into()]);
    }
    args.extend(["--seccomp".into(), seccomp_fd.into(), "--".into()]);
    // Rebound siblings and symlink text also enter the rendered policy, even
    // when they were discovered from the filesystem rather than profile fields.
    for arg in &args {
        let text = arg
            .to_str()
            .context("bubblewrap policy paths must be UTF-8")?;
        if text.chars().any(char::is_control) {
            bail!("control character in bubblewrap policy path");
        }
    }
    Ok(args)
}

/// Store policy bytes in an anonymous file positioned for bubblewrap to read.
/// It remains close-on-exec until the launch layer explicitly preserves it.
fn memory_file(name: &std::ffi::CStr, data: &[u8]) -> Result<File> {
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd == -1 {
        return Err(std::io::Error::last_os_error())
            .context("cannot create bubblewrap policy descriptor");
    }
    // SAFETY: memfd_create returned a fresh valid descriptor; File becomes its
    // sole owner and closes it on any later setup error.
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(data)?;
    file.rewind()?;
    Ok(file)
}

impl Backend for Bubblewrap {
    /// Render shell-quoted diagnostic text; execution uses the original argv vector.
    fn render(&self, policy: &Policy) -> Result<String> {
        let args = arguments(policy, || Ok("<empty-file-fd>".into()), "<seccomp-fd>")?;
        let mut output = format!(
            "# bubblewrap arguments; fd placeholders are supplied by lilbox\n# seccomp: {}; runtime.service_ipc={}\n{}",
            seccomp::description(policy.network),
            policy.service_ipc,
            EXECUTABLE
        );
        for arg in args {
            output.push_str(if arg.to_string_lossy().starts_with("--") {
                "\n  "
            } else {
                " "
            });
            output.push('\'');
            output.push_str(&arg.to_string_lossy().replace('\'', "'\\''"));
            output.push('\'');
        }
        output.push('\n');
        Ok(output)
    }

    /// Allocate filter/mask descriptors and retain their owners through exec.
    fn command(&self, policy: &Policy) -> Result<SandboxCommand> {
        if !Path::new(EXECUTABLE).is_file() {
            bail!("/usr/bin/bwrap is unavailable; install bubblewrap; refusing to run unsandboxed");
        }
        let filter = memory_file(
            c"lilbox-seccomp",
            &seccomp::filter(policy.network, policy.service_ipc)?,
        )?;
        let filter_fd = filter.as_raw_fd().to_string();
        let mut files = vec![filter];
        let args = arguments(
            policy,
            || {
                let empty = memory_file(c"lilbox-empty", &[])?;
                let fd = empty.as_raw_fd().to_string();
                files.push(empty);
                Ok(fd)
            },
            &filter_fd,
        )?;
        let mut command = Command::new(EXECUTABLE);
        command.args(args);
        Ok(SandboxCommand { command, files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config, policy::ContextPaths};

    /// Deny ancestor pins must not restore host data hidden by a private tmpfs,
    /// even when a broader workspace/root grant covers its mount location.
    #[test]
    fn private_tmpfs_hides_parent_grants_but_allows_explicit_children() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let scratch = root.join("scratch");
        let hidden = scratch.join("hidden");
        let exposed = scratch.join("exposed");
        fs::create_dir_all(&hidden).unwrap();
        fs::create_dir(&exposed).unwrap();
        fs::write(hidden.join("secret"), "fixture").unwrap();
        let paths = ContextPaths {
            home: root.join("home"),
            cwd: root.clone(),
            config: root.join("config"),
            temporary: scratch.clone(),
        };
        for mode in ["closed", "open"] {
            let profile = config::parse(&format!(
                "version=1\nmode='{mode}'\n[filesystem]\nprotect_credentials=false\nallow_write=['${{CWD}}']\nallow_read=['scratch/exposed']\ndeny=['scratch/hidden/secret']\n[runtime]\nbasic=true\ntemporary_directory=true\ntemp_dir='private'"
            )).unwrap();
            let policy = Policy::resolve(&profile, &paths).unwrap();
            let args = arguments(&policy, || Ok("10".into()), "11").unwrap();
            let host_binds: BTreeSet<_> = args
                .windows(3)
                .filter(|args| args[0] == "--bind" || args[0] == "--ro-bind")
                .map(|args| PathBuf::from(&args[1]))
                .collect();
            assert!(!host_binds.contains(&scratch));
            assert!(!host_binds.contains(&hidden));
            assert!(host_binds.contains(&exposed));
            assert!(
                args.windows(2)
                    .any(|args| args[0] == "--tmpfs" && args[1] == scratch)
            );
        }
    }
}
