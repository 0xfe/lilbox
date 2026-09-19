use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

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
    pub extends: Option<String>,
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

fn document(source: &str) -> Result<toml::Value> {
    // Validate each file before merging so an overridden bad field cannot hide.
    let profile: Profile = toml::from_str(source).map_err(|error: toml::de::Error| {
        let line = source[..error.span().map_or(0, |s| s.start)]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
            + 1;
        anyhow::anyhow!("invalid profile TOML at line {line}: {}", error.message())
    })?;
    if profile.version != 1 {
        bail!(
            "unsupported profile version {}; expected 1",
            profile.version
        );
    }
    if let Some(parent) = &profile.extends {
        validate_name(parent).context("invalid extends profile name")?;
    }
    Ok(toml::from_str(source)?)
}

pub fn parse(source: &str) -> Result<Profile> {
    let value = document(source)?;
    if value.get("extends").is_some() {
        bail!("extends requires a configuration directory; use config::load");
    }
    Ok(value.try_into()?)
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

/// A resolved profile and every file contributing to it, root first.
#[derive(Debug)]
pub struct LoadedProfile {
    pub profile: Profile,
    pub sources: Vec<PathBuf>,
    pub chain: Vec<String>,
}

impl LoadedProfile {
    pub fn policy(&self, paths: &crate::policy::ContextPaths) -> Result<crate::policy::Policy> {
        let mut policy = crate::policy::Policy::resolve(&self.profile, paths)?;
        for source in &self.sources {
            policy.deny.extend(crate::policy::path_aliases(source)?);
        }
        Ok(policy)
    }
}

pub fn load(directory: &Path, name: &str) -> Result<Profile> {
    Ok(load_resolved(directory, name)?.profile)
}

pub fn load_resolved(directory: &Path, name: &str) -> Result<LoadedProfile> {
    let mut sources = Vec::new();
    let mut chain = Vec::new();
    let value = resolve(directory, name, &mut Vec::new(), &mut sources, &mut chain)?;
    Ok(LoadedProfile {
        profile: value.try_into()?,
        sources,
        chain,
    })
}

fn resolve(
    directory: &Path,
    name: &str,
    visiting: &mut Vec<(String, PathBuf)>,
    sources: &mut Vec<PathBuf>,
    chain: &mut Vec<String>,
) -> Result<toml::Value> {
    validate_name(name)?;
    let path = directory.join(format!("{name}.toml"));
    let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if visiting.iter().any(|(n, p)| n == name || p == &identity) {
        let mut names: Vec<_> = visiting.iter().map(|(n, _)| n.as_str()).collect();
        names.push(name);
        bail!(
            "inheritance cycle: {} (repeated profile or file target)",
            names.join(" -> ")
        );
    }
    if visiting.len() >= 64 {
        bail!("inheritance exceeds 64 profiles at {name}");
    }
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && name == "default" => {
            // A dangling symlink is an invalid configuration, not a missing one.
            if fs::symlink_metadata(&path).is_ok() {
                bail!("cannot read profile {}: {error}", path.display());
            }
            DEFAULT_PROFILE.to_owned()
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read profile {}", path.display()));
        }
    };
    let mut child = document(&source).with_context(|| format!("profile {}", path.display()))?;
    let parent = child.as_table_mut().unwrap().remove("extends");
    visiting.push((name.to_owned(), identity));
    let merged = if let Some(parent) = parent {
        let parent = parent.as_str().unwrap(); // validated by document
        let mut base = resolve(directory, parent, visiting, sources, chain)
            .with_context(|| format!("profile {name} extends {parent}"))?;
        merge(&mut base, child);
        base
    } else {
        // Also deduplicate lists in a root document.
        let mut base = toml::Value::Table(Default::default());
        merge(&mut base, child);
        base
    };
    visiting.pop();
    sources.push(path);
    chain.push(name.to_owned());
    Ok(merged)
}

