# Changelog

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
