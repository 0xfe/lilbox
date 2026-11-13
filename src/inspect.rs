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
    /// Repair instructions and TOML lines, never assignment values from the environment.
    pub advice: Vec<String>,
}

/// Build a definite policy answer.
fn answer(allowed: bool, reason: impl Into<String>) -> Answer {
    Answer {
        allowed: Some(allowed),
        reason: reason.into(),
        advice: Vec::new(),
    }
}

/// Explain every applicable name filter and the narrowest editable repair.
/// The sentinel contains no user value; proxy-userinfo filtering is value-dependent.
fn environment_answer(loaded: &LoadedProfile, paths: &ContextPaths, input: &str) -> Result<Answer> {
    if !environment::valid_name(input) {
        bail!("expected an environment variable name, not an assignment");
    }
    let profile = &loaded.profile;
    let settings = &profile.environment;
    let probe = environment::build(profile, paths, [(OsString::from(input), OsString::new())])?;
    let allowed = probe.contains_key(std::ffi::OsStr::new(input));
    let upper = input.to_ascii_uppercase();
    let mut result = answer(allowed, "");
    if matches!(input, "HOME" | "PWD" | "PATH" | "LILBOX") {
        result.reason =
            "mandatory launch normalization supplies this name; PATH entries are sanitized".into();
        return Ok(result);
    }
    if matches!(input, "TMPDIR" | "TMP" | "TEMP" | "OLDPWD") {
        result.reason = "launch normalization controls scratch/working-directory variables".into();
        if input == "OLDPWD" {
            result
                .advice
                .push("OLDPWD is always removed; no profile override is supported.".into());
        } else if !allowed {
            result.advice.extend([
                "Set this in the selected profile:".into(),
                "[runtime]".into(),
                "temporary_directory = true".into(),
            ]);
        }
        return Ok(result);
    }
    if let Some(pattern) = environment::INJECTION_PATTERNS
        .iter()
        .find(|p| environment::matches_pattern(&upper, p))
    {
        result.reason = format!("unconditional runtime injection filter {pattern:?}");
        result.advice.push("No profile override is supported: keep cannot restore this name and set rejects it. Use the program's explicit command-line options instead.".into());
        return Ok(result);
    }
    let keep = settings
        .keep
        .iter()
        .find(|p| environment::matches_pattern(&upper, &p.to_ascii_uppercase()));
    let assigned = settings.set.contains_key(input);
    let inherits = settings.inherit.unwrap_or(profile.mode == Mode::Open);
    let sensitive: Vec<_> = environment::SENSITIVE_PATTERNS
        .iter()
        .filter(|p| upper != "AWS_CA_BUNDLE" && environment::matches_pattern(&upper, p))
        .collect();
    let mut reasons = Vec::new();
    if !profile.runtime.service_ipc && environment::SERVICE_VARIABLES.contains(&upper.as_str()) {
        reasons.push("runtime.service_ipc=false removes this service pointer".into());
        result.advice.extend(["To pass this service pointer, also set:".into(), "[runtime]".into(), "service_ipc = true".into(), "This enables service IPC broadly; socket access still depends on network and filesystem rules.".into()]);
    }
    for pattern in &settings.drop {
        if !environment::matches_pattern(&upper, &pattern.to_ascii_uppercase()) {
            continue;
        }
        reasons.push(format!(
            "environment.drop {pattern:?} [{}] overrides keep and set",
            loaded.origin("environment.drop", Some(pattern))
        ));
        // Lists are additive, so every occurrence in the inheritance chain must change.
        for (document, source) in loaded.documents.iter().zip(&loaded.sources) {
            if document
                .fields
                .get("environment")
                .and_then(|v| v.get("drop"))
                .and_then(|v| v.as_array())
                .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(pattern)))
            {
                result.advice.push(format!("Remove or narrow environment.drop entry {pattern:?} in {}{} so it no longer matches {input}. Inherited lists are additive; drop = [] in a child does not clear them.", source.display(), if document.embedded { " (embedded: create a reviewed replacement of this parent first)" } else { "" }));
            }
        }
    }
    if assigned {
        reasons.push(format!(
            "environment.set {input} [{}]; value hidden",
            loaded.origin(&format!("environment.set.{input}"), None)
        ));
    } else if let Some(pattern) = keep {
        reasons.push(format!(
            "environment.keep {pattern:?} [{}] admits the parent variable",
            loaded.origin("environment.keep", Some(pattern))
        ));
    } else {
        if !inherits {
            reasons.push("environment.inherit=false and no matching environment.keep".into());
        }
        if !sensitive.is_empty() {
            reasons.push(format!(
                "built-in sensitive-name filter: {} (explicit keep can override)",
                sensitive
                    .iter()
                    .map(|p| format!("{p:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !inherits || !sensitive.is_empty() {
            if loaded
                .documents
                .last()
                .is_some_and(|document| document.embedded)
            {
                let parent = loaded.chain.last().unwrap();
                result.advice.push(format!("The selected profile is embedded. Create an editable project child with `lilbox init --project --extends {parent}`; if local policy already exists, review its extends before editing it. Run without -p to select local policy."));
                result.advice.push("Add this exact name to environment.keep in .lilbox/profile.toml (preserve existing keep entries):".into());
            } else {
                result.advice.push(format!(
                    "Add this exact name to environment.keep in {} (preserve existing keep entries):",
                    loaded.selection()
                ));
            }
            result
                .advice
                .extend(["[environment]".into(), format!("keep = [{input:?}]")]);
        } else {
            reasons.push("environment.inherit=true; no sensitive-name filter matches".into());
        }
    }
    if !assigned {
        result.advice.push(format!(
            "Export {input} in the parent shell before launching; keep does not create a value."
        ));
    }
    if matches!(
        upper.as_str(),
        "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" | "FTP_PROXY"
    ) && !settings.keep_proxy_credentials
    {
        result.advice.push("Name-only answer: an inherited proxy URL containing userinfo is still removed. To pass it, set environment.keep_proxy_credentials = true (applies to all inherited proxy URLs).".into());
    }
    result.reason = reasons.join("; ");
    Ok(result)
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
        return environment_answer(loaded, paths, input);
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
        return Ok(Answer { allowed: None, advice: Vec::new(), reason: "Linux private runtime mounts have operation-specific permissions; inspect `lilbox policy`".into() });
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
        return Ok(Answer { allowed: None, advice: Vec::new(), reason: "basic runtime has operation-specific device/root permissions; inspect `lilbox policy`".into() });
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
        || path_aliases(&paths.cwd.join(".lilbox/profile.toml")).is_ok_and(|p| p.contains(path))
        || loaded
            .sources
            .iter()
            .any(|p| path_aliases(p).is_ok_and(|a| a.contains(path)))
    {
        "mandatory configuration protection".into()
    } else {
        format!(
            "filesystem.protect_credentials: {}",
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
        "extends",
        loaded
            .chain
            .iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join(" -> "),
        "",
        false,
    );
    scalar(&mut output, "profile path", loaded.selection(), "", false);
    scalar(
        &mut output,
        "selection",
        &loaded.selection_reason,
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
        "mode",
        if p.mode == Mode::Open {
            "open"
        } else {
            "closed"
        },
        &loaded.origin("mode", None),
        origins,
    );
    scalar(
        &mut output,
        "network",
        format!("{:?}", policy.network).to_ascii_lowercase(),
        &loaded.origin("network", None),
        origins,
    );
    heading(&mut output, "Filesystem", color);
    scalar(
        &mut output,
        "filesystem.protect_credentials",
        p.filesystem.protect_credentials,
        &loaded.origin("filesystem.protect_credentials", None),
        origins,
    );

    for (label, field, entries) in [
        (
            "filesystem.allow_read",
            "filesystem.allow_read",
            &policy.read,
        ),
        (
            "filesystem.allow_write",
            "filesystem.allow_write",
            &policy.write,
        ),
        ("filesystem.deny", "filesystem.deny", &policy.deny),
        (
            "Ignored absent denies (Linux)",
            "filesystem.deny",
            &policy.ignored_denies,
        ),
    ] {
        if label == "Ignored absent denies (Linux)" && entries.is_empty() {
            continue;
        }
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
        "environment.keep_proxy_credentials",
        p.environment.keep_proxy_credentials,
        &loaded.origin("environment.keep_proxy_credentials", None),
        origins,
    );
    scalar(
        &mut output,
        "environment.inherit",
        p.environment.inherit.unwrap_or(p.mode == Mode::Open),
        &loaded.origin("environment.inherit", None),
        origins,
    );

    for (label, field, entries) in [
        ("environment.keep", "environment.keep", &p.environment.keep),
        ("environment.drop", "environment.drop", &p.environment.drop),
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
        "environment.set (values hidden)",
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

    scalar(
        &mut output,
        "runtime.basic",
        p.runtime.basic,
        &loaded.origin("runtime.basic", None),
        origins,
    );
    scalar(
        &mut output,
        "runtime.temporary_directory",
        p.runtime.temporary_directory,
        &loaded.origin("runtime.temporary_directory", None),
        origins,
    );
    scalar(
        &mut output,
        "runtime.service_ipc",
        p.runtime.service_ipc,
        &loaded.origin("runtime.service_ipc", None),
        origins,
    );
    scalar(
        &mut output,
        "runtime.keychain (macOS)",
        p.runtime.keychain,
        &loaded.origin("runtime.keychain", None),
        origins,
    );
    if p.runtime.temporary_directory {
        scalar(
            &mut output,
            "runtime.temp_dir",
            &p.runtime.temp_dir,
            &loaded.origin("runtime.temp_dir", None),
            origins,
        );
        scalar(
            &mut output,
            "temporary path",
            paths.temporary.display(),
            "",
            false,
        );
        if p.runtime.temp_dir == "private"
            || (p.runtime.temp_dir == "auto" && !paths.cwd.join(".lilbox").is_dir())
        {
            scalar(
                &mut output,
                "temporary storage",
                if cfg!(target_os = "linux") {
                    "per-invocation tmpfs"
                } else {
                    "per-invocation directory (run-PENDING allocated at launch)"
                },
                "",
                false,
            );
        }
    }
    output
        .push_str("\nStatic policy view; host permissions and native runtime rules still apply.\n");
    if !origins {
        output.push_str("Use --origins to see where each setting came from.\n");
    }
    output
}
