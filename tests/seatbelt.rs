//! Run explicitly with `just test-sandbox` on an unsandboxed macOS host.
//! Do not silently skip enforcement failures when this suite is requested.
#![cfg(target_os = "macos")]

use std::{
    fs,
    net::TcpListener,
    os::unix::{fs::symlink, process::CommandExt},
    path::PathBuf,
    process::{Command, Output},
};

/// Disposable home/workspace for testing the real macOS enforcement boundary.
struct Fixture {
    /// Keep fixture files alive until the test and its captured launches finish.
    _root: tempfile::TempDir,
    /// Fake HOME with credentials and ordinary ungranted files.
    home: PathBuf,
    /// Launch directory used by CWD-based grants and denies.
    work: PathBuf,
    /// Protected profile directory inside the fake home.
    config: PathBuf,
}
impl Fixture {
    /// Use canonical temporary paths and fake credentials, never the developer's files.
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(root.path()).unwrap();
        let home = base.join("home");
        let work = base.join("work");
        let config = home.join(".config/lilbox");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir(&work).unwrap();
        fs::create_dir(home.join(".ssh")).unwrap();
        fs::write(home.join(".ssh/key"), "fake credential").unwrap();
        fs::write(home.join("outside"), "outside").unwrap();
        fs::write(work.join("allowed"), "allowed").unwrap();
        fs::write(
            config.join("isolated.toml"),
            lilbox::config::ISOLATED_PROFILE,
        )
        .unwrap();
        Self {
            _root: root,
            home,
            work,
            config,
        }
    }
    /// Select the fixture home/config/CWD and a known shell with a clean environment.
    fn command(&self, profile: &str) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_lilbox"));
        c.env_clear()
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("SHELL", "/bin/zsh")
            .current_dir(&self.work)
            .arg("--config-dir")
            .arg(&self.config)
            .args(["-p", profile]);
        c
    }
    /// Capture one real sandboxed command for status and diagnostic assertions.
    fn run(&self, profile: &str, args: &[&str]) -> Output {
        self.command(profile)
            .arg("exec")
            .args(args)
            .output()
            .unwrap()
    }
    /// Write a named profile into this disposable fixture.
    fn profile(&self, name: &str, text: &str) {
        fs::write(self.config.join(format!("{name}.toml")), text).unwrap();
    }
}
/// Show both streams on failure and attribute the assertion to its test caller.
#[track_caller]
fn succeeds(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
/// Require a permission diagnostic rather than accepting an arbitrary command failure.
#[track_caller]
fn denied(output: &Output) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Operation not permitted") || stderr.contains("Permission denied"),
        "not an enforcement failure: {stderr}"
    );
}

