//! Portable CLI and read-only diagnostics tests using disposable profile directories.
//! Host enforcement is exercised separately by the explicitly ignored OS suites.

use std::{fs, process::Command};

/// Invoke Cargo's test binary, avoiding any installed lilbox found through PATH.
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lilbox"))
}

/// Help/version must work without HOME, while exec still requires a target.
#[test]
fn help_version_and_missing_command() {
    let version = format!("lilbox {}", env!("CARGO_PKG_VERSION"));
    for args in [vec![], vec!["-h"], vec!["--help"]] {
        let output = cli().env_clear().args(args).output().unwrap();
        assert!(output.status.success());
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.contains("Usage:"));
        assert!(help.contains("Tell your coding agent"));
        assert!(help.contains(&version), "{help}");
        assert!(output.stderr.is_empty());
    }
    for flag in ["-v", "-V", "--version"] {
        let output = cli().env_clear().arg(flag).output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), version);
        assert!(output.stderr.is_empty());
    }
    assert!(!cli().arg("exec").output().unwrap().status.success());
}

/// Reject invalid configuration before launch without modifying its contents.
#[test]
fn rejects_bad_profiles_without_modifying_them() {
    let root = tempfile::tempdir().unwrap();
    let configs = root.path().join("config");
    let run = || {
        let mut c = cli();
        c.arg("--config-dir").arg(&configs);
        c
    };
    fs::create_dir(&configs).unwrap();
    fs::write(configs.join("default.toml"), "do not overwrite").unwrap();
    assert_eq!(
        fs::read_to_string(configs.join("default.toml")).unwrap(),
        "do not overwrite"
    );
    for args in [
        vec!["policy"],
        vec!["-p", "../escape", "policy"],
        vec!["-p", "missing", "policy"],
    ] {
        assert!(!run().args(args).output().unwrap().status.success());
    }
}

/// Unsupported platforms must fail before the requested executable produces output.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[test]
fn unsupported_platform_never_launches_command() {
    let root = tempfile::tempdir().unwrap();
    let output = cli()
        .arg("--config-dir")
        .arg(root.path())
        .args(["exec", "echo", "MUST_NOT_RUN"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("refusing to run unsandboxed"));
}

/// Configure isolated launch paths without creating the home or config directory.
fn check_fixture(root: &std::path::Path) -> Command {
    let mut c = cli();
    c.env_clear()
        .env("HOME", root.join("home"))
        .env("NO_COLOR", "1")
        .current_dir(root)
        .arg("--config-dir")
        .arg(root.join("config"));
    c
}

/// Independent errors remain visible without exposing values or creating launch directories.
#[test]
fn check_aggregates_errors_without_leaking_assignments_or_writing_files() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("bad.toml"), "version=1\nextends='missing'").unwrap();
    fs::write(config.join("cycle.toml"), "version=1\nextends='cycle'").unwrap();
    fs::write(config.join("invalid.toml"), "version=1\n[filesystem]\nallow_read=['../escape', '${UNKNOWN}/path']\n[environment]\nkeep=['[bad]']\nset={NODE_OPTIONS='DO_NOT_PRINT_SECRET', 'BAD=NAME'='DO_NOT_PRINT_SECRET'}").unwrap();
    let output = check_fixture(root.path()).arg("check").output().unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "[bad]",
        "[cycle]",
        "[default]",
        "[invalid]",
        "inheritance cycle",
        "filesystem.allow_read[0]",
        "filesystem.allow_read[1]",
        "environment.keep",
        "NODE_OPTIONS",
        "BAD=NAME",
        "6 profile(s)",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("DO_NOT_PRINT_SECRET"));
    assert!(!text.contains('\x1b'));
    assert!(!root.path().join("home").exists());
    assert_eq!(fs::read_dir(config).unwrap().count(), 3);
}

