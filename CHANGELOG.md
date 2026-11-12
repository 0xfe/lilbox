# Changelog

## 0.4.1 — 2026-11-12

- Select `.lilbox/profile.toml` automatically from the working directory, unless
  overridden with `-p`. Show the selected profile path at startup and inspection.
- Support whole-name environment globs with `*` and `?`, including `*_API_KEY`.
- Ship separate macOS/Linux defaults extending `isolated`, with explicit coding
  tool, workspace, cache, agent-state, and network permissions. Existing installed
  profiles retain their settings until explicitly replaced.
- Validate profile ownership, permissions, and opened-file identity before launch.
  Expand credential-path protection, filter inherited proxy credentials, add
  independent service IPC controls, and provide `check --secrets` filename scans.
- Use `.lilbox/tmp` when the project directory exists, otherwise per-run Linux
  tmpfs or a private macOS directory. Support custom temporary paths; permissive
  uses system temp. macOS private directories persist after successful runs.
- Prompt separately for each installer profile/guide replacement, simplify
  profile inspection, and document project-local configuration for coding agents.
- Update rustix to 1.1.6 and add dependency advisory checks to CI and releases.

## 0.4.0 — 2026-11-11

- Linux now ignores deny paths absent at startup instead of restricting parent
  directory entries. Startup and checks summarize three ignored paths and the
  remainder count. Paths created later in the run may be accessible.
- Replace `--print-policy` with the indented, colored `policy` subcommand. Add
  `profile check`, `profile show --all --origins`, and read-only `profile cani`
  queries for paths, environment names, and networking.
- Move the open baseline into `permissive`; the shipped `default` inherits it.
  Both have embedded fallbacks. Existing standalone defaults remain valid.
- Bundle the agent customization guide in docs and install it as config AGENTS.md.
  Launches and init report the guide path. Installer upgrades prompt before
  replacing shipped profiles/guide; decline/no terminal writes versioned
  alternatives. `--overwrite-profiles` explicitly opts into replacement.
- Expand source comments, profile documentation, and regression coverage.
- Add the landing page and separate website/installer publishing workflows.

## 0.3.1 — 2026-11-08

- Add `just bump [VERSION]` and `just release [VERSION]` with stable-version
  validation and checks against existing local tags, remote tags, and GitHub
  releases. Version preparation updates the manifest, lockfile, README, and
  changelog; commit and push remain manual.
- Initialize missing starter profiles after binary installation without
  overwriting existing configs or symlinks. Show a colored box logo, config and
  executable locations, PATH setup, quick-start commands, and recovery guidance
  when initialization fails.
- Test installation with real native binaries, including config preservation,
  custom config locations, failure recovery, terminal colors, and PATH quoting.

## 0.3.0 — 2026-11-08

- Start Linux sandboxes in a new session so interactive shells preserve their
  exit status instead of failing to restore a host terminal process group.
  Standard terminal I/O is retained; host terminal job control is unavailable.
- Add manually requested GitHub binary releases through `just release`, with
  native tests and verified archives for Apple Silicon/Intel macOS and
  ARM64/x86_64 Linux. Pushes and pull requests run CI without publishing.
- Add a curl-compatible installer with platform detection, SHA-256 verification,
  optional version/prefix selection, and atomic executable replacement. Linux
  release binaries use static musl; bubblewrap remains a system dependency.
- Rename local-only archive creation to `just package`.
- Show the embedded version in help and support `-v`, `-V`, and `--version`.
  Display the selected profile and escaped command on stderr before launching,
  with terminal colors respecting `NO_COLOR` and `TERM=dumb`.

## 0.2.0 — 2026-11-08

- Add Linux bubblewrap execution on x86_64 and aarch64, with private process/device
  views, filesystem denies, offline socket filtering, and fail-closed startup.
  Missing denied paths conservatively restrict directory entry changes in their
  nearest existing parent; `check` reports these restrictions.
- Add Linux system grants to starter profiles, Linux credential locations,
  host enforcement tests, CI coverage, and Linux release packaging.

- Add annotated example profiles for standalone, inherited, offline, and online
  configurations, with usage instructions and inclusion in release archives.

- Add recursive single-parent profile inheritance with explicit scalar overrides,
  additive lists, assignment merging, cycle/depth validation, and protection for
  every inherited configuration file.
- Add `lilbox check` and `lilbox -p NAME check`, with aggregated errors, conflict
  warnings, terminal-aware reporting, and `--strict` for warning-free checks.

- Fix native macOS root certificate discovery in online profiles, which could
  cause Codex HTTPS/WebSocket connections to fail with `UnknownIssuer`.
- Preserve inherited proxy and CA-bundle settings for permissive networking.
- Add offline native-certificate regression coverage while retaining direct
  private-keychain file protection and network-deny enforcement.

## 0.1.0 — 2026-09-19

- Initial macOS Seatbelt backend and portable policy/backend interface.
- Direct agent, shell, and command launch with native exit/signal behavior.
- Open and closed profiles, credential path protection, environment scrubbing,
  private temporary storage, and network allow/deny modes.
- Strict versioned TOML configuration and non-overwriting profile initialization.
- Portable tests, macOS enforcement tests, Just recipes, CI, and release archives.