/// Keep target stdout intact and show startup details only for actual launches.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn startup_display_reports_version_profile_and_command_on_stderr() {
    let f = Fixture::new();
    for (profile_args, profile) in [(vec![], "default"), (vec!["-p", "isolated"], "isolated")] {
        let output = Command::new(env!("CARGO_BIN_EXE_lilbox"))
            .env_clear()
            .env("HOME", &f.home)
            .env("TERM", "xterm-256color")
            .current_dir(&f.work)
            .arg("--config-dir")
            .arg(&f.config)
            .args(profile_args)
            .args(["exec", "/usr/bin/printf", "%s", "two words"])
            .output()
            .unwrap();
        succeeds(&output);
        assert_eq!(output.stdout, b"two words");
        let banner = String::from_utf8(output.stderr).unwrap();
        assert!(banner.contains(&format!("lilbox {}", env!("CARGO_PKG_VERSION"))));
        assert!(banner.contains(&format!("├ profile  {profile}")));
        assert!(banner.contains(r#"└ command  /usr/bin/printf "%s" "two words""#));
        assert!(!banner.contains('\x1b'));
    }
    let output = f
        .command("default")
        .args(["shell", "-c", "exit 0"])
        .output()
        .unwrap();
    succeeds(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(r#"└ command  /bin/zsh -f -i -c "exit 0""#)
    );
    let output = f.command("default").arg("--print-policy").output().unwrap();
    succeeds(&output);
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("(version 1)"));
}

/// Use a real PTY to check color support, NO_COLOR, and TERM=dumb.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn startup_display_colors_follow_terminal_settings() {
    let f = Fixture::new();
    for profile in ["default", "isolated"] {
        for (term, no_color, colored) in [
            ("xterm-256color", false, true),
            ("xterm-256color", true, false),
            ("dumb", false, false),
        ] {
            let mut command = Command::new("/usr/bin/script");
            command
                .env_clear()
                .env("HOME", &f.home)
                .env("TERM", term)
                .current_dir(&f.work)
                .args([
                    "-q",
                    "/dev/null",
                    env!("CARGO_BIN_EXE_lilbox"),
                    "--config-dir",
                ])
                .arg(&f.config)
                .args(["-p", profile, "exec", "/usr/bin/true"]);
            if no_color {
                command.env("NO_COLOR", "");
            }
            let output = command.output().unwrap();
            succeeds(&output);
            let banner = String::from_utf8(output.stdout).unwrap();
            assert!(banner.contains(&format!("lilbox {}", env!("CARGO_PKG_VERSION"))));
            assert_eq!(banner.contains('\x1b'), colored, "{banner:?}");
        }
    }
}

/// Subprocesses, symlinks, and ancestor renames must not bypass open-mode exclusions.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn open_mode_denies_credentials_in_children_and_through_aliases() {
    let f = Fixture::new();
    succeeds(&f.run("default", &["/bin/cat", "allowed"]));
    denied(&f.run("default", &["/bin/sh", "-c", "cat \"$HOME/.ssh/key\""]));
    denied(&f.run(
        "default",
        &["/bin/sh", "-c", "echo bad > \"$HOME/.ssh/new\""],
    ));
    symlink(f.home.join(".ssh"), f.work.join("alias")).unwrap();
    denied(&f.run("default", &["/bin/cat", "alias/key"]));
    denied(&f.run(
        "default",
        &["/bin/sh", "-c", "mv \"$HOME/.ssh\" \"$HOME/moved\""],
    ));
    denied(&f.run(
        "default",
        &["/bin/sh", "-c", "mv \"$HOME\" \"${HOME}-moved\""],
    ));
    denied(&f.run(
        "default",
        &[
            "/bin/sh",
            "-c",
            "cat \"$HOME/.config/lilbox/isolated.toml\"",
        ],
    ));
    succeeds(&f.run("default", &["/bin/sh", "-c", "echo ok > created"]));
}

/// Protect external config targets and future paths despite broad write grants.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn configured_symlinks_future_paths_and_deny_precedence() {
    let f = Fixture::new();
    fs::write(f.home.join("external.toml"), "version=1").unwrap();
    symlink(f.home.join("external.toml"), f.config.join("linked.toml")).unwrap();
    succeeds(&f.run("linked", &["/bin/cat", "allowed"]));
    denied(&f.run(
        "linked",
        &["/bin/sh", "-c", "echo bad > \"$HOME/external.toml\""],
    ));
    fs::create_dir(f.home.join("real")).unwrap();
    symlink(f.home.join("real"), f.home.join("linked")).unwrap();
    f.profile(
        "custom",
        "version=1\n[filesystem]\nallow_write=['${HOME}']\ndeny=['~/linked/future', '${CWD}/a\"b']",
    );
    fs::write(f.work.join("a\"b"), "blocked").unwrap();
    denied(&f.run("custom", &["/bin/cat", "a\"b"]));
    denied(&f.run("custom", &["/bin/sh", "-c", "mkdir \"$HOME/real/future\""]));
    denied(&f.run("custom", &["/bin/sh", "-c", "cat \"$HOME/.ssh/key\""]));
    // .config itself must not be movable to bypass the descendant restriction.
    denied(&f.run(
        "custom",
        &[
            "/bin/sh",
            "-c",
            "mv \"$HOME/.config\" \"$HOME/config-moved\"",
        ],
    ));
}

