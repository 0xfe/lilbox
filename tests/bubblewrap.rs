//! Actual Linux enforcement. Explicitly run with `just test-sandbox`.
#![cfg(target_os = "linux")]

use std::{
    fs,
    net::TcpListener,
    os::unix::{fs::symlink, net::UnixListener},
    path::PathBuf,
    process::{Command, Output},
};

/// Disposable host filesystem used to distinguish real enforcement from setup errors.
struct Fixture {
    /// Own the tree until every sandboxed command in the test has finished.
    _root: tempfile::TempDir,
    /// Fake HOME containing credentials, ordinary files, and scratch storage.
    home: PathBuf,
    /// Granted workspace, including existing .env names to avoid incidental restrictions.
    work: PathBuf,
    /// Protected directory holding starter and test-specific profiles.
    config: PathBuf,
}

impl Fixture {
    /// Create fake credentials and writable fixtures entirely inside a disposable home.
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let work = root.path().join("work");
        let config = home.join(".config/lilbox");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir(&work).unwrap();
        fs::create_dir(home.join(".ssh")).unwrap();
        fs::write(home.join(".ssh/key"), "fake credential").unwrap();
        fs::write(home.join("outside"), "outside").unwrap();
        fs::write(work.join("allowed"), "allowed").unwrap();
        fs::write(work.join(".env"), "fake env").unwrap();
        fs::write(work.join(".env.local"), "fake local env").unwrap();
        fs::create_dir(work.join("src")).unwrap();
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

    /// Start with a clean environment and select the fixture profile and working directory.
    fn command(&self, profile: &str) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lilbox"));
        cmd.env_clear()
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&self.work)
            .arg("--config-dir")
            .arg(&self.config)
            .args(["-p", profile]);
        cmd
    }

    /// Capture one real sandboxed command, retaining stderr for enforcement assertions.
    fn run(&self, profile: &str, args: &[&str]) -> Output {
        self.command(profile)
            .arg("exec")
            .args(args)
            .output()
            .unwrap()
    }

    /// Write a custom profile into this fixture only.
    fn profile(&self, name: &str, source: &str) {
        fs::write(self.config.join(format!("{name}.toml")), source).unwrap();
    }
}

/// Require launch success and return UTF-8 stdout, attributing failures to the caller.
#[track_caller]
fn succeeds(output: Output) -> String {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Require a target access failure, rejecting bubblewrap setup errors as evidence.
#[track_caller]
fn denied(output: Output) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("bwrap:"),
        "sandbox setup failed, not an enforcement test: {stderr}"
    );
    assert!(
        stderr.contains("Permission denied")
            || stderr.contains("Operation not permitted")
            || stderr.contains("Read-only file system")
            || stderr.contains("Device or resource busy")
            || stderr.contains("No such file"),
        "{stderr}"
    );
}

/// Open access must preserve deny protection through aliases, writes, and renames.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn open_denies_credentials_aliases_ancestor_renames_and_config() {
    let f = Fixture::new();
    succeeds(f.run("default", &["/bin/sh", "-c", "cat allowed; echo new > created; echo changed > allowed; echo outside > \"$HOME/outside\""]));
    assert_eq!(fs::read_to_string(f.work.join("created")).unwrap(), "new\n");
    symlink(f.home.join(".ssh"), f.work.join("alias")).unwrap();
    for path in [
        f.home.join(".ssh/key"),
        f.work.join("alias/key"),
        f.work.join(".env"),
        f.work.join(".env.local"),
        f.config.join("isolated.toml"),
    ] {
        denied(f.run("default", &["/bin/cat", path.to_str().unwrap()]));
    }
    denied(f.run("default", &["/bin/sh", "-c", "echo bad > .env"]));
    denied(f.run("default", &["/bin/sh", "-c", "chmod 700 \"$HOME/.ssh\""]));
    denied(f.run(
        "default",
        &["/bin/sh", "-c", "mv \"$HOME\" \"$HOME-moved\""],
    ));
    denied(f.run(
        "default",
        &[
            "/bin/sh",
            "-c",
            "mv \"$HOME/.config\" \"$HOME/config-moved\"",
        ],
    ));
    assert_eq!(fs::read_to_string(f.work.join(".env")).unwrap(), "fake env");
}

/// Granted writes reach the host while ungranted home/system paths remain inaccessible.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn closed_grants_persist_writes_and_do_not_expose_ungranted_home() {
    let f = Fixture::new();
    succeeds(f.run("isolated", &["/bin/sh", "-c", "cat allowed; cat /etc/os-release; echo okay > created; echo scratch > \"$TMPDIR/probe\"; /bin/sh -c 'echo child > src/child'"]));
    assert_eq!(
        fs::read_to_string(f.work.join("created")).unwrap(),
        "okay\n"
    );
    assert_eq!(
        fs::read_to_string(f.home.join(".cache/lilbox/tmp/probe")).unwrap(),
        "scratch\n"
    );
    denied(f.run(
        "isolated",
        &["/bin/cat", f.home.join("outside").to_str().unwrap()],
    ));
    denied(f.run("isolated", &["/bin/cat", ".env"]));
    denied(f.run(
        "isolated",
        &["/bin/sh", "-c", "echo bad > /etc/lilbox-must-not-exist"],
    ));
    denied(f.run("isolated", &["/bin/sh", "-c", "mkdir /ungranted"]));
}

