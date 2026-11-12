//! Host diagnostics with an actual, bounded sandbox startup probe.
//! The requested executable is only located and inspected; it is never run.

use crate::{backend, config, display, environment, inspect, policy::ContextPaths, temporary};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Return executable regular files, checking the process user's access.
#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    path.is_file() && unsafe { libc::access(cpath.as_ptr(), libc::X_OK) == 0 }
}

/// Unsupported hosts cannot certify executable availability.
#[cfg(not(unix))]
fn executable(_path: &Path) -> bool {
    false
}

/// Locate exactly as a child would, using the sanitized PATH rather than ambient PATH.
fn locate(
    program: &OsStr,
    paths: &ContextPaths,
    environment: &environment::ChildEnvironment,
) -> Result<PathBuf> {
    if program.is_empty() {
        bail!("empty executable name");
    }
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        let path = paths.cwd.join(path);
        if executable(&path) {
            return Ok(path);
        }
    } else if let Some(path) = environment.get(OsStr::new("PATH")) {
        for directory in std::env::split_paths(path) {
            let candidate = directory.join(program);
            if executable(&candidate) {
                return Ok(candidate);
            }
        }
    }
    bail!(
        "executable {:?} is missing or not executable in the filtered PATH",
        program
    )
}

/// Print a check and its specific remediation without interpreting terminal controls.
fn finding(label: &str, result: Result<String>, remedy: &str) -> bool {
    match result {
        Ok(message) => {
            println!("PASS {label}: {}", display::safe(&message));
            true
        }
        Err(error) => {
            println!("FAIL {label}: {}", display::safe(&format!("{error:#}")));
            println!("  Remedy: {}", display::safe(remedy));
            false
        }
    }
}

/// Start the embedded isolated profile through the real CLI/launch path.
/// Disposable HOME/CWD/config prevent project overrides and contain macOS scratch.
fn startup_probe() -> Result<String> {
    let root = tempfile::tempdir()?;
    let log = tempfile::tempfile()?;
    let mut child = Command::new(std::env::current_exe()?)
        .env_clear()
        .env("HOME", root.path())
        .env("NO_COLOR", "1")
        .current_dir(root.path())
        .arg("--config-dir")
        .arg(root.path().join("config"))
        .args(["-p", "isolated", "exec", "/usr/bin/true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log.try_clone()?)
        .spawn()
        .context("cannot start the sandbox probe")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("sandbox startup timed out after 10 seconds");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        use std::io::{Read, Seek, SeekFrom};
        let mut log = log;
        log.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        log.take(16384).read_to_end(&mut bytes)?;
        bail!(
            "sandbox probe exited {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
    Ok(if cfg!(target_os = "linux") {
        "isolated /usr/bin/true succeeded: user/mount/PID/network namespaces, seccomp and private tmpfs initialized"
    } else { "isolated /usr/bin/true succeeded under Seatbelt" }.into())
}

/// Map backend evidence to an explicit next step without altering host security policy.
fn startup_remedy(error: &str) -> &'static str {
    if error.contains("timed out") {
        "The probe was killed after 10 seconds. Check host load and retry from an ordinary host terminal; investigate a repeatedly stalled sandbox launcher."
    } else if error.contains("sandbox_apply") {
        "Run doctor from an ordinary macOS terminal outside any enclosing sandbox; nested Seatbelt initialization is blocked on this host."
    } else if error.contains("seccomp") {
        "Ask the host/container administrator to permit seccomp filter installation and confirm kernel seccomp support; then rerun doctor."
    } else if error.contains("unshare")
        || error.contains("uid map")
        || error.contains("namespace")
        || error.contains("Permission denied")
        || error.contains("Operation not permitted")
    {
        "Ask the host administrator to permit bubblewrap user namespaces in AppArmor/container policy. On Ubuntu, grant userns to /usr/bin/bwrap in its AppArmor profile. Retry on the host if the container forbids namespace creation."
    } else if error.contains("Unknown option") || error.contains("unrecognized option") {
        "Upgrade /usr/bin/bwrap to bubblewrap 0.6.3 or newer using your distribution's package manager."
    } else if error.contains("No such file") || error.contains("not found") {
        "Install the missing launcher or /usr/bin/true named above (Linux: bubblewrap and coreutils; macOS: restore the OS tools), then rerun doctor."
    } else {
        "Inspect the launcher error above. Retry from an ordinary terminal on supported macOS or Linux x86_64/aarch64; restore missing OS files or correct the named host restriction."
    }
}

/// Keep trust repairs distinct from missing-name and malformed-policy repairs.
fn configuration_remedy(error: &str) -> &'static str {
    if error.contains("untrusted profile path") {
        "Review the named path's owner and contents. Set ownership to the effective user or root if appropriate, and run chmod go-w on that specific path. Do not bypass trust for an unreviewed repository."
    } else if error.contains("dangling") || error.contains("not a directory") {
        "Repair the named symlink/parent directory so it resolves to the intended trusted profile location; rerun doctor."
    } else if error.contains("No such file") || error.contains("cannot read profile") {
        "Use `lilbox profile list` to choose an existing name or correct the reported extends parent. Restore the missing profile at the printed path; review any symlink target before repairing it."
    } else if error.contains("cycle") || error.contains("64 profiles") {
        "Edit the reported extends chain to remove the cycle or reduce it to at most 64 profiles; rerun `lilbox check`."
    } else {
        "Correct the reported TOML field/schema at the named source path, then run `lilbox check` and rerun doctor."
    }
}

/// Resolve common policy validation failures to the configuration knob that repairs them.
fn policy_remedy(error: &str) -> &'static str {
    if error.contains("temporary") || error.contains("scratch") || error.contains("0700") {
        "Choose an owned, non-symlink mode-0700 directory using runtime.temp_dir, or use temp_dir = \"private\". Repair the specific unsafe scratch path named above; rerun doctor."
    } else if error.contains("runtime.basic") {
        "Set runtime.basic = true for ordinary command execution in a closed profile; rerun `lilbox check`."
    } else if error.contains("keychain") {
        "Remove runtime.keychain = false from a Linux profile; that restriction is supported only on macOS."
    } else if error.contains("environment.") {
        "Correct the named environment rule: keep/drop accept ASCII whole-name globs (* and ?), set accepts valid variable names, and injection hooks cannot be set. Run `lilbox profile cani read-env NAME` for a specific permission repair."
    } else {
        "Correct the reported field or path in the selected profile; use absolute, ~/ or CWD-relative paths without .. and repair dangling links. Run `lilbox check` for individual diagnostics."
    }
}

