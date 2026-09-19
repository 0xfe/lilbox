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

pub type ChildEnvironment = BTreeMap<OsString, OsString>;

fn injection_variable(name: &str) -> bool {
    name.starts_with("DYLD_")
        || name.starts_with("LD_")
        || name.starts_with("BASH_FUNC_")
        || matches!(
            name,
            "ENV"
                | "BASH_ENV"
                | "ZDOTDIR"
                | "SHELLOPTS"
                | "BASHOPTS"
                | "CDPATH"
                | "GLOBIGNORE"
                | "IFS"
                | "NODE_OPTIONS"
                | "NODE_PATH"
                | "PYTHONPATH"
                | "PYTHONHOME"
                | "PYTHONSTARTUP"
                | "RUBYOPT"
                | "RUBYLIB"
                | "PERL5OPT"
                | "PERL5LIB"
                | "JAVA_TOOL_OPTIONS"
                | "JDK_JAVA_OPTIONS"
                | "_JAVA_OPTIONS"
                | "GIT_CONFIG_PARAMETERS"
                | "GIT_CONFIG_COUNT"
                | "GIT_ASKPASS"
                | "SSH_ASKPASS"
        )
        || name.starts_with("GIT_CONFIG_KEY_")
        || name.starts_with("GIT_CONFIG_VALUE_")
}

fn sensitive(name: &str) -> bool {
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
                | "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "FTP_PROXY"
                | "NO_PROXY"
        )
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or(name == pattern, |prefix| name.starts_with(prefix))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}

pub fn build(
    profile: &Profile,
    paths: &ContextPaths,
    source: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<ChildEnvironment> {
    let settings = &profile.environment;
    for pattern in settings.keep.iter().chain(&settings.drop) {
        let base = pattern.strip_suffix('*').unwrap_or(pattern);
        if !valid_name(base) {
            bail!("invalid environment pattern {pattern:?}; use NAME or PREFIX*");
        }
    }
    for (name, value) in &settings.set {
        if !valid_name(name) || value.contains('\0') {
            bail!("invalid environment assignment for {name:?}");
        }
        if injection_variable(&name.to_ascii_uppercase()) {
            bail!("runtime injection variable {name} cannot be set");
        }
    }
    let inherits = settings.inherit.unwrap_or(profile.mode == Mode::Open);
    let mut result = BTreeMap::new();
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
    // Avoid trusting an inherited shell path in closed mode; shell selection
    // happens separately, before environment scrubbing.
    result.remove(OsStr::new("OLDPWD"));
    Ok(result)
}

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
    fn paths() -> ContextPaths {
        ContextPaths {
            home: "/home/user".into(),
            cwd: "/work".into(),
            config: "/config".into(),
            temporary: "/scratch".into(),
        }
    }
    fn source() -> Vec<(OsString, OsString)> {
        [
            ("OPENAI_API_KEY", "secret"),
            ("SSH_AUTH_SOCK", "/socket"),
            ("github_token", "secret"),
            ("DATABASE_URL", "secret"),
            ("HTTPS_PROXY", "secret"),
            ("NODE_OPTIONS", "--require evil.js"),
            ("BASH_ENV", "evil.sh"),
            ("TERM", "xterm"),
            ("PATH", ":.:/work:relative:/bin:/usr/bin"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }
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
            "HTTPS_PROXY",
            "NODE_OPTIONS",
            "BASH_ENV",
        ] {
            assert!(!result.contains_key(OsStr::new(key)), "{key}");
        }
    }
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
}
