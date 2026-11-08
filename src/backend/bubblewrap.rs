//! Mount-based filesystem policy. Missing denies require conservative read-only
//! parents: creating mountpoints in a host bind would modify the host filesystem.
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

pub struct Bubblewrap;
pub const EXECUTABLE: &str = "/usr/bin/bwrap";

fn canonical(paths: &BTreeSet<PathBuf>) -> Result<BTreeSet<PathBuf>> {
    paths.iter().map(|p| canonicalize_future(p)).collect()
}

fn covered(path: &Path, roots: &BTreeSet<PathBuf>) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn ordered(paths: &BTreeSet<PathBuf>) -> Vec<&PathBuf> {
    let mut paths: Vec<_> = paths.iter().collect();
    paths.sort_by_key(|p| (p.components().count(), *p));
    paths
}

struct Mounts {
    read: BTreeSet<PathBuf>,
    write: BTreeSet<PathBuf>,
    deny: BTreeSet<PathBuf>,
    restricted: BTreeSet<PathBuf>,
}

impl Mounts {
    fn new(policy: &Policy) -> Result<Self> {
        let mut mounts = Self {
            read: canonical(&policy.read)?,
            write: canonical(&policy.write)?,
            deny: canonical(&policy.deny)?,
            restricted: BTreeSet::new(),
        };
        for deny in &mounts.deny {
            if deny == Path::new("/") {
                bail!("bubblewrap cannot deny the filesystem root");
            }
            if !deny.try_exists()? {
                let parent = deny
                    .ancestors()
                    .skip(1)
                    .find(|p| p.exists())
                    .context("missing deny ancestor")?;
                if mounts.writable(policy, parent) && !covered(parent, &mounts.deny) {
                    mounts.restricted.insert(parent.into());
                }
            }
        }
        Ok(mounts)
    }

    fn writable(&self, policy: &Policy, path: &Path) -> bool {
        policy.mode == Mode::Open || covered(path, &self.write)
    }

    fn visible(&self, policy: &Policy, path: &Path) -> bool {
        self.writable(policy, path) || covered(path, &self.read)
    }
}

/// Exposed to read-only diagnostics; never creates mountpoints or scratch files.
pub fn restricted_parents(policy: &Policy) -> Result<Vec<PathBuf>> {
    Ok(Mounts::new(policy)?.restricted.into_iter().collect())
}

fn bind(args: &mut Vec<OsString>, path: &Path, write: bool) {
    args.push(if write { "--bind" } else { "--ro-bind" }.into());
    args.push(path.into());
    args.push(path.into());
}

fn arguments(
    policy: &Policy,
    mut empty_fd: impl FnMut() -> Result<String>,
    seccomp_fd: &str,
) -> Result<Vec<OsString>> {
    seccomp::filter(policy.network)?; // Also reject unsupported syscall ABIs.
    if policy.mode == Mode::Closed && !policy.basic_runtime {
        bail!(
            "bubblewrap requires runtime.basic=true in closed mode; refusing to grant implicit execution"
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
    let mut bindings: BTreeSet<_> = if policy.mode == Mode::Closed {
        mounts.read.union(&mounts.write).cloned().collect()
    } else {
        BTreeSet::new()
    };
    for deny in &mounts.deny {
        bindings.extend(
            deny.ancestors()
                .skip(1)
                .filter(|p| p.parent().is_some() && p.is_dir() && mounts.visible(policy, p))
                .map(Path::to_path_buf),
        );
    }
    // Reserve absent deny names without creating anything on the host. Existing
    // allowed children keep their original permissions; directory entry changes
    // in the parent remain prohibited (including atomic replacement of files).
    for parent in ordered(&mounts.restricted) {
        bindings.insert(parent.clone());
        let mut children = fs::read_dir(parent)?
            .map(|entry| Ok(entry?.path()))
            .collect::<Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            let metadata = fs::symlink_metadata(&child)?;
            if !metadata.file_type().is_symlink() && !covered(&child, &mounts.deny) {
                bindings.insert(child);
            }
        }
    }
    // One parent-first order for grants, pins, and restricted parents prevents
    // a later parent bind from undoing child write grants or rename protection.
    for path in ordered(&bindings) {
        if covered(path, &mounts.deny) {
            continue;
        }
        let writable = mounts.writable(policy, path) && !mounts.restricted.contains(path);
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
        if covered(deny, &masked) || !deny.try_exists()? || !mounts.visible(policy, deny) {
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

fn memory_file(name: &std::ffi::CStr, data: &[u8]) -> Result<File> {
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd == -1 {
        return Err(std::io::Error::last_os_error())
            .context("cannot create bubblewrap policy descriptor");
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(data)?;
    file.rewind()?;
    Ok(file)
}

impl Backend for Bubblewrap {
    fn render(&self, policy: &Policy) -> Result<String> {
        let args = arguments(policy, || Ok("<empty-file-fd>".into()), "<seccomp-fd>")?;
        let mut output = format!(
            "# bubblewrap arguments; fd placeholders are supplied by lilbox\n# seccomp: {}\n{}",
            seccomp::description(policy.network),
            EXECUTABLE
        );
        for arg in args {
            output.push(' ');
            output.push('\'');
            output.push_str(&arg.to_string_lossy().replace('\'', "'\\''"));
            output.push('\'');
        }
        output.push('\n');
        Ok(output)
    }

    fn command(&self, policy: &Policy) -> Result<SandboxCommand> {
        if !Path::new(EXECUTABLE).is_file() {
            bail!("/usr/bin/bwrap is unavailable; install bubblewrap; refusing to run unsandboxed");
        }
        let filter = memory_file(c"lilbox-seccomp", &seccomp::filter(policy.network)?)?;
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