/// Separate executable lookup failures from a located tool's static policy denial.
fn check_executable(
    program: &OsStr,
    paths: &ContextPaths,
    env: &environment::ChildEnvironment,
    loaded: &config::LoadedProfile,
    policy: &crate::policy::Policy,
) -> bool {
    let path = match locate(program, paths, env) {
        Ok(path) => path,
        Err(error) => {
            return finding(
                "executable discovery",
                Err(error),
                "Install the named program or pass its absolute path to `lilbox doctor PROGRAM`. Add its absolute bin directory to the parent PATH and keep PATH in [environment]. Check execute permission on the file and search permission on its parents.",
            );
        }
    };
    println!(
        "PASS executable discovery: {}",
        display::safe(&path.display().to_string())
    );
    let permission = (|| {
        let text = path
            .to_str()
            .context("executable path must be valid UTF-8 for policy inspection")?;
        let answer = inspect::cani(
            loaded,
            policy,
            paths,
            inspect::Capability::ExecutePath,
            Some(text),
        )?;
        if answer.allowed != Some(true) {
            bail!("{}: {}", path.display(), answer.reason);
        }
        Ok("static execute permission allowed; program and dependencies not run".into())
    })();
    finding(
        "executable permission",
        permission,
        "Add the executable's narrow directory to filesystem.allow_read. Remove or narrow any matching filesystem.deny in its defining profile; grants cannot override denies. Check `lilbox profile show --origins` and review interpreter/library grants.",
    )
}