/// Selecting one chain excludes unrelated failures; only strict mode fails on warnings.
#[test]
fn check_selected_chain_reports_conflicts_and_strict_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::create_dir_all(root.path().join("home/.ssh")).unwrap();
    fs::write(config.join("unrelated.toml"), "broken").unwrap();
    fs::write(config.join("work.toml"), "version=1\nextends='default'\nmode='closed'\n[filesystem]\nallow_read=['~/.ssh']\n[environment]\nkeep=['OPENAI_*','NODE_OPTIONS']\ndrop=['OPENAI_API_KEY','HOME']\nset={OPENAI_API_KEY='DO_NOT_PRINT_SECRET',HOME='/fake'}\n[runtime]\nbasic=true").unwrap();
    let output = check_fixture(root.path())
        .args(["-p", "work", "check"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "work -> default -> isolated",
        "blocked by deny",
        "drop wins",
        "cannot restore",
        "overwritten or removed",
        "cannot remove mandatory",
        "environment.inherit=false",
        "0 error(s)",
        "1 profile(s)",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("DO_NOT_PRINT_SECRET"));
    assert!(!text.contains("unrelated"));
    assert!(
        !check_fixture(root.path())
            .args(["-p", "work", "check", "--strict"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

/// Check the embedded fallback without creating files, then inspect installed examples.
#[test]
fn check_missing_directory_and_clean_examples() {
    let root = tempfile::tempdir().unwrap();
    let output = check_fixture(root.path()).arg("check").output().unwrap();
    assert!(output.status.success());
    assert!(!root.path().join("config").exists());
    assert!(String::from_utf8_lossy(&output.stdout).contains("embedded fallback"));
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    for (name, contents) in [
        ("default", lilbox::config::DEFAULT_PROFILE),
        ("isolated", lilbox::config::ISOLATED_PROFILE),
        ("permissive", lilbox::config::PERMISSIVE_PROFILE),
    ] {
        fs::write(config.join(format!("{name}.toml")), contents).unwrap();
    }
    let output = check_fixture(root.path()).arg("check").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("3 profile(s), 0 error(s)"), "{text}");
    #[cfg(target_os = "macos")]
    assert!(text.contains("1 warning(s)"), "{text}");
}

/// Detect aliases of denied paths and unsafe scratch permissions without changing either.
#[cfg(unix)]
#[test]
fn check_rejects_unsafe_scratch_and_finds_canonical_conflicts() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    let scratch = root.path().join("home/.cache/lilbox/tmp");
    fs::create_dir_all(&scratch).unwrap();
    fs::set_permissions(&scratch, fs::Permissions::from_mode(0o755)).unwrap();
    let private = root.path().join("home/Private");
    fs::create_dir(&private).unwrap();
    symlink(&private, root.path().join("alias")).unwrap();
    fs::write(
        config.join("work.toml"),
        "version=1\nextends='default'\n[filesystem]\nallow_write=['alias']\n[runtime]\ntemp_dir='~/.cache/lilbox/tmp'",
    )
    .unwrap();
    let output = check_fixture(root.path())
        .args(["-p", "work", "check"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("blocked by deny"), "{text}");
    assert!(text.contains("runtime.temporary_directory"), "{text}");
    assert_eq!(
        fs::metadata(scratch).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

/// Invalid inheritance prevents side effects, and diagnostic names cannot forge output.
#[test]
fn inherited_errors_prevent_execution_and_report_text_escapes_controls() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("loop.toml"), "version=1\nextends='loop'").unwrap();
    let output = check_fixture(root.path())
        .args([
            "-p",
            "loop",
            "exec",
            "/bin/sh",
            "-c",
            "echo escaped > MUST_NOT_EXIST",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("MUST_NOT_EXIST").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("inheritance cycle"));
    let output = check_fixture(root.path())
        .args(["-p", "bad\x1b[31m\nforged", "check"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains('\x1b'));
    assert!(!text.contains("\nforged"));
    assert!(text.contains("\\u{1b}"), "{text}");
}

/// All three entry points must reject capabilities the Linux backend cannot enforce.
#[cfg(target_os = "linux")]
#[test]
fn unsupported_linux_policies_fail_in_check_preview_and_launch() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    for source in [
        "version=1\nmode='closed'",
        "version=1\n[filesystem]\nallow_read=['/proc']",
        "version=1\n[filesystem]\ndeny=['/']",
    ] {
        fs::write(config.join("bad.toml"), source).unwrap();
        for args in [
            vec!["check"],
            vec!["policy"],
            vec!["exec", "/bin/sh", "-c", "echo escaped > MUST_NOT_EXIST"],
        ] {
            let output = check_fixture(root.path())
                .args(["-p", "bad"])
                .args(args)
                .output()
                .unwrap();
            assert!(!output.status.success());
        }
        assert!(!root.path().join("MUST_NOT_EXIST").exists());
    }
}

/// The new command replaces the flag and colors/indents only when requested.
#[test]
fn policy_command_and_profile_check_alias() {
    let root = tempfile::tempdir().unwrap();
    let plain = check_fixture(root.path())
        .args(["policy"])
        .output()
        .unwrap();
    assert!(plain.status.success(), "{plain:?}");
    let text = String::from_utf8(plain.stdout).unwrap();
    assert!(text.lines().any(|line| line.starts_with("  ")), "{text}");
    assert!(!text.contains('\x1b'));
    let colored = check_fixture(root.path())
        .args(["--color", "always", "policy"])
        .output()
        .unwrap();
    assert!(colored.status.success());
    assert!(colored.stdout.contains(&0x1b));
    assert!(
        !check_fixture(root.path())
            .arg("--print-policy")
            .output()
            .unwrap()
            .status
            .success()
    );
    let a = check_fixture(root.path())
        .args(["check", "--strict"])
        .output()
        .unwrap();
    let b = check_fixture(root.path())
        .args(["profile", "check", "--strict"])
        .output()
        .unwrap();
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.status.code(), b.status.code());
    let colored_check = check_fixture(root.path())
        .args(["--color", "always", "profile", "check"])
        .output()
        .unwrap();
    assert!(colored_check.stdout.contains(&0x1b));
    assert!(!root.path().join("home").exists());
}

/// Summaries expose origins and abbreviate lists without exposing assignment values.
#[test]
fn profile_show_reports_item_origins_and_can_expand_lists() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("base.toml"), "version=1\nnetwork='deny'\n[environment]\nkeep=['A','B','C','D','E']\nset={FLAG='DO_NOT_PRINT_SECRET'}").unwrap();
    fs::write(config.join("child.toml"), "version=1\nextends='base'\nnetwork='allow'\n[environment]\nkeep=['F']\nset={FLAG='CHILD_SECRET'}").unwrap();
    let run = |extra: &[&str]| {
        check_fixture(root.path())
            .args(["-p", "child", "profile", "show", "--origins"])
            .args(extra)
            .output()
            .unwrap()
    };
    let short = run(&[]);
    assert!(short.status.success(), "{short:?}");
    let text = String::from_utf8(short.stdout).unwrap();
    assert!(text.contains("network: allow  [child]"), "{text}");
    assert!(text.contains("A  [base]"));
    assert!(text.contains("FLAG  [child]"));
    assert!(text.contains("3 more items"));
    assert!(!text.contains("SECRET"));
    let full = String::from_utf8(run(&["--all"]).stdout).unwrap();
    assert!(full.contains("F  [child]"));
    assert!(!full.contains("more items"));
    assert!(!root.path().join("home").exists());
}

/// Permission queries follow deny precedence and environment normalization, without I/O.
#[test]
fn cani_explains_paths_environment_and_network_without_performing_operations() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("secret"), "untouched").unwrap();
    fs::write(config.join("work.toml"), "version=1\nmode='closed'\nnetwork='deny'\n[filesystem]\nallow_read=['${CWD}/work']\nallow_write=['${CWD}/work/secret']\ndeny=['${CWD}/work/secret']\n[runtime]\nbasic=true\n[environment]\nkeep=['EXAMPLE_*', 'NODE_OPTIONS']\ndrop=['EXAMPLE_SECRET','HOME']\nset={FLAG='DO_NOT_PRINT_SECRET'}").unwrap();
    for (operation, argument, allowed, reason) in [
        ("read-path", Some("work/future"), true, "covered by grant"),
        (
            "write-path",
            Some("work/future"),
            false,
            "no matching grant",
        ),
        ("read-path", Some("work/secret"), false, "denied by"),
        ("write-path", Some("work/secret"), false, "denied by"),
        ("execute-path", Some("work/tool"), true, "covered by grant"),
        ("read-env", Some("EXAMPLE_OK"), true, "environment.keep"),
        (
            "read-env",
            Some("EXAMPLE_SECRET"),
            false,
            "environment.drop",
        ),
        ("read-env", Some("NODE_OPTIONS"), false, "injection"),
        ("read-env", Some("HOME"), true, "normalization"),
        ("read-env", Some("FLAG"), true, "value hidden"),
        ("network", None, false, "network = Deny"),
    ] {
        let mut cmd = check_fixture(root.path());
        cmd.args(["-p", "work", "profile", "cani", operation]);
        if let Some(arg) = argument {
            cmd.arg(arg);
        }
        let result = cmd.output().unwrap();
        assert_eq!(
            result.status.success(),
            allowed,
            "{operation} {argument:?}: {result:?}"
        );
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(text.contains(reason), "{text}");
        assert!(!text.contains("DO_NOT_PRINT_SECRET"));
    }
    assert!(!work.join("future").exists());
    assert_eq!(
        fs::read_to_string(work.join("secret")).unwrap(),
        "untouched"
    );
    assert!(!root.path().join("home").exists());
}