/// Missing denies freeze parent entries but retain writes to existing allowed children.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn future_denies_reserve_names_without_host_side_effects() {
    let f = Fixture::new();
    f.profile(
        "future",
        "version=1\nextends='default'\n[filesystem]\ndeny=['${CWD}/future/key']",
    );
    let before: Vec<_> = fs::read_dir(&f.work)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let report = succeeds(f.command("future").arg("check").output().unwrap());
    assert!(report.contains("read-only directory entries"));
    succeeds(f.run(
        "future",
        &[
            "/bin/sh",
            "-c",
            "echo updated > allowed; echo okay > src/created",
        ],
    ));
    denied(f.run("future", &["/bin/mkdir", "future"]));
    denied(f.run("future", &["/bin/sh", "-c", "ln -s src future"]));
    denied(f.run("future", &["/bin/mv", "src", "future"]));
    denied(f.run("future", &["/bin/sh", "-c", "echo bad > new-sibling"]));
    let after: Vec<_> = fs::read_dir(&f.work)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
    assert!(!f.work.join("future").exists());
    // Built-in absent credentials use the same enforcement, including .env.
    fs::remove_file(f.work.join(".env")).unwrap();
    denied(f.run("default", &["/bin/sh", "-c", "echo bad > .env"]));
    assert!(!f.work.join(".env").exists());
}

/// Child grants cannot expose inherited denies or externally symlinked profile sources.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn inherited_profile_symlink_targets_and_nested_deny_precedence() {
    let f = Fixture::new();
    let parent = f.home.join("parent.toml");
    fs::write(
        &parent,
        "version=1\nextends='default'\n[filesystem]\ndeny=['${CWD}/src/private']",
    )
    .unwrap();
    symlink(&parent, f.config.join("parent.toml")).unwrap();
    fs::create_dir(f.work.join("src/private")).unwrap();
    fs::write(f.work.join("src/private/key"), "fake").unwrap();
    f.profile(
        "child",
        "version=1\nextends='parent'\n[filesystem]\nallow_write=['${CWD}/src/private', '~']",
    );
    denied(f.run("child", &["/bin/cat", parent.to_str().unwrap()]));
    denied(f.run("child", &["/bin/cat", "src/private/key"]));
    denied(f.run("child", &["/bin/mv", "src", "moved"]));
    succeeds(f.run("child", &["/bin/sh", "-c", "echo okay > src/allowed"]));
}

/// Online mode retains sockets and CA discovery; offline mode rejects TCP, UDP, and Unix.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn online_tcp_unix_and_public_ca_reads_offline_socket_denial() {
    let f = Fixture::new();
    f.profile("offline", "version=1\nextends='default'\nnetwork='deny'");
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    let socket = f.work.join("service.sock");
    let _unix = UnixListener::bind(&socket).unwrap();
    let tcp_script = format!(
        "import socket; socket.create_connection(('127.0.0.1', {}), 2)",
        tcp.local_addr().unwrap().port()
    );
    let unix_script = "import socket; s=socket.socket(socket.AF_UNIX); s.connect('service.sock')";
    for script in [tcp_script.as_str(), unix_script] {
        succeeds(f.run("default", &["/usr/bin/python3", "-c", script]));
        denied(f.run("offline", &["/usr/bin/python3", "-c", script]));
        denied(f.run("isolated", &["/usr/bin/python3", "-c", script]));
    }
    denied(f.run(
        "offline",
        &[
            "/usr/bin/python3",
            "-c",
            "import socket; socket.socket(socket.AF_INET, socket.SOCK_DGRAM)",
        ],
    ));
    succeeds(f.run(
        "default",
        &[
            "/usr/bin/python3",
            "-c",
            "import ssl; assert ssl.create_default_context().cert_store_stats()['x509_ca'] > 0",
        ],
    ));
}

