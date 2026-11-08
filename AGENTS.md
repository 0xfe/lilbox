# Working on lilbox

## Project overview

lilbox is a Rust CLI that launches coding agents, arbitrary commands, and shells
inside an OS sandbox. Execution backends are macOS Seatbelt
(`/usr/bin/sandbox-exec`) and Linux bubblewrap (`/usr/bin/bwrap`, x86_64/aarch64).
Unsupported platforms and syscall ABIs must refuse execution.

The default profile is intentionally permissive, especially for networking, with
specific credential/private filesystem denies and environment scrubbing. The
`isolated` example starts closed and explicitly grants system reads, CWD writes,
basic runtime operations, and scratch storage. Do not confuse either mode with a
VM or separate user account.

Read [README.md](README.md) for usage,
[docs/design.md](docs/design.md) for architecture,
[docs/profiles.md](docs/profiles.md) for effective permissions,
[docs/security.md](docs/security.md) for security boundaries, and
[docs/development.md](docs/development.md) for testing and releases. [plan.md](plan.md)
is a historical implementation/verification record.

## Code map

- `src/main.rs`: CLI, configuration lookup, launch orchestration.
- `src/config.rs`: strict TOML schema, inheritance resolution, dependency protection,
  and non-overwriting initialization.
- `src/check.rs`: read-only profile discovery and configuration/precedence diagnostics.
- `src/policy.rs`: portable policy, path expansion/canonicalization, credential list.
- `src/environment.rs`: environment filtering/normalization and shell arguments.
- `src/backend/`: backend interface, Seatbelt rendering/runtime SBPL, and Linux
  bubblewrap mount planning plus seccomp filtering.
- `src/launch.rs`: private scratch directory and hardened Unix process replacement.
- `profiles/`: shipped TOML files embedded into the executable.
- `examples/`: annotated standalone/inherited profiles; validate with
  `just run --config-dir examples check --strict` when changing the gallery.
  On Linux, review expected missing-deny parent warnings as well; strict mode
  intentionally fails on those filesystem-dependent warnings.
- `tests/bubblewrap.rs`: actual Linux enforcement; needs bubblewrap 0.6.3+,
  permitted user namespaces/seccomp, Python 3, and public CA certificates.
- `tests/seatbelt.rs`: actual macOS enforcement tests; `tests/fixtures/trust_roots.c`
  checks native certificate discovery without network access or private keys.

Keep OS-specific enforcement in the backend/launch layers. New capabilities must
be represented explicitly and either enforced or rejected by each backend.

## Building and running

Use Rust/Cargo, rustfmt, Clippy, and Just. `Cargo.toml` declares edition 2024 and
the minimum Rust version; keep `Cargo.lock` tracked. Use the current stable
toolchain for routine development without silently raising the declared minimum.
macOS host tests need developer tools for the C certificate probe.
Installer/release-script tests and release packaging additionally need Python 3.9+.

```sh
just                          # list recipes
just build                    # debug build, locked dependencies
just run --help
just run --print-policy
just run check                # inspect every profile without launching
just run -p default check --strict
just fmt
just install                  # writes ~/.local/bin/lilbox
just install /custom/prefix    # writes PREFIX/bin/lilbox
```

`lilbox init` writes missing profiles to the user's config directory and never
overwrites existing files. Prefer temporary homes/config directories for tests;
do not modify real credentials or user profiles as test fixtures.

## Testing

```sh
just check                    # formatting, Clippy -D warnings, portable tests
just test-sandbox             # current macOS/Linux host enforcement tests
just verify                   # check + test-sandbox
```

For changes to permissions, path handling, environment rules, process launch, or
TLS/network behavior, add meaningful regression coverage and run `just verify`
on a host that permits the applicable backend (`sandbox-exec` or bubblewrap
user namespaces/seccomp). The enforcement tests are intentionally
ignored by ordinary `cargo test`; they must be run explicitly. Nested Seatbelt
initialization can fail with `sandbox_apply: Operation not permitted`. Report
that limitation instead of silently skipping failures or claiming enforcement
was tested. Use the available host execution permission mechanism when needed.

For release-specific launch changes, the optimized enforcement suite is:

