//! Profile inventory, conservative spelling suggestions, and project-only creation.
//! Discovery never executes policy; creation publishes a complete file without clobbering.

use crate::{config, display, policy::ContextPaths};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeSet, fs, io::Write, path::Path};

/// Enumerate global TOML names alongside the three embedded starters.
pub fn names(directory: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::from(["default".into(), "isolated".into(), "permissive".into()]);
    match fs::read_dir(directory) {
        Ok(entries) => {
            for entry in entries {
                let path = entry?.path();
                if path.extension().is_some_and(|ext| ext == "toml") {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .context("profile filename is not valid UTF-8")?;
                    if config::validate_name(name).is_ok() {
                        names.insert(name.into());
                    }
                }
            }
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && fs::symlink_metadata(directory).is_err() => {}
        Err(error) => return Err(error).context("cannot enumerate profile directory"),
    }
    Ok(names)
}

/// Compute bounded-size edit distance, including adjacent swapped letters.
fn distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut rows = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in rows[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            rows[i][j] = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]));
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                rows[i][j] = rows[i][j].min(rows[i - 2][j - 2] + 1);
            }
        }
    }
    rows[a.len()][b.len()]
}

/// Suggest at most three nearby global names; never silently select a correction.
pub fn suggestion(directory: &Path, name: &str) -> String {
    if name.len() > 256 {
        return String::new();
    }
    let mut candidates: Vec<_> = names(directory)
        .unwrap_or_default()
        .into_iter()
        .filter(|candidate| candidate.len() <= 256)
        .map(|candidate| (distance(name, &candidate), candidate))
        .filter(|(distance, _)| *distance <= if name.len() < 5 { 1 } else { 2 })
        .collect();
    candidates.sort();
    let names = candidates
        .into_iter()
        .take(3)
        .map(|(_, name)| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    if names.is_empty() {
        "; run `lilbox profile list` to see available profiles".into()
    } else {
        format!("; did you mean {names}? Run `lilbox profile list` for paths")
    }
}

/// List paths even when selection is invalid, then explain the attempted selection.
pub fn list(paths: &ContextPaths, selected: Option<&str>, allow_shared: bool) -> Result<()> {
    println!(
        "Global profiles: {}",
        display::safe(&paths.config.display().to_string())
    );
    for name in names(&paths.config)? {
        let path = paths.config.join(format!("{name}.toml"));
        let source = if fs::symlink_metadata(&path).is_ok() {
            "file"
        } else {
            "embedded fallback"
        };
        println!(
            "  {}: {} ({source})",
            display::safe(&name),
            display::safe(&path.display().to_string())
        );
    }
    let local = paths.cwd.join(".lilbox/profile.toml");
    println!(
        "Local profile: {} ({})",
        display::safe(&local.display().to_string()),
        if fs::symlink_metadata(&local).is_ok() {
            "present"
        } else {
            "absent"
        }
    );
    let loaded = config::load_selected(&paths.config, Some(&paths.cwd), selected, allow_shared)?;
    println!("Selected: {}", display::safe(&loaded.selection()));
    println!("Reason: {}", display::safe(&loaded.selection_reason));
    Ok(())
}

/// Create only CWD/.lilbox/profile.toml; existing entries, including symlinks, fail.
/// There is deliberately no force flag: review and move an existing policy first.
pub fn create_project(
    paths: &ContextPaths,
    parent: &str,
    allow_shared: bool,
) -> Result<std::path::PathBuf> {
    config::validate_name(parent)?;
    let loaded = config::load_selected(&paths.config, None, Some(parent), allow_shared)?;
    let mut selected_paths = paths.clone();
    let kind = crate::temporary::select(&loaded.profile, &mut selected_paths)?;
    crate::temporary::validate(kind, &selected_paths)?;
    let policy = loaded.policy(&selected_paths)?;
    crate::environment::build(&loaded.profile, &selected_paths, [])?;
    crate::backend::current()?.render(&policy)?;
    let directory = paths.cwd.join(".lilbox");
    // Never write through a project-directory symlink, including dangling links.
    if let Ok(metadata) = fs::symlink_metadata(&directory) {
        if !metadata.is_dir() {
            bail!("{} must be a non-symlink directory", directory.display());
        }
    }
    crate::trust::directory(&directory, false)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        if !directory.exists() {
            fs::DirBuilder::new().mode(0o700).create(&directory)?;
        }
    }
    let destination = directory.join("profile.toml");
    let mut staged = tempfile::NamedTempFile::new_in(&directory)?;
    writeln!(
        staged,
        "# Project policy; named parents resolve in the global profile directory.\nversion = 1\nextends = {parent:?}"
    )?;
    staged.as_file().sync_all()?;
    loaded.revalidate()?;
    staged.persist_noclobber(&destination).map_err(|error| error.error)
        .with_context(|| format!("cannot create {}; existing profiles are never overwritten; review and move the existing file before retrying", destination.display()))?;
    Ok(destination)
}
