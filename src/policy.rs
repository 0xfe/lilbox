//! Portable permissions and path resolution shared by inspection and execution.
//! Preserve lexical and canonical aliases so symlinks cannot bypass path denies.

use crate::config::{Mode, Network, Profile};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

/// Fixed private-path inventory, expanded against the launch home and CWD.
/// This is not a recursive secret scan; unlisted copies and hard links can remain.
pub const CREDENTIAL_PATHS: &[&str] = &[
    "~/.ssh",
    "~/.aws",
    "~/.azure",
    "~/.gnupg",
    "~/.kube",
    "~/.config/gcloud",
    "~/.config/gh",
    "~/.config/hub",
    "~/.config/op",
    "~/.local/share/keyrings",
    "~/.local/share/kwalletd",
    "~/.config/1Password",
    "~/.config/google-chrome",
    "~/.config/chromium",
    "~/.mozilla",
    "~/.password-store",
    "~/.docker/config.json",
    "~/.netrc",
    "~/.npmrc",
    "~/.pypirc",
    "~/.git-credentials",
    "~/.vault-token",
    "~/.terraform.d/credentials.tfrc.json",
    "~/Private",
    "~/Library/Keychains",
    "~/Library/Group Containers/2BUA8C4S2C.com.1password",
    "~/Library/Application Support/1Password",
    "~/Library/Application Support/Google/Chrome",
    "~/Library/Application Support/Firefox",
    "~/Library/Safari",
    "~/Library/Mail",
    "~/Library/Messages",
    "~/.bash_history",
    "~/.zsh_history",
    "~/.python_history",
    "~/.local/share/fish/fish_history",
    "${CWD}/.env",
    "${CWD}/.env.local",
];

/// Absolute launch locations used for expansion, protection, and normalization.
#[derive(Debug, Clone)]
pub struct ContextPaths {
    /// Parent user's home; setting HOME does not itself grant filesystem access.
    pub home: PathBuf,
    /// Launch working directory, also the base for relative profile paths.
    pub cwd: PathBuf,
    /// Selected configuration directory, always protected from the target.
    pub config: PathBuf,
    /// Persistent per-user scratch directory, granted only when requested.
    pub temporary: PathBuf,
}

/// Backend-independent permissions with deterministic, deduplicated path sets.
#[derive(Debug)]
pub struct Policy {
    /// Baseline for operations not covered by a more specific rule.
    pub mode: Mode,
    /// Effective network setting after mode-dependent defaults.
    pub network: Network,
    /// Readable/executable subtrees, including lexical and canonical aliases.
    pub read: BTreeSet<PathBuf>,
    /// Writable subtrees, which also grant read and executable-map access.
    pub write: BTreeSet<PathBuf>,
    /// Exclusions that override grants and require ancestor rename protection.
    pub deny: BTreeSet<PathBuf>,
    /// Whether the profile explicitly enables ordinary runtime operations.
    pub basic_runtime: bool,
    /// Enables shared path denies and backend-specific credential restrictions.
    pub protect_credentials: bool,
}

impl Policy {
    /// Expand configured paths and add credential, config, and scratch rules.
    /// The loader adds individual inherited profile files to the resulting deny set.
    pub fn resolve(profile: &Profile, paths: &ContextPaths) -> Result<Self> {
        let mut policy = Self {
            mode: profile.mode,
            network: profile.network.unwrap_or(if profile.mode == Mode::Open {
                Network::Allow
            } else {
                Network::Deny
            }),
            read: BTreeSet::new(),
            write: BTreeSet::new(),
            deny: BTreeSet::new(),
            basic_runtime: profile.runtime.basic,
            protect_credentials: profile.filesystem.protect_credentials,
        };
        for (inputs, output) in [
            (&profile.filesystem.allow_read, &mut policy.read),
            (&profile.filesystem.allow_write, &mut policy.write),
            (&profile.filesystem.deny, &mut policy.deny),
        ] {
            for input in inputs {
                output.extend(resolve_path(input, paths)?);
            }
        }
        if policy.protect_credentials {
            for input in CREDENTIAL_PATHS {
                policy.deny.extend(resolve_path(input, paths)?);
            }
        }
        // Configuration cannot be altered by a sandboxed agent to weaken its next run.
        policy.deny.extend(path_aliases(&paths.config)?);
        if profile.runtime.temporary_directory {
            policy.write.extend(path_aliases(&paths.temporary)?);
        }
        Ok(policy)
    }
}

