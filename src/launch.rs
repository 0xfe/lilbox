use crate::{backend::Backend, environment::ChildEnvironment, policy::Policy};
use anyhow::{Context, Result, bail};
use std::{ffi::OsString, fs, path::Path};

#[cfg(unix)]
pub fn prepare_temporary(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .context("cannot create private temporary directory")?;
    validate_temporary(path)
}

/// Inspect existing scratch storage without creating or changing it.
#[cfg(unix)]
pub fn validate_temporary(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = fs::symlink_metadata(path)?;
    // Do not chmod an existing directory: it may be someone else's or a symlink.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        bail!(
            "temporary directory must be an owned, non-symlink directory with mode 0700: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn temporary_directory_rejects_symlinks_and_insecure_permissions() {
        let root = tempfile::tempdir().unwrap();
        let scratch = root.path().join("nested/tmp");
        prepare_temporary(&scratch).unwrap();
        prepare_temporary(&scratch).unwrap();
        assert_eq!(
            fs::metadata(&scratch).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let link = root.path().join("link");
        symlink(&scratch, &link).unwrap();
        assert!(prepare_temporary(&link).is_err());
        fs::set_permissions(&scratch, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare_temporary(&scratch).is_err());
        assert_eq!(
            fs::metadata(&scratch).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}

#[cfg(not(unix))]
pub fn validate_temporary(_path: &Path) -> Result<()> {
    bail!("temporary directory validation is unsupported on this platform")
}

#[cfg(not(unix))]
pub fn prepare_temporary(_path: &Path) -> Result<()> {
    bail!("no sandbox backend for this platform")
}

#[cfg(unix)]
pub fn execute(
    backend: &dyn Backend,
    policy: &Policy,
    argv: &[OsString],
    environment: ChildEnvironment,
) -> Result<()> {
    use std::os::unix::process::CommandExt;
    if argv.is_empty() || argv[0].is_empty() {
        bail!("missing command");
    }
    let mut command = backend.command(policy)?;
    command.args(argv).env_clear().envs(environment);
    // Enumerate before pre_exec: no allocation, locks, or directory I/O in that
    // callback. lilbox is single-threaded and opens no files after this point.
    let descriptors: Vec<i32> = fs::read_dir("/dev/fd")
        .context("cannot enumerate inherited file descriptors")?
        .map(|entry| -> Result<Option<i32>> {
            let entry = entry?;
            Ok(entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
                .filter(|fd| *fd > 2))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    unsafe {
        command.pre_exec(move || {
            libc::umask(0o077);
            // Prevent secrets from being written into core dumps.
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            for fd in &descriptors {
                libc::close(*fd);
            }
            Ok(())
        });
    }
    Err(command.exec())
        .with_context(|| format!("failed to launch {} in sandbox", argv[0].to_string_lossy()))
}

#[cfg(not(unix))]
pub fn execute(
    _backend: &dyn Backend,
    _policy: &Policy,
    _argv: &[OsString],
    _environment: ChildEnvironment,
) -> Result<()> {
    bail!("no sandbox backend for this platform")
}