/// Verify closed grants, nested deny precedence, data-volume aliases, and no-runtime failure.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn closed_mode_grants_only_configured_paths_and_runtime() {
    let f = Fixture::new();
    succeeds(&f.run("isolated", &["/bin/cat", "allowed"]));
    succeeds(&f.run(
        "isolated",
        &[
            "/bin/sh",
            "-c",
            "echo ok > created; echo tmp > \"$TMPDIR/test\"; cat created",
        ],
    ));
    denied(&f.run("isolated", &["/bin/sh", "-c", "cat \"$HOME/outside\""]));
    denied(&f.run("isolated", &["/bin/sh", "-c", "echo bad > \"$HOME/new\""]));
    denied(&f.run("isolated", &["/bin/sh", "-c", "cat \"$HOME/.ssh/key\""]));
    fs::write(f.work.join(".env"), "fake secret").unwrap();
    denied(&f.run("isolated", &["/bin/cat", ".env"]));
    denied(&f.run("isolated", &["/bin/sh", "-c", "echo bad > .env"]));
    fs::create_dir(f.work.join("blocked")).unwrap();
    fs::write(f.work.join("blocked/key"), "secret").unwrap();
    f.profile(
        "nested",
        &lilbox::config::ISOLATED_PROFILE.replace("deny = []", "deny = ['${CWD}/blocked']"),
    );
    denied(&f.run("nested", &["/bin/cat", "blocked/key"]));
    denied(&f.run("nested", &["/bin/sh", "-c", "echo bad > blocked/new"]));
    f.profile(
        "metadata",
        &lilbox::config::ISOLATED_PROFILE
            .replace("deny = []", "deny = ['${CWD}/blocked']")
            .replace("\"/usr\"", "\"/usr\", \"${CWD}/blocked/key\""),
    );
    denied(&f.run("metadata", &["/usr/bin/stat", "blocked"]));
    let data_alias = format!("/System/Volumes/Data{}/outside", f.home.display());
    if std::path::Path::new(&data_alias).exists() {
        denied(&f.run("isolated", &["/bin/cat", &data_alias]));
    }
    f.profile("empty", "version=1\nmode='closed'");
    assert!(!f.run("empty", &["/usr/bin/true"]).status.success());
}

/// Test all mode/network combinations against a local TCP listener.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn network_allow_and_deny_are_enforced_in_both_modes() {
    let f = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    for (name, mode, network, allow) in [
        ("open_yes", "open", "allow", true),
        ("open_no", "open", "deny", false),
        ("closed_yes", "closed", "allow", true),
        ("closed_no", "closed", "deny", false),
    ] {
        let profile = lilbox::config::ISOLATED_PROFILE
            .replace("mode = \"closed\"", &format!("mode = \"{mode}\""))
            .replace("network = \"deny\"", &format!("network = \"{network}\""));
        f.profile(name, &profile);
        let output = f.run(name, &["/usr/bin/nc", "-z", "-G", "1", "127.0.0.1", &port]);
        if allow {
            succeeds(&output);
        } else {
            assert!(!output.status.success(), "{name} allowed network");
        }
    }
}

/// Network denial must block local service sockets as well as Internet protocols.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn unix_socket_connections_are_blocked_by_network_deny() {
    use std::os::unix::net::UnixListener;
    let f = Fixture::new();
    let socket = f.work.join("s");
    let _listener = UnixListener::bind(&socket).unwrap();
    f.profile("offline", "version=1\nnetwork='deny'");
    let args = [
        "/usr/bin/nc",
        "-v",
        "-U",
        "-w",
        "1",
        socket.to_str().unwrap(),
    ];
    succeeds(&f.run("default", &args));
    assert!(!f.run("offline", &args).status.success());
    assert!(!f.run("isolated", &args).status.success());
}

