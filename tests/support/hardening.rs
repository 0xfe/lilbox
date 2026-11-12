//! Shared host-enforcement regressions for the closed default and scratch isolation.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

/// Own a disposable home/workspace for actual sandbox launches.
struct Workspace {
    _root: tempfile::TempDir,
    home: PathBuf,
    work: PathBuf,
}
impl Workspace {
    /// Keep home and workspace separate, as in normal coding sessions.
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(root.path()).unwrap();
        let home = base.join("home");
        let work = base.join("work");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&work).unwrap();
        Self {
            _root: root,
            home,
            work,
        }
    }
    /// Use embedded native profiles and no real credentials or inherited settings.
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lilbox"));
        command
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&self.work)
            .arg("--config-dir")
            .arg(self.home.join("config"));
        command
    }
    /// Require a successful confined shell and return its stdout.
    fn shell(&self, script: &str) -> String {
        let output = self
            .command()
            .args(["exec", "/bin/sh", "-ec", script])
            .output()
            .unwrap();
        success(&output);
        String::from_utf8(output.stdout).unwrap()
    }
}

/// Surface setup errors instead of mistaking them for enforcement.
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Each private launch starts empty; concurrent invocations cannot share scratch files.
#[test]
#[ignore = "requires the native host sandbox"]
fn private_scratch_is_fresh_and_separate_across_launches() {
    let f = Workspace::new();
    let first = f.shell("printf '%s' \"$TMPDIR\"; echo private > \"$TMPDIR/sentinel\"");
    f.shell("test ! -e \"$TMPDIR/sentinel\"; echo new > \"$TMPDIR/sentinel\"");
    let output = f
        .command()
        .args([
            "exec",
            "/bin/sh",
            "-ec",
            "test ! -r \"$1/sentinel\"",
            "sh",
            &first,
        ])
        .output()
        .unwrap();
    success(&output);
    let mut children = Vec::new();
    for value in ["one", "two"] {
        children.push(f.command().args(["exec", "/bin/sh", "-ec", "echo \"$1\" > \"$TMPDIR/collision\"; sleep 0.2; test \"$(cat \"$TMPDIR/collision\")\" = \"$1\"", "sh", value]).spawn().unwrap());
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    #[cfg(target_os = "linux")]
    f.shell("awk '$2 == \"/tmp/lilbox\" && $3 == \"tmpfs\" { found=1 } END { exit !found }' /proc/mounts");
}

/// Workspace files are writable; unrelated home data and maintained credential fixtures are denied.
#[test]
#[ignore = "requires the native host sandbox"]
fn closed_default_and_credential_inventory_are_enforced() {
    let f = Workspace::new();
    fs::write(f.home.join("outside"), "private").unwrap();
    f.shell("echo ok > result; test ! -r \"$HOME/outside\"");
    // Permissive's broad baseline tests that the new inventory supplies actual denies.
    for relative in [
        ".cargo/credentials.toml",
        ".gradle/gradle.properties",
        ".m2/settings.xml",
        ".config/containers/auth.json",
        ".config/rclone/rclone.conf",
        ".config/pypoetry/auth.toml",
        ".config/pip/pip.conf",
        ".config/sops/age/keys.txt",
    ] {
        let path = f.home.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "fake").unwrap();
        let output = f
            .command()
            .args([
                "-p",
                "permissive",
                "exec",
                "/bin/sh",
                "-ec",
                "test ! -r \"$1\"",
                "sh",
            ])
            .arg(&path)
            .output()
            .unwrap();
        success(&output);
    }
}

/// Local policy takes precedence, is named on startup, and cannot be read or changed inside.
#[test]
#[ignore = "requires the native host sandbox"]
fn local_profile_and_project_temporary_directory_are_enforced() {
    let f = Workspace::new();
    fs::create_dir(f.work.join(".lilbox")).unwrap();
    fs::write(
        f.work.join(".lilbox/profile.toml"),
        "version=1\nextends='default'\n[environment]\nkeep=['MY_*']\ndrop=['*_API_KEY']",
    )
    .unwrap();
    let output = f.command().env("MY_API_KEY", "fake").env("MY_FLAG", "ok").args(["exec", "/bin/sh", "-ec", "test -z \"${MY_API_KEY+x}\"; test \"$MY_FLAG\" = ok; test ! -r .lilbox/profile.toml; test ! -w .lilbox/profile.toml; test \"$TMPDIR\" = \"$PWD/.lilbox/tmp\"; echo ok > \"$TMPDIR/file\""]).output().unwrap();
    success(&output);
    let banner = String::from_utf8_lossy(&output.stderr);
    assert!(
        banner.contains("profile  local") && banner.contains(".lilbox/profile.toml"),
        "{banner}"
    );
    assert!(f.work.join(".lilbox/tmp/file").is_file());
}