```sh
cargo test --release --locked --test seatbelt -- --ignored --test-threads=1
# On Linux:
cargo test --release --locked --test bubblewrap -- --ignored --test-threads=1
```

For documentation-only changes, verify links, commands, and consistency with the
source; a full sandbox run is unnecessary. Check the release archive if packaging
contents change. Finish with `git diff --check` and review the diff.

## Security-sensitive implementation constraints

- Never retry a failed sandbox launch without confinement.
- Pass executable arguments directly; do not concatenate a shell command.
- Preserve explicit-deny precedence, canonical/lexical path protection, and
  protection against renaming ancestors of denied paths. Seatbelt operation
  specificity matters; a generic `file*` deny alone is insufficient.
- Keep configuration paths protected, including all inherited and individually
  symlinked profiles. Preserve explicit child overrides, additive lists, deferred
  schema defaults, and cycle/depth rejection.
- Keep check and launch resolution/validation consistent. Check must remain
  read-only, aggregate independent failures, and omit assignment values from
  semantic diagnostics. Warnings describe precedence; they do not alter policy.
- Preserve native TLS certificate discovery in online profiles. Blocking
  SecurityServer/securityd blocks public trust roots too. Never work around TLS
  failures by disabling certificate verification.
- Keep inherited proxy and public CA settings usable in the permissive default;
  retain secret/injection filtering and explicit drop precedence.
- Do not imply that private-keychain file denial blocks all service-mediated
  keychain access. Document the actual boundary.
- On Linux, preserve missing-path deny protection without creating host
  placeholders: read-only parents and their diagnostics are intentional. Apply
  mounts parent-first and denies last; preserve canonical symlink semantics.
- Linux network deny must block Unix sockets as well as TCP/UDP. A network
  namespace alone is insufficient. Keep syscall ABI checks and descriptor ownership.
- Keep allocation/locking out of the `pre_exec` callback; descriptor enumeration
  relies on the current single-threaded launch design.
- Avoid broad `/System` grants that expose `/System/Volumes/Data`.

## Releases

`Cargo.toml` is the version source. Update its version and `CHANGELOG.md` for a
release, refresh `Cargo.lock`, and keep version references in README consistent.

```sh
just version
just verify
just package                         # local native macOS/Linux archive
just package x86_64-apple-darwin       # matching Rust target must be installed
just release                         # explicitly request GitHub publication from pushed main
```

`scripts/release.sh` supports ARM64 and x86_64 macOS/Linux (GNU or musl on Linux), builds with locked
dependencies, and writes versioned archives/checksums in `dist/`. Archives include
the executable, README, license, changelog, `install.sh`, profiles, `examples/`, and `docs/`. Verify the
SHA-256 sidecar from the archive directory and smoke-test the extracted executable.
`just package` is local: it does not sign, notarize, tag, or publish a release.
`just release` requires clean, pushed main and dispatches `.github/workflows/release.yml`
with an exact commit/version. Publication waits for all four native targets to
pass tests, optimized enforcement, packaging, and archive smoke tests. Push/PR CI
never publishes. Keep the shared verification workflow and installer target mapping
aligned. Test installer/request scripts offline with `just test-release`; never
publish a release merely to test automation.
`target/` and `dist/` are ignored build outputs.

CI runs portable checks, host enforcement, and packaging on Linux/macOS. Do not claim an unexecuted platform or CI run was verified.
The disposable Ubuntu CI runners load a bwrap-specific AppArmor `userns` grant
and probe namespace startup; do not disable enforcement tests or add host-policy
changes to the installed CLI/installer to work around runner restrictions.

## Documentation must stay current

**Keep `docs/` updated in the same change as the implementation.** In particular,
**update `docs/profiles.md` whenever code changes alter profile behavior**, even
if no TOML file changes. This includes shared credential paths, environment
filters/defaults, directory grants, certificate/service access, network rules,
runtime capabilities, shell startup, and descriptor or temporary-file behavior.

Update `docs/design.md` when architecture, module responsibilities, configuration
resolution, backend behavior, execution, or release packaging changes. Update
README for user-facing command/configuration changes. Keep this AGENTS.md aligned
with build/test/release commands, and preserve release packaging of the docs.
Describe effective permissions and their limitations, not just profile fields.
