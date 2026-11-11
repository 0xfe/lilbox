//! Human-readable effective profiles and read-only policy permission queries.
//! Answers describe lilbox rules, not host ACLs, connectivity, or a kernel probe.

use crate::{
    config::{LoadedProfile, Mode, Network},
    display, environment,
    policy::{ContextPaths, Policy, path_aliases, resolve_path},
};
use anyhow::{Result, bail};
use clap::ValueEnum;
use std::{ffi::OsString, path::Path};

/// Supported policy questions; path writes mean content writes or ordinary creation.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Capability {
    /// Read a file or list a directory.
    ReadPath,
    /// Write contents or create a path; rename/unlink are separate OS operations.
    WritePath,
    /// Execute a file, subject to its runtime and library dependencies.
    ExecutePath,
    /// Pass an environment variable by name; values are never displayed.
    ReadEnv,
    /// Use networking (this profile has no endpoint-specific rules).
    Network,
}

/// A static policy answer with an explanation rather than an unqualified guarantee.
#[derive(Debug)]
pub struct Answer {
    /// None means this operation depends on native runtime rules we do not approximate.
    pub allowed: Option<bool>,
    /// Matching rule or reason, without any environment assignment values.
    pub reason: String,
}

/// Build a definite policy answer.
fn answer(allowed: bool, reason: impl Into<String>) -> Answer {
    Answer {
        allowed: Some(allowed),
        reason: reason.into(),
    }
}

/// Explain one capability using the same resolved policy and environment as launch.
/// This never opens the queried file, executes a target, or connects to a service.
pub fn cani(
    loaded: &LoadedProfile,
    policy: &Policy,
    paths: &ContextPaths,
    capability: Capability,
    argument: Option<&str>,
) -> Result<Answer> {
    if matches!(capability, Capability::Network) {
        if argument.is_some() {
            bail!("network takes no argument; policy is not endpoint-specific");
        }
        return Ok(answer(
            policy.network == Network::Allow,
            format!(
                "network = {:?} [{}]",
                policy.network,
                loaded.origin("network", None)
            ),
        ));
    }
    let input = argument
        .ok_or_else(|| anyhow::anyhow!("this capability requires a path or environment name"))?;
    if matches!(capability, Capability::ReadEnv) {
        if !environment::valid_name(input) {
            bail!("expected an environment variable name, not an assignment");
        }
        // A sentinel source answers whether a name would be admitted even when
        // the current parent does not have it. Mandatory normalization still wins.
        let probe = environment::build(
            &loaded.profile,
            paths,
            [(OsString::from(input), OsString::new())],
        )?;
        let allowed = probe.contains_key(std::ffi::OsStr::new(input));
        let upper = input.to_ascii_uppercase();
        let settings = &loaded.profile.environment;
        let reason = if environment::injection_variable(&upper) {
            "runtime injection variables are always removed".into()
        } else if matches!(input, "HOME" | "PWD" | "PATH" | "LILBOX") {
            "mandatory launch normalization supplies this name".into()
        } else if matches!(input, "TMPDIR" | "TMP" | "TEMP" | "OLDPWD") {
            "launch normalization controls scratch/working-directory variables".into()
        } else if let Some(pattern) = settings
            .drop
            .iter()
            .find(|p| environment::matches_pattern(&upper, &p.to_ascii_uppercase()))
        {
            format!(
                "environment.drop {pattern:?} [{}]",
                loaded.origin("environment.drop", Some(pattern))
            )
        } else if settings.set.contains_key(input) {
            format!(
                "environment.set {input} [{}]; value hidden",
                loaded.origin(&format!("environment.set.{input}"), None)
            )
        } else if let Some(pattern) = settings
            .keep
            .iter()
            .find(|p| environment::matches_pattern(&upper, &p.to_ascii_uppercase()))
        {
            format!(
                "environment.keep {pattern:?} [{}]; passed only if present in the parent",
                loaded.origin("environment.keep", Some(pattern))
            )
        } else {
            "environment inheritance and sensitive-name filtering; passed only if present in the parent".into()
        };
        return Ok(answer(allowed, reason));
    }
    let aliases = resolve_path(input, paths)?;
    if let Some(deny) = policy
        .deny
        .iter()
        .find(|d| aliases.iter().any(|p| p.starts_with(d)))
    {
        return Ok(answer(false, format!("denied by {}", deny.display())));
    }
    // Native runtime mounts and device operations are not ordinary subtree grants.
    // Give an honest indeterminate answer rather than inventing an allow/deny.
    if cfg!(target_os = "linux")
        && aliases
            .iter()
            .any(|p| ["/proc", "/dev", "/sys"].iter().any(|r| p.starts_with(r)))
    {
        return Ok(Answer { allowed: None, reason: "Linux private runtime mounts have operation-specific permissions; inspect `lilbox policy`".into() });
    }
    if policy.mode == Mode::Open {
        return Ok(answer(
            true,
            "open filesystem baseline; explicit denies checked first",
        ));
    }
    if matches!(capability, Capability::ExecutePath) && !policy.basic_runtime {
        return Ok(answer(
            false,
            "closed mode requires runtime.basic for execution",
        ));
    }
    let grants: Vec<_> = if matches!(capability, Capability::WritePath) {
        policy.write.iter().collect()
    } else {
        policy.read.iter().chain(&policy.write).collect()
    };
    // Launch creates requested scratch storage before Linux mount planning, so
    // the implicit scratch grant also covers a not-yet-created directory.
    let scratch = path_aliases(&paths.temporary)?;
    for grant in grants {
        if aliases.iter().any(|p| p.starts_with(grant))
            && (!cfg!(target_os = "linux")
                || grant.try_exists()?
                || (loaded.profile.runtime.temporary_directory && scratch.contains(grant)))
        {
            return Ok(answer(
                true,
                format!("covered by grant {}", grant.display()),
            ));
        }
    }
    if policy.basic_runtime
        && aliases
            .iter()
            .any(|p| p == Path::new("/") || p.starts_with("/dev"))
    {
        return Ok(Answer { allowed: None, reason: "basic runtime has operation-specific device/root permissions; inspect `lilbox policy`".into() });
    }
    Ok(answer(
        false,
        "closed filesystem baseline with no matching grant",
    ))
}

