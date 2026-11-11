//! Child environment filtering and interactive-shell startup arguments.
//! Filtering selects ordinary variables first, then normalizes launch metadata.

use crate::{
    config::{Mode, Profile},
    policy::ContextPaths,
};
use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::Path,
};

/// Deterministically ordered environment preserving OS-native keys and values.
pub type ChildEnvironment = BTreeMap<OsString, OsString>;

/// Loader, interpreter, and startup hooks that keep/set must never restore.
pub(crate) const INJECTION_PATTERNS: &[&str] = &[
    "DYLD_*",
    "LD_*",
    "BASH_FUNC_*",
    "GIT_CONFIG_KEY_*",
    "GIT_CONFIG_VALUE_*",
    "ENV",
    "BASH_ENV",
    "ZDOTDIR",
    "SHELLOPTS",
    "BASHOPTS",
    "CDPATH",
    "GLOBIGNORE",
    "IFS",
    "NODE_OPTIONS",
    "NODE_PATH",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
    "RUBYOPT",
    "RUBYLIB",
    "PERL5OPT",
    "PERL5LIB",
    "JAVA_TOOL_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "_JAVA_OPTIONS",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
];

/// Match a caller-normalized uppercase name against the injection deny list.
pub(crate) fn injection_variable(name: &str) -> bool {
    INJECTION_PATTERNS
        .iter()
        .any(|pattern| matches_pattern(name, pattern))
}

/// Identify likely secrets by uppercase name; values are deliberately not scanned.
fn sensitive(name: &str) -> bool {
    // This is a public trust-bundle path, not an AWS credential. Other common
    // CA variables and proxy settings likewise pass through inherited environments.
    if name == "AWS_CA_BUNDLE" {
        return false;
    }
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "API_KEY",
        "APIKEY",
        "PRIVATE_KEY",
        "CREDENTIAL",
        "ACCESS_KEY",
        "AUTH",
    ]
    .iter()
    .any(|part| name.contains(part))
        || [
            "AWS_",
            "AZURE_",
            "GOOGLE_",
            "GCLOUD_",
            "OPENAI_",
            "ANTHROPIC_",
            "COHERE_",
            "HUGGING_FACE_",
            "HUGGINGFACE_",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || matches!(
            name,
            "SSH_AGENT_PID"
                | "SSH_CONNECTION"
                | "SSH_CLIENT"
                | "GPG_AGENT_INFO"
                | "KRB5CCNAME"
                | "DOCKER_HOST"
                | "KUBECONFIG"
                | "DATABASE_URL"
                | "REDIS_URL"
                | "HF_TOKEN"
        )
}

/// Match an exact name or trailing-star prefix; callers normalize case first.
pub(crate) fn matches_pattern(name: &str, pattern: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or(name == pattern, |prefix| name.starts_with(prefix))
}

/// Accept portable ASCII variable names, with digits permitted after the first byte.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}

/// Shared by launching and static checking; never include assignment values.
pub(crate) fn validation_errors(settings: &crate::config::Environment) -> Vec<String> {
    let mut errors = Vec::new();
    for (field, patterns) in [("keep", &settings.keep), ("drop", &settings.drop)] {
        for pattern in patterns {
            if !valid_name(pattern.strip_suffix('*').unwrap_or(pattern)) {
                errors.push(format!(
                    "environment.{field}: invalid pattern {pattern:?}; use NAME or PREFIX*"
                ));
            }
        }
    }
    for (name, value) in &settings.set {
        if !valid_name(name) || value.contains('\0') {
            errors.push(format!(
                "environment.set {name:?}: invalid name or NUL in value"
            ));
        }
        if injection_variable(&name.to_ascii_uppercase()) {
            errors.push(format!(
                "environment.set {name:?}: runtime injection variable cannot be set"
            ));
        }
    }
    errors
}

