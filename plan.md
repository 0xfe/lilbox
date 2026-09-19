# lilbox design and implementation plan

## Goal

Build a small, versioned Rust CLI that launches coding agents, arbitrary commands,
or an interactive shell inside an operating-system sandbox. The first backend is
macOS Seatbelt (`/usr/bin/sandbox-exec`). Unsupported platforms must fail clearly,
never silently run an unsandboxed command.

## Design

- **CLI:** `lilbox codex [args...]`, `lilbox claude [args...]`,
  `lilbox shell [args...]`, and `lilbox exec <command> [args...]`. No arguments
  display help. `-p NAME` selects a profile, `init` writes example profiles without
  overwriting existing files, and `--print-policy` shows the effective sandbox.
  Arguments are passed directly, without constructing a shell command.
- **Configuration:** versioned, strict TOML files in
  `${XDG_CONFIG_HOME:-~/.config}/lilbox/NAME.toml`. The built-in `default` works
  without setup. Named profiles must exist. Profiles are standalone, with
  documented defaults; unknown keys and unsafe profile names are errors.
- **Policy model:** platform-independent mode, filesystem, environment, process,
  and network settings are resolved before passing to a backend. Paths support
  `~`, `${HOME}`, and `${CWD}`; relative paths resolve against the launch directory.
  Existing symlinks and existing ancestors of future paths are resolved, retaining
  both lexical and canonical restrictions. Denies take precedence over grants.
- **Open mode:** allow by default, with a built-in deny list for common credential
  stores, private directories, shell history, and lilbox's own configuration.
  Scrub credential variables, agent sockets, and runtime injection variables.
  Block common credential-service IPC. Keep network available by default.
- **Closed mode:** deny by default, with filesystem grants and runtime
  capabilities explicitly selected in configuration. Environment inheritance and
  network access default off. The shipped `isolated` example explicitly enables a
  minimal command-line runtime and system reads, workspace writes, and temporary
  storage. Explain that additional runtime/tool/authentication grants can be
  necessary; do not claim arbitrary applications work under a universal policy.
- **Execution:** replace the wrapper process on Unix to preserve signals, exit
  codes, and terminal behavior. Preserve standard I/O, close other inherited
  descriptors, set a restrictive umask, and use a dedicated temporary directory.
  Default shell startup skips user rc files where supported so they cannot casually
  reload scrubbed secrets. Cleanup is a tradeoff with process replacement and must
  be addressed explicitly during implementation.
- **Network evolution:** use a typed network policy (`allow` / `deny`) and backend
  capability validation. Future proxy/domain rules belong in this layer; unknown
  or unsupported policies must fail closed rather than be ignored.
- **Security scope:** Seatbelt is OS-enforced confinement, but open mode is a
  practical deny list, not comprehensive secret isolation. Document standard-I/O,
  hard-link/race, local-service, authentication, and deprecated/private Seatbelt
  interface limitations honestly. No downloads or agent permission bypass flags.

## Phases and completion criteria

1. **Foundation and configuration** — scaffold the Cargo package, CLI, strict
   configuration schema, defaults, examples, and portable policy resolution.
   Verify parsing, path handling, profile selection, and environment filtering.
2. **macOS backend and launch** — render escaped Seatbelt policies, implement
   runtime grants and network modes, validate backend support, launch directly,
   and handle shell/default environment behavior. Verify real Seatbelt enforcement
   with read/write, child-process, network, and closed-mode probes on macOS.
3. **Distribution and documentation** — add Just build/check/test/install/release
   recipes, reproducible versioned archives and checksums, CI, licensing, and a
   readable README with quickstart, configuration, examples, extension points,
   and limitations.
4. **Review and verification** — run formatting, lint, all tests, release/build
   smoke checks, and CLI examples. Review security-sensitive paths and error
   behavior; record results and remaining platform constraints here.

Each phase is executed in order. Changes to this design and validation evidence
will be recorded below as implementation proceeds.

## Progress

- Planning: inspected an empty workspace; Rust, Cargo, Just, and macOS
  `sandbox-exec` are available. No repository-specific instructions were found.
- Phase 1 complete: Cargo package version 0.1.0, CLI/profile parsing, strict schema,
  shipped profiles, path resolution, and environment filtering implemented. Nine
  initial unit tests pass. Dependencies are pinned by Cargo.lock.
- Phase 2 in progress: backend interface, Seatbelt renderer, and direct process
  replacement implemented. Protect denied paths' ancestors against rename/unlink.
  Temporary storage uses a persistent, owned 0700 `~/.cache/lilbox/tmp` directory;
  this avoids a supervisor and preserves native terminal/signal behavior without
  leaking a new directory on every launch. Its contents persist until removed.
- Phase 2 complete: six host enforcement tests pass on macOS 26.2 (Apple Silicon).
  Testing exposed and fixed Seatbelt operation-specific allow precedence over
  `file*` denies, and the system runtime's need to open the filesystem root.
  The explicit runtime bundle grants root-directory reads, not recursive reads.
  Nested sandbox-exec is prohibited here; host tests run outside the tool sandbox.
- Phase 3 complete: readable README, MIT license, changelog, Just build/install/
  verification/release recipes, and Linux/macOS CI are in place. Native release
  packaging succeeds and emits a versioned archive plus SHA-256 checksum.
- Phase 4 in progress: final review added protection for individually symlinked
  profiles, tests for overlapping closed-mode denies and real argument forwarding,
  and private temporary-directory validation. Portable checks currently pass.
- Phase 4 complete: `just verify` passes all formatting and warning-free Clippy
  checks, 12 portable unit/CLI tests, and nine real macOS enforcement tests. The
  host suite additionally verifies Unix sockets, non-UTF-8 arguments, pseudo-TTY
  shell behavior, metadata deny precedence, and macOS data-volume aliases.
  System grants were narrowed to `/System/Library` and `/System/Cryptexes` to avoid
  broadly granting `/System/Volumes/Data`.
- Distribution verified: `just install /private/tmp/lilbox-install-smoke` succeeds;
  `just release` produces `dist/lilbox-0.1.0-aarch64-apple-darwin.tar.gz` and its
  checksum. Archive listing and SHA-256 validation pass, the extracted executable
  reports `lilbox 0.1.0`, and repeated packaging of identical inputs produces the
  same SHA-256.
- Remaining scope: only Apple Silicon macOS 26.2 was executed locally. CI is
  configured for additional macOS versions and Linux portable tests; it has not
  run remotely in this workspace. Intel packaging is supported but untested here.
  Linux/Windows execution and managed network proxies are future backends/features.
  Actual authenticated agent sessions were not exercised; named-agent forwarding
  was tested with a fixture executable. Security limitations and nested-sandbox
  behavior are documented in README.md.

## Toolchain upgrade verification — 2026-09-19

- Updated the machine's default stable Rust toolchain from 1.93.0 to 1.98.1
  (`48a229cea`, 2026-09-01), Cargo to 1.98.1, and rustup to 1.29.1.
  The separately installed 1.93.0 toolchain remains available.
- `just verify` passes on the upgraded toolchain: formatting, warning-free Clippy,
  12 portable tests, and nine macOS Seatbelt enforcement tests.
- All nine enforcement tests also pass against the optimized release binary via
  `cargo test --release --locked --test seatbelt -- --ignored --test-threads=1`.
  Release executable help and version smoke checks pass.
- `just release` rebuilt the Apple Silicon archive; SHA-256 verification passes.
  No source or dependency changes were necessary for compatibility.