/// Scratch is created at launch, so static queries must include its future write grant.
#[test]
fn cani_accounts_for_scratch_creation_without_creating_it() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("scratch.toml"),
        "version=1\nmode='closed'\n[runtime]\nbasic=true\ntemporary_directory=true\ntemp_dir='~/.cache/lilbox/tmp'",
    )
    .unwrap();
    let result = check_fixture(root.path())
        .args([
            "-p",
            "scratch",
            "profile",
            "cani",
            "write-path",
            "~/.cache/lilbox/tmp/future",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(!root.path().join("home").exists());
}

/// Discovery is opt-in, reports paths without contents, and does not create policy/scratch.
#[test]
fn project_secret_check_is_read_only_and_never_prints_values() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(
        root.path().join("nested/.env.production"),
        "MUST_NOT_PRINT_SECRET_VALUE",
    )
    .unwrap();
    let ordinary = check_fixture(root.path())
        .args(["-p", "isolated", "check"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&ordinary.stdout).contains(".env.production"));
    let output = check_fixture(root.path())
        .args(["-p", "isolated", "check", "--secrets", "--strict"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("nested/.env.production"));
    assert!(text.contains("policy unchanged"));
    assert!(!text.contains("MUST_NOT_PRINT_SECRET_VALUE"));
    assert!(!root.path().join("home").exists());
    assert!(!root.path().join("config").exists());
}

/// Inventory includes embedded/global/local sources and explains all selection branches.
#[test]
fn profile_inventory_selection_and_spelling_suggestions() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("config")).unwrap();
    fs::write(
        root.path().join("config/work.toml"),
        "version=1\nextends='default'",
    )
    .unwrap();
    let list = check_fixture(root.path())
        .args(["profile", "list"])
        .output()
        .unwrap();
    assert!(list.status.success(), "{list:?}");
    let text = String::from_utf8_lossy(&list.stdout);
    assert!(text.contains("work.toml") && text.contains("embedded fallback"));
    assert!(text.contains("no CWD/.lilbox/profile.toml"));
    fs::create_dir(root.path().join(".lilbox")).unwrap();
    fs::write(
        root.path().join(".lilbox/profile.toml"),
        "version=1\nextends='work'",
    )
    .unwrap();
    for command in ["list", "show"] {
        let result = check_fixture(root.path())
            .args(["profile", command])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stdout)
                .contains("takes precedence over global default")
        );
        let result = check_fixture(root.path())
            .args(["-p", "isolated", "profile", command])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(String::from_utf8_lossy(&result.stdout).contains("explicit -p isolated"));
    }
    let typo = check_fixture(root.path())
        .args(["-p", "defualt", "profile", "show"])
        .output()
        .unwrap();
    assert!(!typo.status.success());
    assert!(String::from_utf8_lossy(&typo.stderr).contains("did you mean \"default\""));
    fs::write(
        root.path().join(".lilbox/profile.toml"),
        "version=1\nextends='wrok'",
    )
    .unwrap();
    let typo = check_fixture(root.path())
        .args(["profile", "show"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&typo.stderr).contains("did you mean \"work\""));
    // Listing must still reveal candidate names when local selection is broken.
    let list = check_fixture(root.path())
        .args(["profile", "list"])
        .output()
        .unwrap();
    assert!(!list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("work.toml"));
}