/// Check independent prerequisites even when the selected policy cannot load.
pub fn run(
    paths: &ContextPaths,
    selected: Option<&str>,
    allow_shared: bool,
    program: Option<&OsStr>,
) -> Result<bool> {
    println!("lilbox doctor (startup probe runs only /usr/bin/true; no network request)");
    let launcher = if cfg!(target_os = "macos") {
        "/usr/bin/sandbox-exec"
    } else {
        "/usr/bin/bwrap"
    };
    let mut passed = finding(
        "backend",
        backend::current().and_then(|_| {
            if !executable(Path::new(launcher)) {
                bail!("{launcher} is missing or not executable");
            }
            Ok(launcher.into())
        }),
        if cfg!(target_os = "linux") {
            "Install bubblewrap 0.6.3+ using your distribution's package manager (Debian/Ubuntu: sudo apt install bubblewrap); verify /usr/bin/bwrap is executable."
        } else {
            "Run on supported macOS with executable /usr/bin/sandbox-exec; restore the OS tool if missing."
        },
    );
    let probe = startup_probe();
    let remedy = startup_remedy(
        probe
            .as_ref()
            .err()
            .map(|error| format!("{error:#}"))
            .as_deref()
            .unwrap_or(""),
    );
    passed &= finding("sandbox startup", probe, remedy);
    let loaded = config::load_selected(&paths.config, Some(&paths.cwd), selected, allow_shared);
    match loaded {
        Err(error) => {
            let remedy = configuration_remedy(&format!("{error:#}"));
            finding("configuration trust/resolution", Err(error), remedy);
            passed = false;
            println!(
                "SKIP executable discovery: selected configuration is invalid; repair it and rerun doctor."
            );
        }
        Ok(loaded) => {
            println!(
                "PASS configuration trust/resolution: {}",
                display::safe(&loaded.selection())
            );
            println!("  Selection: {}", display::safe(&loaded.selection_reason));
            if allow_shared {
                println!(
                    "  WARNING: --allow-shared-profiles disables ownership/mode checks; identities are still checked."
                );
            }
            let mut paths = paths.clone();
            let prepared = (|| {
                let kind = temporary::select(&loaded.profile, &mut paths)?;
                temporary::validate(kind, &paths)?;
                let policy = loaded.policy(&paths)?;
                backend::current()?.render(&policy)?;
                let env = environment::build(&loaded.profile, &paths, std::env::vars_os())?;
                Ok((policy, env))
            })();
            match prepared {
                Err(error) => {
                    let remedy = policy_remedy(&format!("{error:#}"));
                    passed &= finding("selected configuration", Err(error), remedy);
                    println!(
                        "SKIP executable discovery: repair the selected configuration and rerun doctor."
                    );
                }
                Ok((policy, env)) => {
                    let shell = std::env::var_os("SHELL")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/bin/sh".into());
                    let program = program.unwrap_or(&shell);
                    passed &= check_executable(program, &paths, &env, &loaded, &policy);
                }
            }
        }
    }
    println!(
        "The startup probe uses embedded isolated policy; it does not certify every operation or the selected program's dependencies."
    );
    Ok(passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct launch failures must not collapse to a generic installation suggestion.
    #[test]
    fn remedies_follow_the_failed_prerequisite() {
        assert!(
            startup_remedy("sandbox_apply: Operation not permitted")
                .contains("ordinary macOS terminal")
        );
        assert!(
            startup_remedy("bwrap: setting up uid map: Permission denied").contains("AppArmor")
        );
        assert!(startup_remedy("seccomp filter failed").contains("seccomp filter installation"));
        assert!(configuration_remedy("untrusted profile path /example").contains("chmod go-w"));
        assert!(configuration_remedy("inheritance cycle").contains("extends chain"));
        assert!(policy_remedy("runtime.basic required").contains("runtime.basic = true"));
    }

    /// The child PATH is authoritative and non-executable files must not be selected.
    #[test]
    #[cfg(unix)]
    fn executable_lookup_uses_filtered_path_and_execute_permission() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let tool = root.path().join("fixture-tool");
        std::fs::write(&tool, "fixture only; never executed").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o600)).unwrap();
        let paths = ContextPaths {
            home: root.path().into(),
            cwd: root.path().into(),
            config: root.path().join("config"),
            temporary: root.path().join("scratch"),
        };
        let mut env = environment::ChildEnvironment::new();
        env.insert("PATH".into(), root.path().as_os_str().into());
        assert!(locate(OsStr::new("fixture-tool"), &paths, &env).is_err());
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            locate(OsStr::new("fixture-tool"), &paths, &env).unwrap(),
            tool
        );
        env.clear();
        assert!(locate(OsStr::new("fixture-tool"), &paths, &env).is_err());
        assert_eq!(locate(tool.as_os_str(), &paths, &env).unwrap(), tool);
    }
}