fn merge(base: &mut toml::Value, child: toml::Value) {
    match (base, child) {
        (toml::Value::Table(base), toml::Value::Table(child)) => {
            for (key, value) in child {
                if let Some(previous) = base.get_mut(&key) {
                    merge(previous, value);
                } else {
                    let mut entry = match &value {
                        toml::Value::Table(_) => toml::Value::Table(Default::default()),
                        toml::Value::Array(_) => toml::Value::Array(Vec::new()),
                        _ => value.clone(),
                    };
                    merge(&mut entry, value);
                    base.insert(key, entry);
                }
            }
        }
        (toml::Value::Array(base), toml::Value::Array(child)) => {
            for item in child {
                if !base.contains(&item) {
                    base.push(item);
                }
            }
        }
        (base, child) => *base = child,
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

#[cfg(test)]
mod inheritance_tests {
    use super::*;

    fn put(root: &Path, name: &str, text: &str) {
        fs::write(root.join(format!("{name}.toml")), text).unwrap();
    }

    #[test]
    fn merge_preserves_omissions_false_values_lists_and_map_keys() {
        let root = tempfile::tempdir().unwrap();
        put(
            root.path(),
            "base",
            "version=1\nmode='closed'\nnetwork='deny'\n[filesystem]\nallow_read=['/usr', '/usr']\ndeny=['~/private']\n[environment]\ninherit=false\nkeep=['TERM']\ndrop=['SECRET*']\nset={EDITOR='vi', FLAG='base'}\n[runtime]\nbasic=true\ntemporary_directory=true",
        );
        put(
            root.path(),
            "middle",
            "version=1\nextends='base'\n[filesystem]\nallow_read=['/usr','/bin']\n[environment]\nkeep=['LANG']\nset={FLAG='child'}",
        );
        put(
            root.path(),
            "child",
            "version=1\nextends='middle'\nnetwork='allow'\n[filesystem]\nprotect_credentials=false\ndeny=[]\n[environment]\nkeep=[]\n[runtime]\ntemporary_directory=false",
        );
        let loaded = load_resolved(root.path(), "child").unwrap();
        assert_eq!(loaded.chain, ["base", "middle", "child"]);
        assert_eq!(loaded.sources.len(), 3);
        let p = loaded.profile;
        assert_eq!(p.mode, Mode::Closed);
        assert_eq!(p.network, Some(Network::Allow));
        assert_eq!(p.environment.inherit, Some(false));
        assert_eq!(p.filesystem.allow_read, ["/usr", "/bin"]);
        assert_eq!(p.filesystem.deny, ["~/private"]);
        assert_eq!(p.environment.keep, ["TERM", "LANG"]);
        assert_eq!(p.environment.drop, ["SECRET*"]);
        assert_eq!(p.environment.set["FLAG"], "child");
        assert_eq!(p.environment.set["EDITOR"], "vi");
        assert!(!p.filesystem.protect_credentials);
        assert!(p.runtime.basic && !p.runtime.temporary_directory);
        assert!(p.extends.is_none());
    }

    #[test]
    fn defaults_apply_after_merge_and_default_fallback_is_inherited() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), "base", "version=1");
        put(
            root.path(),
            "child",
            "version=1\nextends='base'\nmode='closed'",
        );
        let p = load(root.path(), "child").unwrap();
        assert_eq!(p.mode, Mode::Closed);
        assert_eq!(p.network, None);
        assert_eq!(p.environment.inherit, None);
        put(
            root.path(),
            "child",
            "version=1\nextends='default'\nmode='closed'",
        );
        let p = load(root.path(), "child").unwrap();
        assert_eq!(p.network, Some(Network::Allow));
        assert_eq!(p.environment.inherit, Some(true));
        assert!(p.runtime.temporary_directory);
        put(root.path(), "default", "version=1\nnetwork='deny'");
        assert_eq!(
            load(root.path(), "child").unwrap().network,
            Some(Network::Deny)
        );
    }

    #[test]
    fn dependency_errors_identify_the_chain_and_reject_ambiguous_parents() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), "a", "version=1\nextends='b'");
        let error = format!("{:#}", load(root.path(), "a").unwrap_err());
        assert!(error.contains("a extends b") && error.contains("b.toml"));
        put(root.path(), "b", "version=1\nextends='a'");
        assert!(format!("{:#}", load(root.path(), "a").unwrap_err()).contains("a -> b -> a"));
        put(root.path(), "b", "version=1\nextends='b'");
        assert!(format!("{:#}", load(root.path(), "a").unwrap_err()).contains("a -> b -> b"));
        for invalid in [
            "version=2",
            "version=1\nnetwork='proxy'",
            "version=1\nextends='../outside'",
            "version=1\nextends=['a','b']",
            "version=1\n[filesystem]\nallow_raed=[]",
        ] {
            put(root.path(), "b", invalid);
            assert!(load(root.path(), "a").is_err(), "{invalid}");
        }
        assert!(parse("version=1\nextends='default'").is_err());
    }

    #[test]
    fn bounded_depth() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..64 {
            let text = if i == 63 {
                "version=1".into()
            } else {
                format!("version=1\nextends='p{}'", i + 1)
            };
            put(root.path(), &format!("p{i}"), &text);
        }
        assert_eq!(load_resolved(root.path(), "p0").unwrap().chain.len(), 64);
        put(root.path(), "extra", "version=1\nextends='p0'");
        assert!(format!("{:#}", load(root.path(), "extra").unwrap_err()).contains("exceeds 64"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cycles_dangling_parents_and_dependency_protection() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir(&config).unwrap();
        put(&config, "a", "version=1\nextends='alias'");
        symlink(config.join("a.toml"), config.join("alias.toml")).unwrap();
        assert!(format!("{:#}", load(&config, "a").unwrap_err()).contains("cycle"));
        symlink(root.path().join("missing"), config.join("default.toml")).unwrap();
        assert!(load(&config, "default").is_err());
        put(root.path(), "outside", "version=1");
        symlink(root.path().join("outside.toml"), config.join("parent.toml")).unwrap();
        put(&config, "child", "version=1\nextends='parent'");
        let loaded = load_resolved(&config, "child").unwrap();
        let paths = crate::policy::ContextPaths {
            home: root.path().into(),
            cwd: root.path().into(),
            temporary: root.path().join("tmp"),
            config,
        };
        let policy = loaded.policy(&paths).unwrap();
        assert!(
            policy
                .deny
                .contains(&fs::canonicalize(root.path().join("outside.toml")).unwrap())
        );
    }
}