/// The scaffold is valid, private and never replaces existing policy or symlinks.
#[test]
#[cfg(unix)]
fn project_creation_is_validated_and_never_clobbers() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let create = || {
        check_fixture(root.path())
            .args(["init", "--project", "--extends", "default"])
            .output()
            .unwrap()
    };
    assert!(
        !check_fixture(root.path())
            .arg("init")
            .output()
            .unwrap()
            .status
            .success()
    );
    let bad = check_fixture(root.path())
        .args(["init", "--project", "--extends", "defualt"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(!root.path().join(".lilbox").exists());
    let result = create();
    assert!(result.status.success(), "{result:?}");
    let profile = root.path().join(".lilbox/profile.toml");
    let original = fs::read(&profile).unwrap();
    assert_eq!(
        fs::metadata(&profile).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!create().status.success());
    assert_eq!(fs::read(&profile).unwrap(), original);
    assert!(
        check_fixture(root.path())
            .args(["profile", "show"])
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::remove_file(&profile).unwrap();
    symlink(root.path().join("missing"), &profile).unwrap();
    assert!(!create().status.success());
    assert!(
        fs::symlink_metadata(&profile)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(&profile).unwrap();
    fs::create_dir(&profile).unwrap();
    assert!(!create().status.success());
    fs::remove_dir_all(root.path().join(".lilbox")).unwrap();
    symlink(root.path().join("elsewhere"), root.path().join(".lilbox")).unwrap();
    assert!(!create().status.success());
    assert!(!root.path().join("elsewhere").exists());
}

/// Suggested exact-name keep repairs work; every additive inherited drop is identified.
#[test]
fn environment_answers_show_minimal_repairs_and_overriding_rules() {
    let root = tempfile::tempdir().unwrap();
    let query = |name: &str| {
        check_fixture(root.path())
            .args(["profile", "cani", "read-env", name])
            .output()
            .unwrap()
    };
    let result = query("ANTHROPIC_API_KEY");
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(!result.status.success());
    assert!(
        text.contains("*API_KEY*")
            && text.contains("ANTHROPIC_*")
            && text.contains("environment.inherit=false"),
        "{text}"
    );
    assert!(text.contains("keep = [\"ANTHROPIC_API_KEY\"]"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("capability is denied"));
    fs::create_dir(root.path().join(".lilbox")).unwrap();
    fs::write(
        root.path().join(".lilbox/profile.toml"),
        "version=1\nextends='default'\n[environment]\nkeep=['ANTHROPIC_API_KEY']",
    )
    .unwrap();
    assert!(query("ANTHROPIC_API_KEY").status.success());
    fs::create_dir(root.path().join("config")).unwrap();
    fs::write(
        root.path().join("config/default.toml"),
        "version=1\nextends='isolated'\n[environment]\ndrop=['*_API_KEY', 'ANTHROPIC_*']",
    )
    .unwrap();
    let result = query("ANTHROPIC_API_KEY");
    assert!(!result.status.success());
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(
        text.contains("overrides keep and set")
            && text.contains("drop = [] in a child does not clear")
    );
    assert!(
        text.contains("default.toml") && text.contains("*_API_KEY") && text.contains("ANTHROPIC_*")
    );
    assert!(
        !text.contains("keep = ["),
        "existing keep must not be prescribed again: {text}"
    );
    let result = query("NODE_OPTIONS");
    assert!(String::from_utf8_lossy(&result.stdout).contains("No profile override is supported"));
    let result = query("SSH_AUTH_SOCK");
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("service_ipc = true") && text.contains("keep = [\"SSH_AUTH_SOCK\"]"));
}