/// Check runtime hardening alongside argument fidelity, shell startup, and exit propagation.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn runtime_filters_process_visibility_environment_arguments_and_exit_status() {
    let f = Fixture::new();
    let text = succeeds(f.command("default").env("OPENAI_API_KEY", "fake-secret").env("HTTPS_PROXY", "http://example.invalid:8080")
        .args(["exec", "/usr/bin/python3", "-c", "import os, resource; assert 'OPENAI_API_KEY' not in os.environ; assert os.environ['HTTPS_PROXY']=='http://example.invalid:8080'; assert resource.getrlimit(resource.RLIMIT_CORE)==(0,0); assert os.umask(0o077)==0o077; assert len([x for x in os.listdir('/proc') if x.isdigit()]) < 10; print('okay')"]).output().unwrap());
    assert_eq!(text.trim(), "okay");
    denied(f.run("default", &["/usr/bin/unshare", "-Ur", "/bin/true"]));
    let output = f.run("default", &["/bin/sh", "-c", "exit 37"]);
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(
        succeeds(f.run(
            "default",
            &["/usr/bin/printf", "%s", "a; echo injected ' --bind / /"]
        )),
        "a; echo injected ' --bind / /"
    );
    // Without SHELL, Linux selects /bin/sh, which receives the explicit -c.
    succeeds(
        f.command("default")
            .args(["shell", "-c", "exit 0"])
            .output()
            .unwrap(),
    );
}

/// Inject a credential FD before lilbox starts and require no extra FDs in the target.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn extra_inherited_descriptors_are_closed_and_backend_descriptors_are_consumed() {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let f = Fixture::new();
    let secret = fs::File::open(f.home.join(".ssh/key")).unwrap();
    let source = secret.as_raw_fd();
    let mut cmd = f.command("default");
    // Deliberately pass a normally close-on-exec secret descriptor to lilbox.
    // The target must lose access even though no path lookup is needed to read it.
    unsafe {
        cmd.pre_exec(move || {
            if libc::dup2(source, 100) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    succeeds(cmd.args(["exec", "/usr/bin/python3", "-c", "import os; assert not os.path.exists('/proc/self/fd/100'); assert not [fd for fd in os.listdir('/proc/self/fd') if fd.isdigit() and int(fd)>2 and os.path.exists('/proc/self/fd/'+fd)]"]).output().unwrap());
}

/// Canonical mount ordering must preserve both exclusions and nested writable grants.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn closed_symlink_grants_cannot_alias_around_denies_or_lose_nested_writes() {
    let f = Fixture::new();
    let shared = f.home.join("shared");
    fs::create_dir_all(shared.join("private")).unwrap();
    fs::create_dir(shared.join("writable")).unwrap();
    fs::write(shared.join("private/key"), "fake").unwrap();
    symlink(&shared, f.home.join("alias")).unwrap();
    f.profile("aliases", "version=1\nextends='isolated'\n[filesystem]\nallow_read=['~/alias']\nallow_write=['~/alias/writable']\ndeny=['~/shared/private']");
    denied(f.run(
        "aliases",
        &[
            "/bin/cat",
            f.home.join("alias/private/key").to_str().unwrap(),
        ],
    ));
    denied(f.run(
        "aliases",
        &["/bin/sh", "-c", "echo bad > \"$HOME/alias/readonly\""],
    ));
    succeeds(f.run(
        "aliases",
        &["/bin/sh", "-c", "echo okay > \"$HOME/alias/writable/new\""],
    ));
    assert_eq!(
        fs::read_to_string(shared.join("writable/new")).unwrap(),
        "okay\n"
    );
}

/// Retain usable PTYs and expected exit codes after detaching the host terminal session.
#[test]
#[ignore = "requires Linux bubblewrap and user namespaces; run just test-sandbox"]
fn terminal_io_and_signal_exit_status() {
    let f = Fixture::new();
    assert_eq!(
        f.run("default", &["/bin/sh", "-c", "kill -TERM $$"])
            .status
            .code(),
        Some(128 + libc::SIGTERM)
    );
    let script = r#"
import errno, os, pty, sys
pid, fd = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], sys.argv[1:])
while True:
    try:
        data = os.read(fd, 4096)
        if not data: break
        sys.stdout.buffer.write(data)
    except OSError as error:
        if error.errno != errno.EIO: raise
        break
_, status = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(status))
"#;
    for profile in ["default", "isolated"] {
        // Exercise both normal shell cleanup and an explicit nonzero exit.
        // Inheriting the host controlling session made dash return 2 on exit
        // even after printing TTY_OK, because its original process group was
        // invisible inside the sandbox's PID namespace.
        for (source, expected_status) in [
            ("test -t 0 && test -t 1 && echo TTY_OK", 0),
            ("test -t 0 && test -t 1 || exit 99; echo TTY_OK; exit 7", 7),
        ] {
            let output = Command::new("/usr/bin/python3")
                .args(["-c", script, env!("CARGO_BIN_EXE_lilbox"), "--config-dir"])
                .arg(&f.config)
                .args(["-p", profile, "shell", "-c", source])
                .env_clear()
                .env("HOME", &f.home)
                .env("PATH", "/usr/bin:/bin")
                .env("SHELL", "/bin/sh")
                .current_dir(&f.work)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(expected_status), "{output:?}");
            let terminal = String::from_utf8(output.stdout).unwrap();
            assert!(terminal.contains("TTY_OK"), "{terminal}");
            assert!(
                !terminal.contains("Cannot set tty process group"),
                "{terminal}"
            );
        }
    }
}