/// Append a styled section heading to a profile report.
fn heading(output: &mut String, title: &str, color: bool) {
    output.push_str(&format!("\n{}\n", display::paint(title, "1;36", color)));
}

/// Add a scalar with optional provenance, escaping all profile-controlled text.
fn scalar(
    output: &mut String,
    label: &str,
    value: impl std::fmt::Display,
    origin: &str,
    origins: bool,
) {
    output.push_str(&format!(
        "  {label}: {}{}\n",
        display::safe(&value.to_string()),
        if origins {
            format!("  [{}]", display::safe(origin))
        } else {
            String::new()
        }
    ));
}

/// Show a small sample of a list; --all expands it without truncating individual entries.
fn list(output: &mut String, label: &str, items: Vec<(String, String)>, all: bool, origins: bool) {
    output.push_str(&format!("  {label} ({}):\n", items.len()));
    let limit = if all { items.len() } else { 3 };
    if items.is_empty() {
        output.push_str("    (none)\n");
    }
    for (value, origin) in items.iter().take(limit) {
        output.push_str(&format!(
            "    • {}{}\n",
            display::safe(value),
            if origins {
                format!("  [{}]", display::safe(origin))
            } else {
                String::new()
            }
        ));
    }
    if items.len() > limit {
        output.push_str(&format!(
            "    … {} more items (use --all)\n",
            items.len() - limit
        ));
    }
}

/// Find why a resolved path was granted/denied, including implicit runtime restrictions.
fn path_origin(loaded: &LoadedProfile, paths: &ContextPaths, path: &Path, field: &str) -> String {
    let inputs = match field {
        "filesystem.allow_read" => &loaded.profile.filesystem.allow_read,
        "filesystem.allow_write" => &loaded.profile.filesystem.allow_write,
        _ => &loaded.profile.filesystem.deny,
    };
    for input in inputs {
        if resolve_path(input, paths).is_ok_and(|p| p.contains(path)) {
            return loaded.origin(field, Some(input));
        }
    }
    if field == "filesystem.allow_write" {
        return format!(
            "runtime.temporary_directory: {}",
            loaded.origin("runtime.temporary_directory", None)
        );
    }
    if path_aliases(&paths.config).is_ok_and(|p| p.contains(path))
        || loaded
            .sources
            .iter()
            .any(|p| path_aliases(p).is_ok_and(|a| a.contains(path)))
    {
        "mandatory configuration protection".into()
    } else {
        format!(
            "shared credential list: {}",
            loaded.origin("filesystem.protect_credentials", None)
        )
    }
}

