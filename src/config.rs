//! Strict profile parsing, inheritance/provenance, and atomic profile initialization.
//! Merge raw TOML before applying defaults so omitted child fields stay omitted.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Platform-specific online coding profile, extending the closed isolated baseline.
#[cfg(target_os = "macos")]
pub const DEFAULT_PROFILE: &str = include_str!("../profiles/macos/default.toml");
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_PROFILE: &str = include_str!("../profiles/linux/default.toml");
/// Open baseline, also available without running init.
pub const PERMISSIVE_PROFILE: &str = include_str!("../profiles/permissive.toml");
/// Instructions installed beside profiles for other coding agents.
pub const AGENT_GUIDE: &str = include_str!("../docs/LILBOX-AGENTS.md");
/// Closed starter and embedded inheritance fallback for the current platform.
#[cfg(target_os = "macos")]
pub const ISOLATED_PROFILE: &str = include_str!("../profiles/macos/isolated.toml");
#[cfg(not(target_os = "macos"))]
pub const ISOLATED_PROFILE: &str = include_str!("../profiles/linux/isolated.toml");

/// Baseline permissions before explicit grants, denies, and runtime capabilities.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Permit operations unless restricted by the policy or backend hardening.
    #[default]
    Open,
    /// Require explicit grants, including the basic runtime needed by programs.
    Closed,
}

/// Keep network policy separate from filesystem permissions so later backends
/// can implement proxy or endpoint restrictions without changing path rules.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// Permit network access, including reachable Unix sockets.
    Allow,
    /// Block network access using the backend's socket and namespace restrictions.
    Deny,
}

/// Typed schema for a single document or a fully merged inheritance chain.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Required schema version, validated independently in every source file.
    pub version: u32,
    /// Parent profile name; removed during resolution before the final parse.
    pub extends: Option<String>,
    /// Default permission baseline when no more specific rule applies.
    #[serde(default)]
    pub mode: Mode,
    /// Omission follows the final mode, unless an ancestor explicitly set it.
    pub network: Option<Network>,
    /// Path grants and exclusions, expanded later against the launch context.
    #[serde(default)]
    pub filesystem: Filesystem,
    /// Rules for reconstructing the environment passed to the target.
    #[serde(default)]
    pub environment: Environment,
    /// Optional execution and scratch-storage capabilities.
    #[serde(default)]
    pub runtime: Runtime,
}

/// Unexpanded path rules; explicit denies take precedence over all grants.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Filesystem {
    /// Add the shared credential/private-path deny list; enabled by default.
    pub protect_credentials: bool,
    /// Subtrees that may be read and mapped as executable code.
    pub allow_read: Vec<String>,
    /// Subtrees that may also be modified; these include read access.
    pub allow_write: Vec<String>,
    /// Subtrees excluded even when covered by a broader grant.
    pub deny: Vec<String>,
}

impl Default for Filesystem {
    /// Protect credentials without adding any explicit filesystem grants.
    fn default() -> Self {
        Self {
            protect_credentials: true,
            allow_read: vec![],
            allow_write: vec![],
            deny: vec![],
        }
    }
}

/// Environment selection rules, applied before mandatory launch normalization.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Environment {
    /// Omission inherits ordinary variables only when the final mode is open.
    pub inherit: Option<bool>,
    /// Preserve inherited proxy URLs with userinfo only when explicitly enabled.
    pub keep_proxy_credentials: bool,
    /// Whole-name globs with * and ? to retain from the parent.
    pub keep: Vec<String>,
    /// Patterns removed after inheritance and assignments, before normalization.
    pub drop: Vec<String>,
    /// Explicit assignments; runtime injection variables are rejected.
    pub set: BTreeMap<String, String>,
}

/// Opt-in capabilities beyond the filesystem and network baseline.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Runtime {
    /// An explicit capability bundle, required for ordinary closed-mode tools.
    pub basic: bool,
    /// Enable TMPDIR/TMP/TEMP and the corresponding filesystem grant.
    pub temporary_directory: bool,
    /// "auto", "private", "system", or an explicit expandable directory path.
    pub temp_dir: String,
    /// Permit desktop/service Unix IPC independently of TCP/UDP networking.
    pub service_ipc: bool,
    /// Permit macOS security/keychain services; disabling can break native TLS.
    pub keychain: bool,
}

