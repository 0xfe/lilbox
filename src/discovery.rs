//! Optional project-secret inventory: filenames and metadata only, never contents.
//! Do not follow symlink directories or modify the effective sandbox policy.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Bound total work independently of directory depth.
const MAX_ENTRIES: usize = 100_000;
/// Bound nested traversal, reporting rather than hiding incomplete coverage.
const MAX_DEPTH: usize = 32;
/// Generated/vendor metadata excluded explicitly from discovery.
pub const SKIPPED_DIRECTORIES: &[&str] = &[".git", "node_modules", "target", "dist"];

/// Candidate paths and incomplete-scan errors, without any file contents.
#[derive(Default)]
pub struct Inventory {
    /// Name-based candidates are not proof of secrets; examples can be false positives.
    pub candidates: BTreeSet<PathBuf>,
    /// I/O failures and traversal limits prevent claiming a complete scan.
    pub errors: Vec<String>,
}

/// Recognize conventional secret names without opening a file.
fn candidate(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".env")
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
        || matches!(
            lower.as_str(),
            "id_rsa"
                | "id_ed25519"
                | "credentials"
                | "credentials.toml"
                | "auth.json"
                | "auth.toml"
                | ".netrc"
                | ".npmrc"
                | ".pypirc"
        )
}

/// Walk CWD only; directory symlinks are never followed and candidate contents stay unread.
pub fn scan(root: &Path) -> Inventory {
    let mut inventory = Inventory::default();
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut count = 0;
    while let Some((directory, depth)) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                inventory
                    .errors
                    .push(format!("cannot scan {}: {error}", directory.display()));
                continue;
            }
        };
        for entry in entries {
            count += 1;
            if count > MAX_ENTRIES {
                inventory.errors.push(format!(
                    "project-secret scan stopped at {MAX_ENTRIES} entries"
                ));
                return inventory;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    inventory
                        .errors
                        .push(format!("cannot enumerate {}: {error}", directory.display()));
                    continue;
                }
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    inventory.errors.push(format!(
                        "cannot inspect {}: {error}",
                        entry.path().display()
                    ));
                    continue;
                }
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() {
                if SKIPPED_DIRECTORIES.contains(&name.as_ref())
                    || entry.path() == root.join(".lilbox/tmp")
                {
                    continue;
                }
                if depth >= MAX_DEPTH {
                    inventory.errors.push(format!(
                        "project-secret scan depth limit at {}",
                        entry.path().display()
                    ));
                } else {
                    pending.push((entry.path(), depth + 1));
                }
            } else if candidate(&name) {
                inventory.candidates.insert(entry.path());
            }
        }
    }
    inventory.errors.sort();
    inventory
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nested candidates are reported; generated files and directory-link cycles are excluded.
    #[test]
    fn discovers_names_without_following_links() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::create_dir(root.path().join("node_modules")).unwrap();
        fs::write(
            root.path().join("nested/.env.production"),
            "NEVER_PRINT_VALUE",
        )
        .unwrap();
        fs::write(root.path().join("node_modules/hidden.key"), "fake").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.path(), root.path().join("cycle")).unwrap();
        let inventory = scan(root.path());
        assert!(inventory.errors.is_empty());
        assert_eq!(
            inventory.candidates,
            BTreeSet::from([root.path().join("nested/.env.production")])
        );
    }
}
