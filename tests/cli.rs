use std::{fs, process::Command};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lilbox"))
}

#[test]
fn help_version_and_missing_command() {
    let output = cli().output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    let output = cli().arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("lilbox {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(!cli().arg("exec").output().unwrap().status.success());
}

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

#[cfg(not(target_os = "macos"))]
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