/// Pass agent flags and raw OS argument bytes through the wrapper unchanged.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn named_agents_keep_their_arguments_and_non_utf8_arguments_survive() {
    use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
    let f = Fixture::new();
    let bin = f.home.join("bin");
    fs::create_dir(&bin).unwrap();
    let agent = bin.join("codex");
    fs::write(&agent, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();
    let output = f
        .command("default")
        .env("PATH", &bin)
        .args(["codex", "--help", "-p", "agent-profile", "two words"])
        .output()
        .unwrap();
    succeeds(&output);
    assert_eq!(output.stdout, b"--help\n-p\nagent-profile\ntwo words\n");
    let output = f
        .command("default")
        .args(["exec", "/usr/bin/printf", "%s"])
        .arg(std::ffi::OsString::from_vec(vec![b'a', 0xff, b'b']))
        .output()
        .unwrap();
    succeeds(&output);
    assert_eq!(output.stdout, vec![b'a', 0xff, b'b']);
}

/// Exercise environment scrubbing, literal argv, shell startup suppression, and process setup.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn environment_arguments_exit_status_shell_startup_and_umask() {
    let f = Fixture::new();
    let output = f.command("default").env("OPENAI_API_KEY", "fake-secret").env("SSH_AUTH_SOCK", "/fake/agent.sock").args(["exec", "/bin/sh", "-c", "test -z \"${OPENAI_API_KEY+x}\" && test -z \"${SSH_AUTH_SOCK+x}\" && test \"$(umask)\" = 0077"]).output().unwrap();
    succeeds(&output);
    let output = f.run(
        "default",
        &["/usr/bin/printf", "%s", "a; echo unsafe $(id) two words"],
    );
    succeeds(&output);
    assert_eq!(output.stdout, b"a; echo unsafe $(id) two words");
    assert_eq!(
        f.run("default", &["/bin/sh", "-c", "exit 42"])
            .status
            .code(),
        Some(42)
    );
    fs::write(f.home.join(".zshrc"), "print LEAKED_RC").unwrap();
    let output = f
        .command("default")
        .args(["shell", "-c", "print SAFE"])
        .output()
        .unwrap();
    succeeds(&output);
    assert_eq!(output.stdout, b"SAFE\n");
}