/// Display effective permissions and optional per-field/per-item inheritance origins.
/// Assignment names are shown, but their values may be secrets and remain hidden.
pub fn show(
    loaded: &LoadedProfile,
    policy: &Policy,
    paths: &ContextPaths,
    all: bool,
    origins: bool,
    color: bool,
) -> String {
    let p = &loaded.profile;
    let mut output = display::paint("Effective profile", "1;36", color) + "\n";
    scalar(
        &mut output,
        "Inheritance (base → child)",
        loaded.chain.join(" → "),
        "",
        false,
    );
    scalar(
        &mut output,
        "Working directory",
        paths.cwd.display(),
        "",
        false,
    );
    scalar(
        &mut output,
        "Mode",
        if p.mode == Mode::Open {
            "open — broad access with exclusions"
        } else {
            "closed — explicit grants only"
        },
        &loaded.origin("mode", None),
        origins,
    );
    scalar(
        &mut output,
        "Network",
        format!("{:?}", policy.network),
        &loaded.origin("network", None),
        origins,
    );
    heading(&mut output, "Filesystem", color);
    scalar(
        &mut output,
        "Shared credential protection",
        p.filesystem.protect_credentials,
        &loaded.origin("filesystem.protect_credentials", None),
        origins,
    );
    output.push_str("  Denies override grants. Write grants also permit reads and execution.\n");
    for (label, field, entries) in [
        ("Read grants", "filesystem.allow_read", &policy.read),
        ("Write grants", "filesystem.allow_write", &policy.write),
        ("Active denies", "filesystem.deny", &policy.deny),
        (
            "Ignored absent denies (Linux)",
            "filesystem.deny",
            &policy.ignored_denies,
        ),
    ] {
        list(
            &mut output,
            label,
            entries
                .iter()
                .map(|path| {
                    (
                        path.display().to_string(),
                        path_origin(loaded, paths, path, field),
                    )
                })
                .collect(),
            all,
            origins,
        );
    }
    heading(&mut output, "Environment", color);
    scalar(
        &mut output,
        "Inherit ordinary variables",
        p.environment.inherit.unwrap_or(p.mode == Mode::Open),
        &loaded.origin("environment.inherit", None),
        origins,
    );
    output.push_str("  Sensitive names are filtered unless explicitly kept or set.\n  Injection hooks are always blocked. Drops override keep/set.\n  HOME, PWD, PATH, LILBOX and scratch variables are normalized last.\n");
    for (label, field, entries) in [
        ("Keep", "environment.keep", &p.environment.keep),
        ("Drop", "environment.drop", &p.environment.drop),
    ] {
        list(
            &mut output,
            label,
            entries
                .iter()
                .map(|v| (v.clone(), loaded.origin(field, Some(v))))
                .collect(),
            all,
            origins,
        );
    }
    list(
        &mut output,
        "Assignments (values hidden)",
        p.environment
            .set
            .keys()
            .map(|key| {
                (
                    key.clone(),
                    loaded.origin(&format!("environment.set.{key}"), None),
                )
            })
            .collect(),
        all,
        origins,
    );
    list(
        &mut output,
        "Always-blocked injection patterns",
        environment::INJECTION_PATTERNS
            .iter()
            .map(|p| (p.to_string(), "built-in environment filter".into()))
            .collect(),
        all,
        origins,
    );
    heading(&mut output, "Runtime", color);
    if p.mode == Mode::Open {
        output.push_str("  Open baseline already permits ordinary runtime operations.\n");
    }
    scalar(
        &mut output,
        "Basic runtime",
        p.runtime.basic,
        &loaded.origin("runtime.basic", None),
        origins,
    );
    scalar(
        &mut output,
        "Private scratch",
        p.runtime.temporary_directory,
        &loaded.origin("runtime.temporary_directory", None),
        origins,
    );
    output
        .push_str("\nStatic policy view; host permissions and native runtime rules still apply.\n");
    if !origins {
        output.push_str("Use --origins to see where each setting came from.\n");
    }
    output
}
