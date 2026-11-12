//! Read-only configuration analysis. Diagnostics describe effective precedence;
//! warnings never change the policy used by the launcher.
use crate::{
    backend, config,
    config::{Mode, Network},
    environment,
    policy::{ContextPaths, Policy, resolve_path},
};
use anyhow::{Context, Result};
use std::{collections::BTreeSet, fs, path::Path};

/// Diagnostic level; warnings affect the exit status only in strict mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Explain effective settings or dependencies without flagging a problem.
    Info,
    /// Highlight surprising but valid precedence or host restrictions.
    Warning,
    /// Identify invalid configuration or an unavailable launch prerequisite.
    Error,
}

/// One diagnostic associated with a profile or directory, escaped when printed.
#[derive(Debug)]
pub struct Finding {
    /// Controls display and whether this finding fails a check.
    pub severity: Severity,
    /// Profile name, or a label for directory/platform-wide diagnostics.
    pub profile: String,
    /// Explanation without environment assignment values; escaped when printed.
    pub message: String,
}

/// Aggregated results so one invalid profile does not hide independent failures.
#[derive(Default, Debug)]
pub struct Report {
    /// Findings in deterministic discovery and analysis order.
    pub findings: Vec<Finding>,
    /// Number of selected/discovered profiles attempted, including invalid ones.
    pub profiles: usize,
}

impl Report {
    /// Add filename-only discovery without changing any effective policy.
    pub fn discover_secrets(&mut self, cwd: &Path) {
        self.add(Severity::Info, "project secrets", format!("filename-only scan; no contents read; skips {:?}, .lilbox/tmp, and symlink directories; candidates may be false positives", crate::discovery::SKIPPED_DIRECTORIES));
        let inventory = crate::discovery::scan(cwd);
        for path in inventory.candidates {
            self.add(
                Severity::Warning,
                "project secrets",
                format!(
                    "candidate: {}; review filesystem.deny (policy unchanged)",
                    path.display()
                ),
            );
        }
        for error in inventory.errors {
            self.add(Severity::Error, "project secrets", error);
        }
    }

    /// Append a finding while attaching the profile that produced it.
    fn add(&mut self, severity: Severity, profile: &str, message: impl Into<String>) {
        self.findings.push(Finding {
            severity,
            profile: profile.into(),
            message: message.into(),
        });
    }

    /// Count one severity for the summary and exit-status decision.
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    /// Fail for errors, and additionally for warnings when strict was requested.
    pub fn failed(&self, strict: bool) -> bool {
        self.count(Severity::Error) > 0 || (strict && self.count(Severity::Warning) > 0)
    }

    /// Print escaped diagnostics using the CLI's terminal-aware color preference.
    pub fn print(&self, color: bool) {
        println!("Configuration check (static analysis; no sandbox launched)");
        for finding in &self.findings {
            let (glyph, label, code) = match finding.severity {
                Severity::Info => ("✓", "info", "36"),
                Severity::Warning => ("⚠", "warning", "33"),
                Severity::Error => ("✗", "error", "31"),
            };
            let prefix = format!("{glyph} {label}");
            let prefix = if color {
                format!("\x1b[{code}m{prefix}\x1b[0m")
            } else {
                prefix
            };
            println!(
                "{prefix} [{}] {}",
                safe_text(&finding.profile),
                safe_text(&finding.message)
            );
        }
        println!(
            "{} profile(s), {} error(s), {} warning(s)",
            self.profiles,
            self.count(Severity::Error),
            self.count(Severity::Warning)
        );
    }
}