impl Default for Runtime {
    /// Preserve custom-profile IPC behavior; scratch is disabled unless requested.
    fn default() -> Self {
        Self {
            basic: false,
            temporary_directory: false,
            temp_dir: "auto".into(),
            service_ipc: true,
            keychain: true,
        }
    }
}

/// Validate one source, then retain only its explicit TOML fields for merging.
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

/// Parse a standalone profile; inheritance requires a directory-aware loader.
pub fn parse(source: &str) -> Result<Profile> {
    let value = document(source)?;
    if value.get("extends").is_some() {
        bail!("extends requires a configuration directory; use config::load");
    }
    Ok(value.try_into()?)
}

/// Restrict names to simple file stems so lookup cannot traverse directories.
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
    /// Effective fields after inheritance and schema defaults.
    pub profile: Profile,
    /// Base-to-child paths, including the expected path of an embedded fallback.
    pub sources: Vec<PathBuf>,
    /// Base-to-child names for explaining the effective configuration.
    pub chain: Vec<String>,
    /// Validated source documents, retained for exact field/item provenance.
    pub documents: Vec<SourceDocument>,
    /// Opened source identities retained until launch completes or exec replaces us.
    pub identities: Vec<crate::trust::Source>,
    /// Permission relaxation applies only to explicitly opted-in shared profiles.
    pub allow_shared: bool,
    /// Configuration root, including when every selected source is embedded.
    pub directory: PathBuf,
    /// Absence used for automatic selection must remain true until exec.
    pub absent_local: Option<PathBuf>,
}

/// One validated input before defaults or inheritance are applied.
#[derive(Debug)]
pub struct SourceDocument {
    /// Profile name used in origin labels.
    pub name: String,
    /// Whether the document came from the embedded fallback.
    pub embedded: bool,
    /// Explicit TOML fields only; assignment values must never be printed.
    pub fields: toml::Value,
}

impl LoadedProfile {
    /// Resolve permissions and deny every source file, including symlink targets.
    pub fn policy(&self, paths: &crate::policy::ContextPaths) -> Result<crate::policy::Policy> {
        let mut policy = crate::policy::Policy::resolve(&self.profile, paths)?;
        self.revalidate()?;
        for identity in &self.identities {
            policy
                .deny
                .extend(crate::policy::path_aliases(&identity.target)?);
        }
        for source in &self.sources {
            policy.deny.extend(crate::policy::path_aliases(source)?);
        }
        #[cfg(target_os = "linux")]
        policy.ignore_missing_denies()?;
        Ok(policy)
    }

