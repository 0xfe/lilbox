//! Actual host doctor probe, shared by the native ignored enforcement suites.
use std::{fs, process::Command};

/// Exercise real startup, executable discovery, diagnostics and quiet launch output.
#[test]
#[ignore = "requires host sandbox startup"]
fn doctor_checks_host_and_reports_executable_and_trust_failures() {
    let root = tempfile::tempdir().unwrap();
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lilbox"));
        command
            .env_clear()
            .env("HOME", root.path())
            .env("NO_COLOR", "1")
            .current_dir(root.path())
            .arg("--config-dir")
            .arg(root.path().join("config"));
        command
    };
    let result = command()
        .args(["doctor", "/usr/bin/true"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("PASS sandbox startup") && text.contains("PASS executable discovery"));
    let result = command()
        .args(["doctor", "lilbox-missing-test-executable"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("Remedy: Install the named program"));
    let result = command().args(["exec", "/usr/bin/true"]).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(!String::from_utf8_lossy(&result.stderr).contains("Tell your coding agent"));
    fs::create_dir(root.path().join("config")).unwrap();
    fs::write(root.path().join("config/default.toml"), "version=1").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        root.path().join("config/default.toml"),
        fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    let result = command().arg("doctor").output().unwrap();
    assert!(!result.status.success());
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("PASS sandbox startup") && text.contains("FAIL configuration trust"));
    assert!(text.contains("chmod go-w"));
}