/// Build the entire child environment instead of mutating the parent's environment.
/// Drops override keep/set; mandatory context is normalized after those rules.
pub fn build(
    profile: &Profile,
    paths: &ContextPaths,
    source: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<ChildEnvironment> {
    let settings = &profile.environment;
    let errors = validation_errors(settings);
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    let inherits = settings.inherit.unwrap_or(profile.mode == Mode::Open);
    let mut result = BTreeMap::new();
    // Explicit keeps may restore credentials, but never runtime injection hooks.
    // Names must be text for matching; retained values can still be non-UTF-8.
    for (key, value) in source {
        let Some(name) = key.to_str() else { continue };
        let upper = name.to_ascii_uppercase();
        let keep = settings
            .keep
            .iter()
            .any(|p| matches_pattern(&upper, &p.to_ascii_uppercase()));
        if !injection_variable(&upper) && (keep || (inherits && !sensitive(&upper))) {
            result.insert(key, value);
        }
    }
    // Assignments replace exact keys. Apply drops afterwards so assignments do
    // not provide a second route around an explicit exclusion.
    for (name, value) in &settings.set {
        result.insert(name.into(), value.into());
    }
    result.retain(|key, _| {
        !settings.drop.iter().any(|p| {
            matches_pattern(
                &key.to_string_lossy().to_ascii_uppercase(),
                &p.to_ascii_uppercase(),
            )
        })
    });

    // Required launch context is deterministic even in closed mode. These are
    // metadata, not filesystem grants. PATH never searches CWD implicitly.
    let path = result
        .remove(OsStr::new("PATH"))
        .unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    let entries: Vec<_> = std::env::split_paths(&path)
        .filter(|p| {
            p.is_absolute()
                && p != &paths.cwd
                && !p
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
        })
        .collect();
    result.insert("PATH".into(), std::env::join_paths(entries)?);
    result.insert("HOME".into(), paths.home.as_os_str().into());
    result.insert("PWD".into(), paths.cwd.as_os_str().into());
    result.insert("LILBOX".into(), "1".into());
    // Never pass a parent TMPDIR through to a closed profile without a grant.
    for name in ["TMPDIR", "TMP", "TEMP"] {
        result.remove(OsStr::new(name));
    }
    if profile.runtime.temporary_directory {
        for name in ["TMPDIR", "TMP", "TEMP"] {
            result.insert(name.into(), paths.temporary.as_os_str().into());
        }
    }
    // Do not leave the parent's previous working directory as a shell shortcut.
    result.remove(OsStr::new("OLDPWD"));
    Ok(result)
}

/// Select known interactive-shell flags that suppress user startup files where
/// supported. Environment filtering separately removes ENV/BASH_ENV-style hooks.
pub fn shell_command(shell: &Path) -> Result<Vec<OsString>> {
    if !shell.is_absolute() {
        bail!("SHELL must be an absolute executable path");
    }
    let name = shell.file_name().and_then(OsStr::to_str).unwrap_or("");
    let flags: &[&str] = match name {
        "zsh" => &["-f", "-i"],
        "bash" => &["--noprofile", "--norc", "-i"],
        "fish" => &["--no-config", "-i"],
        "sh" | "dash" | "ksh" => &["-i"],
        _ => bail!(
            "unsupported shell {name:?}; use `lilbox exec /path/to/shell` with its no-startup-file options"
        ),
    };
    Ok(std::iter::once(shell.as_os_str().into())
        .chain(flags.iter().map(OsString::from))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Provide launch metadata without accessing a real user home or workspace.
    fn paths() -> ContextPaths {
        ContextPaths {
            home: "/home/user".into(),
            cwd: "/work".into(),
            config: "/config".into(),
            temporary: "/scratch".into(),
        }
    }
    /// Mix fake credentials, injection hooks, normal settings, and unsafe PATH entries.
    fn source() -> Vec<(OsString, OsString)> {
        [
            ("OPENAI_API_KEY", "secret"),
            ("SSH_AUTH_SOCK", "/socket"),
            ("github_token", "secret"),
            ("DATABASE_URL", "secret"),
            ("HTTPS_PROXY", "http://127.0.0.1:8080"),
            ("NODE_OPTIONS", "--require evil.js"),
            ("BASH_ENV", "evil.sh"),
            ("TERM", "xterm"),
            ("PATH", ":.:/work:relative:/bin:/usr/bin"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }
    /// The open profile must still remove secrets, startup hooks, and implicit CWD lookup.
    #[test]
    fn scrubs_credentials_injection_and_unsafe_path_entries() {
        let p = crate::config::parse(crate::config::DEFAULT_PROFILE).unwrap();
        let result = build(&p, &paths(), source()).unwrap();
        assert_eq!(result[OsStr::new("PATH")], "/bin:/usr/bin");
        assert_eq!(result[OsStr::new("TERM")], "xterm");
        for key in [
            "OPENAI_API_KEY",
            "SSH_AUTH_SOCK",
            "github_token",
            "DATABASE_URL",
            "NODE_OPTIONS",
            "BASH_ENV",
        ] {
            assert!(!result.contains_key(OsStr::new(key)), "{key}");
        }
    }
    /// Closed mode keeps only requested variables, with drops and injection rules prevailing.
    #[test]
    fn closed_environment_is_opt_in_and_drop_wins() {
        let p = crate::config::parse("version=1\nmode='closed'\n[environment]\nkeep=['OPENAI_*', 'NODE_OPTIONS']\ndrop=['OPENAI_API_KEY']\n[environment.set]\nMY_FLAG='yes'").unwrap();
        let result = build(&p, &paths(), source()).unwrap();
        assert!(!result.contains_key(OsStr::new("TERM")));
        assert!(!result.contains_key(OsStr::new("OPENAI_API_KEY")));
        assert!(!result.contains_key(OsStr::new("NODE_OPTIONS")));
        assert_eq!(result[OsStr::new("MY_FLAG")], "yes");
        assert_eq!(result[OsStr::new("PATH")], "/usr/bin:/bin:/usr/sbin:/sbin");
        assert!(!result.contains_key(OsStr::new("TMPDIR")));
    }
    /// Credential opt-in is allowed; malformed names and injection assignments are not.
    #[test]
    fn explicit_authentication_opt_in_and_invalid_settings() {
        let p = crate::config::parse("version=1\n[environment]\nkeep=['OPENAI_API_KEY']").unwrap();
        assert!(
            build(&p, &paths(), source())
                .unwrap()
                .contains_key(OsStr::new("OPENAI_API_KEY"))
        );
        for settings in [
            "keep=['*KEY*']",
            "set={NODE_OPTIONS='bad'}",
            "set={'BAD=NAME'='x'}",
        ] {
            let p = crate::config::parse(&format!("version=1\n[environment]\n{settings}")).unwrap();
            assert!(build(&p, &paths(), source()).is_err());
        }
    }

    /// Public trust/proxy settings survive open inheritance but obey drops and closed defaults.
    #[test]
    fn inherits_proxy_and_ca_settings_but_respects_explicit_drops() {
        let source: Vec<(OsString, OsString)> = [
            ("HTTP_PROXY", "http://proxy.example:8080"),
            ("HTTPS_PROXY", "http://proxy.example:8080"),
            ("ALL_PROXY", "socks5://proxy.example:1080"),
            ("NO_PROXY", "localhost,127.0.0.1"),
            ("https_proxy", "http://proxy.example:8080"),
            ("no_proxy", "localhost"),
            ("SSL_CERT_FILE", "/certs/roots.pem"),
            ("SSL_CERT_DIR", "/certs"),
            ("NODE_EXTRA_CA_CERTS", "/certs/node.pem"),
            ("REQUESTS_CA_BUNDLE", "/certs/python.pem"),
            ("CURL_CA_BUNDLE", "/certs/curl.pem"),
            ("AWS_CA_BUNDLE", "/certs/aws.pem"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        let profile = crate::config::parse(crate::config::DEFAULT_PROFILE).unwrap();
        let result = build(&profile, &paths(), source.clone()).unwrap();
        for (key, value) in &source {
            assert_eq!(result.get(key), Some(value), "{key:?}");
        }
        let profile =
            crate::config::parse("version=1\n[environment]\ndrop=['HTTPS_PROXY', 'AWS_CA_BUNDLE']")
                .unwrap();
        let result = build(&profile, &paths(), source.clone()).unwrap();
        for key in ["HTTPS_PROXY", "https_proxy", "AWS_CA_BUNDLE"] {
            assert!(!result.contains_key(OsStr::new(key)));
        }
        let profile = crate::config::parse("version=1\nmode='closed'").unwrap();
        let result = build(&profile, &paths(), source.clone()).unwrap();
        for (key, _) in source {
            assert!(!result.contains_key(&key));
        }
    }
}