/// Escape controls in untrusted profile filenames and TOML diagnostic text.
fn safe_text(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Check one inheritance chain or all discovered profiles without mutating files.
/// Rendering checks backend support, not actual kernel enforcement or connectivity.
pub fn analyze(paths: &ContextPaths, selected: Option<&str>) -> Result<Report> {
    analyze_with_options(paths, selected, false)
}

/// Include a project-local profile in discovery and carry explicit shared-policy consent.
pub fn analyze_with_options(
    paths: &ContextPaths,
    selected: Option<&str>,
    allow_shared: bool,
) -> Result<Report> {
    let mut report = Report::default();
    let mut names = BTreeSet::new();
    if let Some(name) = selected {
        names.insert(name.to_owned());
    } else {
        // Include the embedded fallback even when no configuration directory exists.
        names.extend([
            "default".to_owned(),
            "permissive".to_owned(),
            "isolated".to_owned(),
        ]);
        if fs::symlink_metadata(paths.cwd.join(".lilbox/profile.toml")).is_ok() {
            names.insert(".lilbox/profile.toml".into());
        }
        match fs::read_dir(&paths.config) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.context("cannot enumerate profile directory")?;
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "toml") {
                        match path.file_stem().and_then(|s| s.to_str()) {
                            Some(name) => {
                                names.insert(name.to_owned());
                            }
                            None => report.add(
                                Severity::Error,
                                "directory",
                                "profile filename is not valid UTF-8",
                            ),
                        }
                    }
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && fs::symlink_metadata(&paths.config).is_err() => {}
            Err(error) => return Err(error).context("cannot enumerate profile directory"),
        }
    }
    for name in names {
        report.profiles += 1;
        let loaded = match config::load_selected(
            &paths.config,
            Some(&paths.cwd),
            if name == ".lilbox/profile.toml" {
                None
            } else {
                Some(&name)
            },
            allow_shared,
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                report.add(Severity::Error, &name, format!("{error:#}"));
                continue;
            }
        };
        let mut selected_paths = paths.clone();
        let temporary_kind = match crate::temporary::select(&loaded.profile, &mut selected_paths) {
            Ok(kind) => kind,
            Err(error) => {
                report.add(Severity::Error, &name, format!("{error:#}"));
                continue;
            }
        };
        let paths = &selected_paths;
        report.add(
            Severity::Info,
            &name,
            format!(
                "extends: {}",
                loaded
                    .chain
                    .iter()
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
        );
        if allow_shared {
            report.add(
                Severity::Warning,
                &name,
                "--allow-shared-profiles: ownership and writable-mode checks disabled",
            );
        }
        for source in &loaded.sources {
            let embedded = if !source.exists() {
                " (embedded fallback)"
            } else {
                ""
            };
            report.add(
                Severity::Info,
                &name,
                format!("source: {}{embedded}", source.display()),
            );
        }
        // Collect independent path and environment errors before attempting policy
        // resolution, which returns on its first error during an actual launch.
        let profile = &loaded.profile;
        let mut paths_valid = true;
        for (field, values) in [
            ("filesystem.allow_read", &profile.filesystem.allow_read),
            ("filesystem.allow_write", &profile.filesystem.allow_write),
            ("filesystem.deny", &profile.filesystem.deny),
        ] {
            for (index, value) in values.iter().enumerate() {
                if let Err(error) = resolve_path(value, paths) {
                    paths_valid = false;
                    report.add(
                        Severity::Error,
                        &name,
                        format!("{field}[{index}]: {error:#}"),
                    );
                }
            }
        }
        check_environment(&mut report, &name, profile);
        if !paths_valid {
            continue;
        }
        let policy = match loaded.policy(paths) {
            Ok(policy) => policy,
            Err(error) => {
                report.add(
                    Severity::Error,
                    &name,
                    format!("filesystem policy: {error:#}"),
                );
                continue;
            }
        };
        report.add(Severity::Info, &name, format!(
            "effective: mode={:?}, network={:?}, environment.inherit={}, protect_credentials={}, runtime.basic={}, temporary_directory={}",
            profile.mode, policy.network, profile.environment.inherit.unwrap_or(profile.mode == Mode::Open),
            profile.filesystem.protect_credentials, profile.runtime.basic, profile.runtime.temporary_directory));
        check_filesystem(&mut report, &name, profile, paths, &policy);
        if profile.mode == Mode::Open
            && policy.network == Network::Allow
            && profile.environment.inherit.unwrap_or(true)
        {
            report.add(Severity::Warning, &name, "mode=open, network=allow, environment.inherit=true: broad host filesystem, network and environment access");
        }
        if profile.mode == Mode::Closed {
            if !profile.runtime.basic {
                report.add(
                    Severity::Warning,
                    &name,
                    "runtime.basic=false in closed mode: ordinary programs cannot execute",
                );
            }
            if profile.environment.inherit == Some(true) {
                report.add(Severity::Warning, &name, "closed mode with environment.inherit=true: non-sensitive host variables are inherited; changing mode does not reset inherited settings");
            }
        }
        // Validate scratch ancestry/permissions without allocating storage.
        if let Err(error) = crate::temporary::validate(temporary_kind, paths) {
            report.add(
                Severity::Error,
                &name,
                format!("runtime.temporary_directory: {error:#}"),
            );
        }
        // Use the same renderer as launch, but never execute its output. A present
        // launcher does not prove that this host permits namespaces or Seatbelt.
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            match backend::current().and_then(|b| b.render(&policy)) {
                Ok(_) => {}
                Err(error) => report.add(
                    Severity::Error,
                    &name,
                    format!("sandbox rendering: {error:#}"),
                ),
            }
            let launcher = if cfg!(target_os = "macos") {
                "/usr/bin/sandbox-exec"
            } else {
                "/usr/bin/bwrap"
            };
            if !Path::new(launcher).is_file() {
                report.add(
                    Severity::Error,
                    &name,
                    format!("sandbox launcher {launcher} is missing"),
                );
            }
            if !policy.ignored_denies.is_empty() {
                report.add(
                    Severity::Warning,
                    &name,
                    crate::display::ignored_denies(&policy),
                );
            }
        }
    }
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        report.add(
            Severity::Warning,
            "platform",
            "configuration analysis only: this OS has no execution backend",
        );
    }
    Ok(report)
}

