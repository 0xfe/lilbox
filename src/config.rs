use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, io::Write, path::Path};

pub const DEFAULT_PROFILE: &str = include_str!("../profiles/default.toml");
pub const ISOLATED_PROFILE: &str = include_str!("../profiles/isolated.toml");

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Open,
    Closed,
}

/// Keep network policy separate from filesystem permissions so later backends
/// can implement proxy or endpoint restrictions without changing path rules.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Allow,
    Deny,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    #[serde(default)]
    pub mode: Mode,
    pub network: Option<Network>,
    #[serde(default)]
    pub filesystem: Filesystem,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub runtime: Runtime,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Filesystem {
    pub protect_credentials: bool,
    pub allow_read: Vec<String>,
    pub allow_write: Vec<String>,
    pub deny: Vec<String>,
}

impl Default for Filesystem {
    fn default() -> Self {
        Self {
            protect_credentials: true,
            allow_read: vec![],
            allow_write: vec![],
            deny: vec![],
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Environment {
    pub inherit: Option<bool>,
    pub keep: Vec<String>,
    pub drop: Vec<String>,
    pub set: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Runtime {
    /// An explicit capability bundle, required for ordinary closed-mode tools.
    pub basic: bool,
    /// Give children a private, persistent per-user temporary directory.
    pub temporary_directory: bool,
}

pub fn parse(source: &str) -> Result<Profile> {
    let profile: Profile = toml::from_str(source).context("invalid profile TOML")?;
    if profile.version != 1 {
        bail!(
            "unsupported profile version {}; expected 1",
            profile.version
        );
    }
    Ok(profile)
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        bail!("profile names must contain 1–64 ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

pub fn load(directory: &Path, name: &str) -> Result<Profile> {
    validate_name(name)?;
    let path = directory.join(format!("{name}.toml"));
    match fs::read_to_string(&path) {
        Ok(source) => parse(&source).with_context(|| format!("profile {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && name == "default" => {
            // A dangling symlink is an invalid configuration, not a missing one.
            if fs::symlink_metadata(&path).is_ok() {
                bail!("cannot read profile {}: {error}", path.display());
            }
            parse(DEFAULT_PROFILE)
        }
        Err(error) => Err(error).with_context(|| format!("cannot read profile {}", path.display())),
    }
}

/// Atomic, no-clobber creation also refuses existing symlinks.
pub fn init(directory: &Path) -> Result<Vec<String>> {
    fs::create_dir_all(directory).context("cannot create profile directory")?;
    let mut created = Vec::new();
    for (name, contents) in [("default", DEFAULT_PROFILE), ("isolated", ISOLATED_PROFILE)] {
        let path = directory.join(format!("{name}.toml"));
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(contents.as_bytes())?;
        match file.persist_noclobber(&path) {
            Ok(_) => created.push(path.display().to_string()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error).context("cannot write profile"),
        }
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_schema_and_versions() {
        for text in [
            "mode = 'open'",
            "version = 2",
            "version = 1\nnetwrok = 'allow'",
            "version = 1\nnetwork = 'proxy'",
            "version = 1\n[filesystem]\nallow = ['/']",
        ] {
            assert!(parse(text).is_err(), "accepted {text}");
        }
        parse(DEFAULT_PROFILE).unwrap();
        parse(ISOLATED_PROFILE).unwrap();
    }

    #[test]
    fn selection_and_no_clobber() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(load(root.path(), "default").unwrap().mode, Mode::Open);
        assert!(load(root.path(), "missing").is_err());
        for name in ["../secret", "/etc/passwd", ".", "a.b", ""] {
            assert!(load(root.path(), name).is_err());
        }
        assert_eq!(init(root.path()).unwrap().len(), 2);
        fs::write(root.path().join("default.toml"), "custom").unwrap();
        assert!(init(root.path()).unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(root.path().join("default.toml")).unwrap(),
            "custom"
        );
        assert!(load(root.path(), "default").is_err());
    }
}
