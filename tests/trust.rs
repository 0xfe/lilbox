//! Portable profile trust-root, local selection, and scratch-planning regressions.
use lilbox::{config, policy::ContextPaths, temporary};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};

/// Build a disposable context without creating launch-time scratch directories.
fn paths(root: &std::path::Path) -> ContextPaths {
    ContextPaths {
        home: root.join("home"),
        cwd: root.join("work"),
        config: root.join("config"),
        temporary: root.join("unused"),
    }
}

/// Unsafe files, ancestors and symlink targets fail; the CLI opt-out is deliberate.
#[test]
fn profile_ownership_modes_and_replacement_are_checked() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    let file = config.join("work.toml");
    fs::write(&file, "version=1").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(config::load_resolved(&config, "work").is_err());
    assert!(config::load_selected(&config, None, Some("work"), true).is_ok());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(config::load_resolved(&config, "work").is_err());
    fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
    let loaded = config::load_resolved(&config, "work").unwrap();
    fs::rename(&file, config.join("old")).unwrap();
    fs::write(&file, "version=1").unwrap();
    assert!(loaded.revalidate().is_err());
    let loaded = config::load_resolved(&config, "work").unwrap();
    fs::write(&file, "version=1\nnetwork='deny'").unwrap();
    assert!(loaded.revalidate().is_err());
    let target = root.path().join("target.toml");
    fs::write(&target, "version=1").unwrap();
    symlink(&target, config.join("linked.toml")).unwrap();
    let loaded = config::load_resolved(&config, "linked").unwrap();
    fs::remove_file(config.join("linked.toml")).unwrap();
    symlink(&file, config.join("linked.toml")).unwrap();
    assert!(loaded.revalidate().is_err());
    fs::set_permissions(&target, fs::Permissions::from_mode(0o666)).unwrap();
    symlink(&target, config.join("unsafe.toml")).unwrap();
    assert!(config::load_resolved(&config, "unsafe").is_err());
}

/// Local selection is CWD-only, cannot override -p, and protects the source target.
#[test]
fn local_selection_precedence_integrity_and_read_only_inspection() {
    let root = tempfile::tempdir().unwrap();
    let mut paths = paths(root.path());
    fs::create_dir_all(paths.cwd.join(".lilbox")).unwrap();
    let local = paths.cwd.join(".lilbox/profile.toml");
    fs::write(&local, "version=1\nextends='isolated'\nnetwork='allow'").unwrap();
    let loaded = config::load_selected(&paths.config, Some(&paths.cwd), None, false).unwrap();
    assert_eq!(loaded.chain, ["isolated", "local"]);
    assert_eq!(loaded.sources.last(), Some(&local));
    assert_eq!(
        temporary::select(&loaded.profile, &mut paths).unwrap(),
        temporary::Kind::Directory
    );
    assert_eq!(paths.temporary, paths.cwd.join(".lilbox/tmp"));
    assert!(loaded.policy(&paths).unwrap().deny.contains(&local));
    assert!(!paths.temporary.exists());
    assert_eq!(
        config::load_selected(&paths.config, Some(&paths.cwd), Some("permissive"), false)
            .unwrap()
            .profile
            .mode,
        config::Mode::Open
    );
    fs::write(&local, "broken").unwrap();
    assert!(config::load_selected(&paths.config, Some(&paths.cwd), None, false).is_err());
    let output = Command::new(env!("CARGO_BIN_EXE_lilbox"))
        .env_clear()
        .env("HOME", &paths.home)
        .current_dir(&paths.cwd)
        .arg("--config-dir")
        .arg(&paths.config)
        .arg("check")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(".lilbox/profile.toml"));
    fs::create_dir(paths.cwd.join("child")).unwrap();
    assert_eq!(
        config::load_selected(&paths.config, Some(&paths.cwd.join("child")), None, false)
            .unwrap()
            .chain
            .last()
            .unwrap(),
        "default"
    );
}

/// Fresh embedded profiles cannot silently change to files during launch preparation.
#[test]
fn embedded_fallback_and_temp_overrides_remain_explicit() {
    let root = tempfile::tempdir().unwrap();
    let mut paths = paths(root.path());
    fs::create_dir(&paths.cwd).unwrap();
    let loaded = config::load_resolved(&paths.config, "default").unwrap();
    assert_eq!(
        temporary::select(&loaded.profile, &mut paths).unwrap(),
        temporary::Kind::Private
    );
    config::init(&paths.config).unwrap();
    assert!(loaded.revalidate().is_err());
    for (setting, expected) in [
        ("system", temporary::Kind::System),
        ("${CWD}/scratch", temporary::Kind::Directory),
    ] {
        let profile = config::parse(&format!(
            "version=1\n[runtime]\ntemporary_directory=true\ntemp_dir='{setting}'"
        ))
        .unwrap();
        assert_eq!(temporary::select(&profile, &mut paths).unwrap(), expected);
    }
    assert!(!paths.cwd.join("scratch").exists());
}

/// Per-file initialization must not replace a neighboring file or accept traversal.
#[test]
fn initialization_selects_one_shipped_file() {
    let root = tempfile::tempdir().unwrap();
    config::init(root.path()).unwrap();
    fs::write(root.path().join("isolated.toml"), "user custom").unwrap();
    fs::write(root.path().join("default.toml"), "user custom").unwrap();
    config::init_files(
        root.path(),
        config::InitMode::Overwrite,
        &["default.toml".into()],
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("isolated.toml")).unwrap(),
        "user custom"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("default.toml")).unwrap(),
        config::DEFAULT_PROFILE
    );
    assert!(
        config::init_files(
            root.path(),
            config::InitMode::Overwrite,
            &["../outside".into()]
        )
        .is_err()
    );
}