    /// Detect replaced, edited, or newly-created source files before launch.
    pub fn revalidate(&self) -> Result<()> {
        crate::trust::directory(&self.directory, self.allow_shared)?;
        if let Some(path) = &self.absent_local {
            if fs::symlink_metadata(path).is_ok() {
                bail!("local profile appeared after selection: {}", path.display());
            }
        }
        for identity in &self.identities {
            identity.revalidate()?;
        }
        for (path, document) in self.sources.iter().zip(&self.documents) {
            if document.embedded && fs::symlink_metadata(path).is_ok() {
                bail!(
                    "profile appeared after embedded fallback was loaded: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }

    /// Show the selected on-disk source or the platform-specific embedded fallback.
    pub fn selection(&self) -> String {
        let path = self.sources.last().unwrap();
        let embedded = self.documents.last().unwrap().embedded;
        format!(
            "{}{}",
            path.display(),
            if embedded { " (embedded fallback)" } else { "" }
        )
    }

    /// Identify the last scalar override or the first introduction of a list item.
    pub fn origin(&self, field: &str, item: Option<&str>) -> String {
        let mut found = None;
        for document in &self.documents {
            let value = field
                .split('.')
                .try_fold(&document.fields, |v, key| v.get(key));
            let matches = match (value, item) {
                (Some(toml::Value::Array(items)), Some(item)) => {
                    items.iter().any(|v| v.as_str() == Some(item))
                }
                (Some(_), None) => true,
                _ => false,
            };
            if matches {
                found = Some(format!(
                    "{}{}",
                    document.name,
                    if document.embedded { " (embedded)" } else { "" }
                ));
                if item.is_some() {
                    break;
                }
            }
        }
        found.unwrap_or_else(|| {
            if matches!(field, "network" | "environment.inherit") {
                format!("mode-dependent default ({})", self.origin("mode", None))
            } else {
                "schema default".into()
            }
        })
    }
}

/// Load effective settings when the caller does not need dependency metadata.
pub fn load(directory: &Path, name: &str) -> Result<Profile> {
    Ok(load_resolved(directory, name)?.profile)
}

/// Resolve a named profile with secure ownership and mode checks.
pub fn load_resolved(directory: &Path, name: &str) -> Result<LoadedProfile> {
    load_selected(directory, None, Some(name), false)
}

/// Select an explicit global name, otherwise CWD/.lilbox/profile.toml when present.
/// A local profile's parents are always named profiles in the global config root.
pub fn load_selected(
    directory: &Path,
    cwd: Option<&Path>,
    name: Option<&str>,
    allow_shared: bool,
) -> Result<LoadedProfile> {
    crate::trust::directory(directory, allow_shared)?;
    let local = cwd.map(|p| p.join(".lilbox/profile.toml"));
    let selected = if name.is_none() {
        match local.clone() {
            Some(path) => match fs::symlink_metadata(&path) {
                Ok(_) => Some(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error).context("cannot inspect local profile"),
            },
            None => None,
        }
    } else {
        None
    };
    let absent_local = if name.is_none() && selected.is_none() {
        local
    } else {
        None
    };
    let mut resolver = Resolver {
        directory,
        allow_shared,
        visiting: Vec::new(),
        sources: Vec::new(),
        chain: Vec::new(),
        documents: Vec::new(),
        identities: Vec::new(),
    };
    let value = if let Some(path) = selected {
        resolver.resolve("local", path, false)?
    } else {
        let name = name.unwrap_or("default");
        validate_name(name)?;
        resolver.resolve(name, directory.join(format!("{name}.toml")), true)?
    };
    Ok(LoadedProfile {
        profile: value.try_into()?,
        sources: resolver.sources,
        chain: resolver.chain,
        documents: resolver.documents,
        identities: resolver.identities,
        allow_shared,
        directory: directory.into(),
        absent_local,
    })
}

/// Mutable state for bounded inheritance resolution, with descriptor-backed sources.
struct Resolver<'a> {
    directory: &'a Path,
    allow_shared: bool,
    visiting: Vec<PathBuf>,
    sources: Vec<PathBuf>,
    chain: Vec<String>,
    documents: Vec<SourceDocument>,
    identities: Vec<crate::trust::Source>,
}

impl Resolver<'_> {
    /// Read once and merge parents first. Only genuinely absent shipped names fall back.
    fn resolve(&mut self, name: &str, path: PathBuf, fallback: bool) -> Result<toml::Value> {
        if self.visiting.len() >= 64 {
            bail!("inheritance exceeds 64 profiles at {name}");
        }
        let (source, embedded, identity) =
            match crate::trust::Source::read(&path, self.allow_shared) {
                Ok((identity, source)) => {
                    let target = identity.target.clone();
                    self.identities.push(identity);
                    (source, false, target)
                }
                Err(error)
                    if fallback
                        && error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                        && fs::symlink_metadata(&path).is_err()
                        && matches!(name, "default" | "permissive" | "isolated") =>
                {
                    let source = match name {
                        "default" => DEFAULT_PROFILE,
                        "permissive" => PERMISSIVE_PROFILE,
                        _ => ISOLATED_PROFILE,
                    };
                    (source.to_owned(), true, path.clone())
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("cannot read profile {}", path.display()));
                }
            };
        if self.visiting.contains(&identity) {
            let mut names: Vec<_> = self
                .visiting
                .iter()
                .map(|p| {
                    p.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.push(name.into());
            bail!(
                "inheritance cycle: {} (repeated profile or file target)",
                names.join(" -> ")
            );
        }
        let mut child = document(&source).with_context(|| format!("profile {}", path.display()))?;
        let fields = child.clone();
        let parent = child.as_table_mut().unwrap().remove("extends");
        self.visiting.push(identity);
        let mut base = if let Some(parent) = parent {
            let parent = parent.as_str().unwrap();
            self.resolve(parent, self.directory.join(format!("{parent}.toml")), true)
                .with_context(|| format!("profile {name} extends {parent}"))?
        } else {
            toml::Value::Table(Default::default())
        };
        merge(&mut base, child);
        self.visiting.pop();
        self.documents.push(SourceDocument {
            name: name.into(),
            embedded,
            fields,
        });
        self.sources.push(path);
        self.chain.push(name.into());
        Ok(base)
    }
}

/// Merge tables by key, append unique list entries, and replace scalar values.
/// Empty child lists preserve inherited entries; explicit false replaces true.
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

/// How init handles existing shipped files; ordinary init always preserves them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitMode {
    /// Leave every existing entry, including symlinks, unchanged.
    Preserve,
    /// Write version-suffixed alternatives without replacing existing alternatives.
    Versioned,
    /// Atomically replace regular files; refuse symlinks and non-file entries.
    Overwrite,
}

/// Install the three starter profiles and agent guide without replacing user files.
pub fn init(directory: &Path) -> Result<Vec<String>> {
    init_with_mode(directory, InitMode::Preserve)
}

/// Write embedded defaults atomically under the caller's explicit overwrite choice.
/// Versioned alternatives are deliberately not active profiles until reviewed/renamed.
pub fn init_with_mode(directory: &Path, mode: InitMode) -> Result<Vec<String>> {
    init_files(directory, mode, &[])
}

/// Initialize only selected shipped files, supporting independent installer decisions.
pub fn init_files(directory: &Path, mode: InitMode, selected: &[String]) -> Result<Vec<String>> {
    for name in selected {
        if !matches!(
            name.as_str(),
            "default.toml" | "permissive.toml" | "isolated.toml" | "AGENTS.md"
        ) {
            bail!("not a shipped profile/guide filename: {name}");
        }
    }
    fs::create_dir_all(directory).context("cannot create profile directory")?;
    let mut created = Vec::new();
    for (name, contents) in [
        ("permissive.toml", PERMISSIVE_PROFILE),
        ("default.toml", DEFAULT_PROFILE),
        ("isolated.toml", ISOLATED_PROFILE),
        ("AGENTS.md", AGENT_GUIDE),
    ] {
        if !selected.is_empty() && !selected.iter().any(|item| item == name) {
            continue;
        }
        let mut path = directory.join(name);
        let exists = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if mode == InitMode::Overwrite && !metadata.file_type().is_file() {
                    bail!("refusing to overwrite non-regular file {}", path.display());
                }
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error).context("cannot inspect existing profile"),
        };
        if exists {
            match mode {
                InitMode::Preserve => continue,
                InitMode::Versioned => {
                    path = directory.join(format!("{name}.{}", env!("CARGO_PKG_VERSION")))
                }
                InitMode::Overwrite => {}
            }
        }
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(contents.as_bytes())?;
        let result = if mode == InitMode::Overwrite {
            file.persist(&path)
        } else {
            file.persist_noclobber(&path)
        };
        match result {
            Ok(_) => created.push(path.display().to_string()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error).context("cannot write profile or agent guide"),
        }
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reject misspellings and unsupported schemas while accepting both shipped profiles.
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
        parse(PERMISSIVE_PROFILE).unwrap();
        parse(ISOLATED_PROFILE).unwrap();
    }

