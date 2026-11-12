//! Select scratch storage without side effects; create it only for a real launch.
//! Linux private storage lives in the mount namespace. macOS uses a fresh private
//! directory retained after exec; explicit/project/system paths are reusable.

use crate::{
    config::Profile,
    policy::{ContextPaths, expand_path},
};
use anyhow::{Context, Result, bail};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::PathBuf};

/// Storage semantics are separate from the path supplied to the child environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// No temporary environment variables or filesystem grant.
    Disabled,
    /// Invocation-specific Linux tmpfs or macOS disk directory.
    Private,
    /// Reusable project or explicitly selected private directory.
    Directory,
    /// Shared system temporary storage, explicitly requested.
    System,
}

/// Choose effective scratch semantics and update the shared inspection/launch path.
pub fn select(profile: &Profile, paths: &mut ContextPaths) -> Result<Kind> {
    if !profile.runtime.temporary_directory {
        return Ok(Kind::Disabled);
    }
    let configured = profile.runtime.temp_dir.as_str();
    let project = paths.cwd.join(".lilbox");
    let has_project = configured == "auto"
        && match fs::symlink_metadata(&project) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    bail!(
                        ".lilbox must be a real directory, not a symlink: {}",
                        project.display()
                    );
                }
                crate::trust::directory(&project, false)?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error).context("cannot inspect .lilbox directory"),
        };
    let kind = match configured {
        "auto" if has_project => {
            paths.temporary = project.join("tmp");
            Kind::Directory
        }
        "auto" | "private" => {
            paths.temporary = if cfg!(target_os = "linux") {
                PathBuf::from("/tmp/lilbox")
            } else {
                paths.home.join(".cache/lilbox/private/run-PENDING")
            };
            Kind::Private
        }
        "system" => {
            paths.temporary = std::env::temp_dir();
            Kind::System
        }
        "" => bail!("runtime.temp_dir must be auto, private, system, or a directory path"),
        path => {
            paths.temporary = expand_path(path, paths)?;
            Kind::Directory
        }
    };
    crate::policy::path_aliases(&paths.temporary)?;
    Ok(kind)
}

/// Check existing storage and ancestry without creating anything, for all CLI paths.
pub fn validate(kind: Kind, paths: &ContextPaths) -> Result<()> {
    let private_parent = paths
        .temporary
        .parent()
        .context("temporary path has no parent")?;
    if kind == Kind::Private && cfg!(target_os = "linux") {
        match fs::symlink_metadata(&paths.temporary) {
            Ok(metadata) if !metadata.is_dir() => bail!(
                "private tmpfs mount path must not be a symlink or file: {}",
                paths.temporary.display()
            ),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect private tmpfs mount path"),
        }
    }
    let checked = match kind {
        Kind::Directory => Some(paths.temporary.as_path()),
        Kind::Private if cfg!(target_os = "macos") => Some(private_parent),
        Kind::System => {
            if !paths.temporary.is_dir() {
                bail!("system temporary directory does not exist");
            }
            None
        }
        _ => None,
    };
    if let Some(path) = checked {
        crate::trust::directory(path, false)?;
        match fs::symlink_metadata(path) {
            Ok(_) => crate::launch::validate_temporary(path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect temporary directory"),
        }
    }
    Ok(())
}

/// Prepare only launch-time storage. Return an owner to remove fresh storage on
/// setup failure; after successful exec the directory remains for explicit cleanup.
#[cfg(unix)]
pub fn prepare(kind: Kind, paths: &mut ContextPaths) -> Result<Option<tempfile::TempDir>> {
    match kind {
        Kind::Private if cfg!(target_os = "linux") => Ok(None),
        Kind::Private => {
            let parent = paths
                .temporary
                .parent()
                .context("temporary directory has no parent")?;
            crate::trust::directory(parent, false)?;
            crate::launch::prepare_temporary(parent)?;
            let directory = tempfile::Builder::new()
                .prefix("run-")
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir_in(parent)?;
            paths.temporary = directory.path().into();
            crate::launch::validate_temporary(&paths.temporary)?;
            Ok(Some(directory))
        }
        Kind::Directory => {
            crate::launch::prepare_temporary(&paths.temporary)?;
            Ok(None)
        }
        Kind::System => {
            if !paths.temporary.is_dir() {
                bail!("system temporary directory does not exist");
            }
            Ok(None)
        }
        Kind::Disabled => Ok(None),
    }
}

/// Reject scratch setup without Unix permissions and a supported native backend.
#[cfg(not(unix))]
pub fn prepare(_kind: Kind, _paths: &mut ContextPaths) -> Result<Option<tempfile::TempDir>> {
    bail!("private temporary storage is unsupported on this platform")
}
