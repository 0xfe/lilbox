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

/// Preserve user edits during init and reject invalid configuration before launch.
#[test]
fn initializes_without_overwriting_and_rejects_bad_profiles() {
    let root = tempfile::tempdir().unwrap();
    let configs = root.path().join("config");
    let run = || {
        let mut c = cli();
        c.arg("--config-dir").arg(&configs);
        c
    };
    assert!(run().arg("init").output().unwrap().status.success());
    fs::write(configs.join("default.toml"), "do not overwrite").unwrap();
    assert!(run().arg("init").output().unwrap().status.success());
    assert_eq!(
        fs::read_to_string(configs.join("default.toml")).unwrap(),
        "do not overwrite"
    );
    for args in [
        vec!["--print-policy"],
        vec!["-p", "../escape", "--print-policy"],
        vec!["-p", "missing", "--print-policy"],
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
    fs::write(config.join("invalid.toml"), "version=1\n[filesystem]\nallow_read=['../escape', '${UNKNOWN}/path']\n[environment]\nkeep=['*bad*']\nset={NODE_OPTIONS='DO_NOT_PRINT_SECRET', 'BAD=NAME'='DO_NOT_PRINT_SECRET'}").unwrap();
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
        "4 profile(s)",
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
    fs::write(config.join("unrelated.toml"), "broken").unwrap();
    fs::write(config.join("work.toml"), "version=1\nextends='default'\nmode='closed'\n[filesystem]\nallow_read=['~/.ssh']\n[environment]\nkeep=['OPENAI_*','NODE_OPTIONS']\ndrop=['OPENAI_API_KEY','HOME']\nset={OPENAI_API_KEY='DO_NOT_PRINT_SECRET',HOME='/fake'}\n[runtime]\nbasic=true").unwrap();
    let output = check_fixture(root.path())
        .args(["-p", "work", "check"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "default → work",
        "blocked by deny",
        "drop wins",
        "cannot restore",
        "overwritten or removed",
        "cannot remove mandatory",
        "environment.inherit=true",
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("embedded default fallback"));
    let config = root.path().join("config");
    lilbox::config::init(&config).unwrap();
    let output = check_fixture(root.path()).arg("check").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("2 profile(s), 0 error(s)"), "{text}");
    #[cfg(target_os = "macos")]
    assert!(text.contains("0 warning(s)"), "{text}");
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
        "version=1\nextends='default'\n[filesystem]\nallow_write=['alias']",
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
            vec!["--print-policy"],
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