/// Require text paths because native policy renderers serialize them as text.
fn path_text(path: &Path) -> Result<&str> {
    path.to_str().context("policy paths must be valid UTF-8")
}

/// Expand the supported home/CWD prefixes and resolve both filesystem spellings.
/// Relative paths use launch CWD, including paths inherited from another profile.
pub fn resolve_path(input: &str, paths: &ContextPaths) -> Result<BTreeSet<PathBuf>> {
    if input.is_empty() {
        bail!("empty policy path");
    }
    let expanded = if input == "~" || input == "${HOME}" {
        paths.home.clone()
    } else if let Some(rest) = input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("${HOME}/"))
    {
        paths.home.join(rest)
    } else if input == "${CWD}" {
        paths.cwd.clone()
    } else if let Some(rest) = input.strip_prefix("${CWD}/") {
        paths.cwd.join(rest)
    } else {
        if input.starts_with('~') || input.contains("${") {
            bail!("unsupported path expansion: {input}");
        }
        paths.cwd.join(input)
    };
    path_aliases(&expanded)
}

/// Keep both spellings: /tmp and /private/tmp, symlink targets, and not-yet-created
/// descendants of existing symlinks. Reject .. rather than changing its meaning
/// when it follows a symlink.
pub fn path_aliases(path: &Path) -> Result<BTreeSet<PathBuf>> {
    let text = path_text(path)?;
    if !path.is_absolute()
        || text.chars().any(char::is_control)
        || path.components().any(|part| part == Component::ParentDir)
    {
        bail!(
            "policy paths must be absolute and contain no '..' or control characters: {}",
            path.display()
        );
    }
    let lexical: PathBuf = path.components().collect();
    let canonical = canonicalize_future(&lexical)?;
    path_text(&canonical)?;
    Ok(BTreeSet::from([lexical, canonical]))
}

/// Resolve existing symlink ancestors while retaining nonexistent suffixes.
/// A dangling symlink is an error, not a suffix we can safely append unchanged.
pub(crate) fn canonicalize_future(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                bail!("dangling symlink in policy path: {}", path.display());
            }
            let parent = path.parent().context("cannot resolve filesystem root")?;
            Ok(canonicalize_future(parent)?
                .join(path.file_name().context("missing path component")?))
        }
        Err(error) => {
            Err(error).with_context(|| format!("cannot resolve policy path {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed schema defaults grant no capabilities and reject ambiguous path syntax.
    #[test]
    fn closed_defaults_grant_nothing() {
        let root = tempfile::tempdir().unwrap();
        let paths = ContextPaths {
            home: root.path().into(),
            cwd: root.path().into(),
            config: root.path().join("config"),
            temporary: root.path().join("tmp"),
        };
        let p = Policy::resolve(
            &crate::config::parse("version = 1\nmode = 'closed'").unwrap(),
            &paths,
        )
        .unwrap();
        assert_eq!(p.network, Network::Deny);
        assert!(p.read.is_empty() && p.write.is_empty() && !p.basic_runtime);
        assert!(p.deny.contains(&paths.config));
        assert!(resolve_path("../escape", &paths).is_err());
        assert!(resolve_path("${OTHER}/key", &paths).is_err());
        assert!(resolve_path("~someone/key", &paths).is_err());
        assert!(resolve_path("/tmp/a\nrule", &paths).is_err());
    }

    /// Future descendants must follow existing aliases without accepting dangling links.
    #[cfg(unix)]
    #[test]
    fn resolves_existing_symlink_ancestors_and_rejects_dangling_links() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let real = fs::canonicalize(root.path()).unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        symlink(root.path().join("real"), root.path().join("link")).unwrap();
        let aliases = path_aliases(&root.path().join("link/future/key")).unwrap();
        assert!(aliases.contains(&real.join("real/future/key")));
        symlink(root.path().join("missing"), root.path().join("dangling")).unwrap();
        assert!(path_aliases(&root.path().join("dangling/key")).is_err());
    }
}
