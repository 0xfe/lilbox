# Working on lilbox

## Project overview

lilbox is a Rust CLI that launches coding agents, arbitrary commands, and shells
inside an OS sandbox. macOS Seatbelt (`/usr/bin/sandbox-exec`) is the only execution
backend currently implemented. Unsupported platforms must refuse execution.

The default profile is intentionally permissive, especially for networking, with
specific credential/private filesystem denies and environment scrubbing. The
`isolated` example starts closed and explicitly grants system reads, CWD writes,
basic runtime operations, and scratch storage. Do not confuse either mode with a
VM or separate user account.

Read [README.md](README.md) for usage,
[docs/design.md](docs/design.md) for architecture, and
[docs/profiles.md](docs/profiles.md) for effective permissions. [plan.md](plan.md)
is a historical implementation/verification record.

## Code map

- `src/main.rs`: CLI, configuration lookup, launch orchestration.
- `src/config.rs`: strict, versioned TOML schema and non-overwriting initialization.
- `src/policy.rs`: portable policy, path expansion/canonicalization, credential list.
- `src/environment.rs`: environment filtering/normalization and shell arguments.
- `src/backend/`: backend interface, Seatbelt rendering, and basic runtime SBPL.
- `src/launch.rs`: private scratch directory and hardened Unix process replacement.
- `profiles/`: shipped TOML files embedded into the executable.
- `tests/seatbelt.rs`: actual macOS enforcement tests; `tests/fixtures/trust_roots.c`
  checks native certificate discovery without network access or private keys.

Keep OS-specific enforcement in the backend/launch layers. New capabilities must
be represented explicitly and either enforced or rejected by each backend.

## Building and running

Use Rust/Cargo, rustfmt, Clippy, and Just. `Cargo.toml` declares edition 2024 and
the minimum Rust version; keep `Cargo.lock` tracked. Use the current stable
toolchain for routine development without silently raising the declared minimum.
macOS host tests need developer tools for the C certificate probe. Release
packaging additionally needs Python 3.

```sh
just                          # list recipes
just build                    # debug build, locked dependencies
just run --help
just run --print-policy
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
just test-sandbox             # macOS host enforcement tests
just verify                   # check + test-sandbox
```

For changes to permissions, path handling, environment rules, process launch, or
TLS/network behavior, add meaningful regression coverage and run `just verify`
on a host that permits `sandbox-exec`. The enforcement tests are intentionally
ignored by ordinary `cargo test`; they must be run explicitly. Nested Seatbelt
initialization can fail with `sandbox_apply: Operation not permitted`. Report
that limitation instead of silently skipping failures or claiming enforcement
was tested. Use the available host execution permission mechanism when needed.

For release-specific launch changes, the optimized enforcement suite is:

```sh
cargo test --release --locked --test seatbelt -- --ignored --test-threads=1
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
- Keep configuration paths protected, including individually symlinked profiles.
- Preserve native TLS certificate discovery in online profiles. Blocking
  SecurityServer/securityd blocks public trust roots too. Never work around TLS
  failures by disabling certificate verification.
- Keep inherited proxy and public CA settings usable in the permissive default;
  retain secret/injection filtering and explicit drop precedence.
- Do not imply that private-keychain file denial blocks all service-mediated
  keychain access. Document the actual boundary.
- Keep allocation/locking out of the `pre_exec` callback; descriptor enumeration
  relies on the current single-threaded launch design.
- Avoid broad `/System` grants that expose `/System/Volumes/Data`.

## Releases

`Cargo.toml` is the version source. Update its version and `CHANGELOG.md` for a
release, refresh `Cargo.lock`, and keep version references in README consistent.

```sh
just version
just verify
just release                         # native macOS target
just release x86_64-apple-darwin       # matching Rust target must be installed
```

`scripts/release.sh` supports Apple Silicon and Intel macOS, builds with locked
dependencies, and writes versioned archives/checksums in `dist/`. Archives include
the executable, README, license, changelog, profiles, and `docs/`. Verify the
SHA-256 sidecar from the archive directory and smoke-test the extracted executable.
Packaging is local: it does not sign, notarize, tag, or publish a release.
`target/` and `dist/` are ignored build outputs.

CI runs portable checks on Linux/macOS and host enforcement plus packaging on
macOS. Do not claim an unexecuted platform or CI run was verified.

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