/// A preopened credential FD must be closed, while signal termination remains observable.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn inherited_descriptors_are_closed_and_signal_status_is_preserved() {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::ExitStatusExt;
    let f = Fixture::new();
    let secret = fs::File::open(f.home.join(".ssh/key")).unwrap();
    let fd = secret.as_raw_fd();
    let mut c = f.command("default");
    c.args(["exec", "/bin/sh", "-c", "cat <&99"]);
    // Pass an already-open credential into lilbox to exercise descriptor closure,
    // independently of Seatbelt's path-based restrictions.
    unsafe {
        c.pre_exec(move || {
            if libc::dup2(fd, 99) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = c.output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Bad file descriptor"));
    let output = f.run("default", &["/bin/sh", "-c", "kill -TERM $$"]);
    assert_eq!(output.status.signal(), Some(libc::SIGTERM));
}

/// Use a PTY to ensure sandboxed shells retain interactive standard input and output.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn interactive_shell_retains_a_terminal_in_both_modes() {
    let f = Fixture::new();
    for profile in ["default", "isolated"] {
        let output = Command::new("/usr/bin/script")
            .env_clear()
            .env("HOME", &f.home)
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/zsh")
            .env("TERM", "xterm")
            .current_dir(&f.work)
            .args([
                "-q",
                "/dev/null",
                env!("CARGO_BIN_EXE_lilbox"),
                "--config-dir",
            ])
            .arg(&f.config)
            .args([
                "-p",
                profile,
                "shell",
                "-c",
                "test -t 0 && test -t 1 && print TTY_OK",
            ])
            .output()
            .unwrap();
        succeeds(&output);
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("TTY_OK"),
            "{profile}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

/// Compare native public-root discovery with the host while retaining keychain-file denial.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn default_profile_preserves_native_tls_trust_roots() {
    let f = Fixture::new();
    let probe = f.work.join("trust-roots");
    succeeds(
        &Command::new("/usr/bin/cc")
            .args([
                "-Werror",
                "-Wno-deprecated-declarations",
                "-framework",
                "Security",
                "-framework",
                "CoreFoundation",
            ])
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/trust_roots.c"
            ))
            .arg("-o")
            .arg(&probe)
            .output()
            .unwrap(),
    );
    let baseline = Command::new(&probe).env("HOME", &f.home).output().unwrap();
    succeeds(&baseline);
    let sandboxed = f.run("default", &[probe.to_str().unwrap()]);
    succeeds(&sandboxed);
    assert_eq!(
        sandboxed.stdout, baseline.stdout,
        "sandbox changed native certificate availability"
    );
    // Restoring service access must not grant direct access to private keychain
    // files. Native services can independently mediate access through their APIs.
    let keychains = f.home.join("Library/Keychains");
    fs::create_dir_all(&keychains).unwrap();
    fs::write(keychains.join("fake.keychain"), "fake private material").unwrap();
    denied(&f.run(
        "default",
        &[
            "/bin/cat",
            keychains.join("fake.keychain").to_str().unwrap(),
        ],
    ));
}

/// Enforce additive restrictions and protect every source even after a child opens the baseline.
#[test]
#[ignore = "requires a host that permits sandbox-exec; run just test-sandbox"]
fn inherited_profiles_enforce_merged_permissions_and_protect_all_sources() {
    let f = Fixture::new();
    let base = f.home.join("external-base.toml");
    fs::write(&base, "version=1\nextends='isolated'\n[filesystem]\ndeny=['${CWD}/parent-secret']\n[environment]\ndrop=['MY_SECRET']\nset={PARENT_FLAG='present'}").unwrap();
    symlink(&base, f.config.join("base.toml")).unwrap();
    f.profile("child", "version=1\nextends='base'\n[filesystem]\nallow_read=['~/outside']\nallow_write=['${CWD}/parent-secret']\ndeny=['${CWD}/child-secret']\n[environment]\nkeep=['MY_SECRET']\nset={CHILD_FLAG='present',MY_SECRET='still-dropped'}");
    fs::write(f.work.join("parent-secret"), "private").unwrap();
    fs::write(f.work.join("child-secret"), "private").unwrap();
    succeeds(&f.run("child", &["/bin/sh", "-c", "test \"$PARENT_FLAG\" = present && test \"$CHILD_FLAG\" = present && test -z \"${MY_SECRET+x}\" && cat allowed && cat \"$HOME/outside\" && echo ok > created"]));
    denied(&f.run("child", &["/bin/cat", "parent-secret"]));
    denied(&f.run("child", &["/bin/cat", "child-secret"]));
    denied(&f.run("child", &["/bin/sh", "-c", "echo bad > parent-secret"]));
    denied(&f.run("child", &["/bin/sh", "-c", "cat \"$HOME/.ssh/key\""]));
    // Open the baseline too: source protection must not rely on a closed default.
    f.profile("open-child", "version=1\nextends='base'\nmode='open'\nnetwork='allow'\n[filesystem]\nprotect_credentials=false");
    denied(&f.run("open-child", &["/bin/cat", base.to_str().unwrap()]));
    denied(&f.run(
        "open-child",
        &["/bin/sh", "-c", "echo bad > \"$HOME/external-base.toml\""],
    ));
    assert!(
        fs::read_to_string(base)
            .unwrap()
            .contains("extends='isolated'")
    );
}