    /// Built-in fallbacks require absence, and ordinary init preserves existing user files.
    #[test]
    fn selection_and_no_clobber() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(load(root.path(), "default").unwrap().mode, Mode::Closed);
        assert!(load(root.path(), "missing").is_err());
        for name in ["../secret", "/etc/passwd", ".", "a.b", ""] {
            assert!(load(root.path(), name).is_err());
        }
        assert_eq!(init(root.path()).unwrap().len(), 4);
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

    /// Write one named profile inside the disposable configuration directory.
    fn put(root: &Path, name: &str, text: &str) {
        fs::write(root.join(format!("{name}.toml")), text).unwrap();
    }

    /// Distinguish omission from explicit false and retain additive lists across three levels.
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
        assert_eq!(loaded.origin("filesystem.allow_read", Some("/usr")), "base");
        assert_eq!(
            loaded.origin("filesystem.allow_read", Some("/bin")),
            "middle"
        );
        assert_eq!(loaded.origin("runtime.temporary_directory", None), "child");
        assert_eq!(loaded.origin("runtime.basic", None), "base");
        assert_eq!(loaded.origin("environment.set.FLAG", None), "middle");
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

    /// Mode-dependent defaults must not erase explicit settings inherited from default.
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
        assert_eq!(p.environment.inherit, Some(false));
        assert!(p.runtime.temporary_directory);
        put(root.path(), "default", "version=1\nnetwork='deny'");
        assert_eq!(
            load(root.path(), "child").unwrap().network,
            Some(Network::Deny)
        );
    }

    /// Expose dependency failures and reject invalid parents even if a child overrides them.
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

    /// Allow exactly 64 profiles, then reject the next recursive dependency.
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

    /// Resolve file identity for cycles and protect parent files outside the config tree.
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