/// Explain deny/grant overlaps and inaccessible launch paths without rewriting rules.
fn check_filesystem(
    report: &mut Report,
    name: &str,
    profile: &config::Profile,
    paths: &ContextPaths,
    policy: &Policy,
) {
    for (field, grants) in [
        ("filesystem.allow_read", &profile.filesystem.allow_read),
        ("filesystem.allow_write", &profile.filesystem.allow_write),
    ] {
        for grant in grants {
            let aliases = match resolve_path(grant, paths) {
                Ok(aliases) => aliases,
                Err(error) => {
                    report.add(
                        Severity::Error,
                        name,
                        format!("{field}: path changed during check: {error:#}"),
                    );
                    continue;
                }
            };
            if let Some(deny) = policy
                .deny
                .iter()
                .find(|deny| aliases.iter().any(|p| p.starts_with(deny)))
            {
                report.add(Severity::Warning, name, format!("{field} {grant:?} is blocked by deny {}; deny wins, remove the grant or revise its source restriction", deny.display()));
            } else {
                let count = policy
                    .deny
                    .iter()
                    .filter(|deny| aliases.iter().any(|p| deny.starts_with(p)))
                    .count();
                if count > 0 {
                    report.add(Severity::Info, name, format!("{field} {grant:?} has {count} denied path(s)/aliases below it; these exclusions and ancestor rename restrictions remain effective"));
                }
            }
            if aliases
                .iter()
                .any(|p| p == Path::new("/") || p == Path::new("/System"))
                && profile.mode == Mode::Closed
            {
                report.add(Severity::Warning, name, format!("{field} {grant:?} exposes broad filesystem trees, including the macOS data volume"));
            }
        }
    }
    for (label, path, required) in [
        ("current directory", &paths.cwd, true),
        (
            "temporary directory",
            &paths.temporary,
            profile.runtime.temporary_directory,
        ),
    ] {
        if required {
            if let Ok(aliases) = crate::policy::path_aliases(path) {
                if let Some(deny) = policy
                    .deny
                    .iter()
                    .find(|d| aliases.iter().any(|p| p.starts_with(d)))
                {
                    report.add(
                        Severity::Warning,
                        name,
                        format!(
                            "{label} is blocked by deny {}; commands may fail",
                            deny.display()
                        ),
                    );
                }
            }
        }
    }
}

/// Use the same glob language as environment filtering for precedence diagnostics.
fn patterns_overlap(a: &str, b: &str) -> bool {
    environment::patterns_overlap(a, b)
}

/// Report invalid rules and precedence surprises using names, never assigned values.
fn check_environment(report: &mut Report, name: &str, profile: &config::Profile) {
    let settings = &profile.environment;
    for error in environment::validation_errors(settings) {
        report.add(Severity::Error, name, error);
    }
    for keep in &settings.keep {
        if !profile.runtime.service_ipc
            && environment::SERVICE_VARIABLES
                .iter()
                .any(|name| environment::matches_pattern(name, &keep.to_ascii_uppercase()))
        {
            report.add(Severity::Warning, name, format!("environment.keep {keep:?}: runtime.service_ipc=false removes matching service pointers"));
        }
        for drop in &settings.drop {
            if patterns_overlap(keep, drop) {
                report.add(Severity::Warning, name, format!("environment.keep {keep:?} overlaps drop {drop:?}; drop wins for matching variables"));
            }
        }
        if environment::INJECTION_PATTERNS
            .iter()
            .any(|pattern| patterns_overlap(keep, pattern))
        {
            report.add(
                Severity::Warning,
                name,
                format!("environment.keep {keep:?} cannot restore runtime injection variables"),
            );
        }
    }
    let mut seen = BTreeSet::new();
    for (key, value) in &settings.set {
        let upper = key.to_ascii_uppercase();
        if !seen.insert(upper.clone()) {
            report.add(Severity::Warning, name, format!("environment.set {key:?}: case variants are distinct assignments but keep/drop matching is case-insensitive"));
        }
        for drop in &settings.drop {
            if environment::matches_pattern(&upper, &drop.to_ascii_uppercase()) {
                report.add(
                    Severity::Warning,
                    name,
                    format!("environment.set {key:?} is removed by drop {drop:?}"),
                );
            }
        }
        if matches!(
            key.as_str(),
            "HOME" | "PWD" | "LILBOX" | "TMPDIR" | "TMP" | "TEMP" | "OLDPWD"
        ) {
            report.add(
                Severity::Warning,
                name,
                format!(
                    "environment.set {key:?} is overwritten or removed by launch normalization"
                ),
            );
        }
        if key == "PATH"
            && std::env::split_paths(value).any(|p| {
                !p.is_absolute() || p.components().any(|c| c == std::path::Component::ParentDir)
            })
        {
            report.add(
                Severity::Warning,
                name,
                "environment.set PATH contains entries that launch normalization removes",
            );
        }
    }
    for drop in &settings.drop {
        for key in ["HOME", "PWD", "PATH", "LILBOX", "TMPDIR", "TMP", "TEMP"] {
            if environment::matches_pattern(key, &drop.to_ascii_uppercase())
                && (profile.runtime.temporary_directory
                    || !matches!(key, "TMPDIR" | "TMP" | "TEMP"))
            {
                report.add(
                    Severity::Warning,
                    name,
                    format!(
                        "environment.drop {drop:?} cannot remove mandatory launch variable {key}"
                    ),
                );
            }
        }
    }
}
