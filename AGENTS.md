# Working on lilbox

## Project overview

lilbox is a Rust CLI that launches coding agents, arbitrary commands, and shells
inside an OS sandbox. Execution backends are macOS Seatbelt
(`/usr/bin/sandbox-exec`) and Linux bubblewrap (`/usr/bin/bwrap`, x86_64/aarch64).
Unsupported platforms and syscall ABIs must refuse execution.

Read [README.md](README.md) for usage,
[docs/design.md](docs/design.md) for architecture,
[docs/profiles.md](docs/profiles.md) for effective permissions,
[docs/security.md](docs/security.md) for security boundaries, and
[docs/development.md](docs/development.md) for testing and releases. [plan.md](plan.md)
is a historical implementation/verification record.

## Code map

- `src/main.rs`: CLI, configuration lookup, launch orchestration.
- `src/config.rs`: strict TOML schema, inheritance resolution, dependency protection,
  and provenance tracking.
- `src/check.rs`: read-only profile discovery and configuration/precedence diagnostics.
- `src/policy.rs`: portable policy, path expansion/canonicalization, credential list.
- `src/profiles.rs`: profile inventory, spelling suggestions, and project-only creation.
- `src/doctor.rs`: real startup probe and actionable host/configuration diagnostics.
- `src/discovery.rs`: optional bounded filename-only project-secret inventory.
- `src/display.rs`: terminal-safe formatting, colors, and native policy indentation.
- `src/inspect.rs`: effective profile summaries, origins, and static cani queries.
- `src/environment.rs`: environment filtering/normalization and shell arguments.
- `src/backend/`: backend interface, Seatbelt rendering/runtime SBPL, and Linux
  bubblewrap mount planning plus seccomp filtering.
- `src/launch.rs`: scratch validation and hardened Unix process replacement.
- `src/temporary.rs`: side-effect-free scratch selection and launch-only allocation.
- `src/trust.rs`: descriptor-backed profile identity and ownership/mode validation.
- `profiles/`: shipped TOML files embedded into the executable.
- `examples/`: annotated standalone/inherited profiles; validate with
  `just run --config-dir examples check --strict` when changing the gallery.
  On Linux, review ignored absent-deny warnings as well; strict mode
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
just run policy
just run check                # inspect every profile without launching
just run profile list
just run doctor               # bounded real sandbox startup probe
just run init --project --extends default # creates local policy, never overwrites
just run -p default check --strict
just fmt
just install                  # writes ~/.local/bin/lilbox
just install /custom/prefix    # writes PREFIX/bin/lilbox
```

The installer copies missing shipped profiles and AGENTS.md from the verified
release archive. Existing files get individual overwrite prompts; decline/no
terminal writes inactive versioned alternatives. `--overwrite-profiles` explicitly
replaces regular shipped files. Custom named profiles are preserved.
Default inherits isolated; all three profiles have embedded fallbacks.
`profiles/macos/` and `profiles/linux/` contain the native closed starters. The
binary embeds its native fallbacks; the installer copies the matching platform
files from the archive. CWD/.lilbox/profile.toml overrides default unless -p is set.
`init --project` creates only local policy with no-clobber publication; it does not
install global profiles. Doctor runs only /usr/bin/true in disposable isolated
policy and inspects the requested executable without running it.
Prefer temporary homes/config directories for tests;
do not modify real credentials or user profiles as test fixtures.

## Testing

```sh
just check                    # formatting, Clippy -D warnings, portable tests
just test-sandbox             # current macOS/Linux host enforcement tests
just verify                   # check + test-sandbox
just audit                    # online OSV advisory gate (also required by CI/releases)
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
- Keep ordinary proxy and public CA settings usable in the online default;
  retain secret/injection filtering and explicit drop precedence.
- Do not imply that private-keychain file denial blocks all service-mediated
  keychain access. Document the actual boundary.
- On Linux, ignore absent deny paths at startup and report at most three plus a
  remainder count. Do not create placeholders or freeze their parent directories.
  Names created later may be accessible. Existing denies retain precedence and
  ancestor protection. Apply mounts parent-first and denies last; preserve aliases.
- Linux network deny must block Unix sockets as well as TCP/UDP. A network
  namespace alone is insufficient. Keep syscall ABI checks and descriptor ownership.
- Keep allocation/locking out of the `pre_exec` callback; descriptor enumeration
  relies on the current single-threaded launch design.
- Avoid broad `/System` grants that expose `/System/Volumes/Data`.

## Releases

`Cargo.toml` is the version source. `just bump [VERSION]` prepares it, the root
`Cargo.lock` version, README references, and a dated `CHANGELOG.md` entry. Review
generated notes and keep commit/push manual. Preparation requires a clean tree,
Python 3.9+, authenticated GitHub CLI, and readable origin/GitHub release history.

```sh
just version
just bump                            # next patch; optional explicit version
just verify
just package                         # local native macOS/Linux archive
just package x86_64-apple-darwin       # matching Rust target must be installed
just release                         # explicitly request GitHub publication from pushed main
just release 0.4.0                   # prepare if newer; rerun after manual commit/push
```

`scripts/release.sh` supports ARM64 and x86_64 macOS/Linux (GNU or musl on Linux), builds with locked
dependencies, and writes versioned archives/checksums in `dist/`. Archives include
the executable, README, license, changelog, `install.sh`, profiles, `examples/`, and `docs/`
(including LILBOX-AGENTS.md, installed as config AGENTS.md). Verify the
SHA-256 sidecar from the archive directory and smoke-test the extracted executable.
`just package` is local: it does not sign, notarize, tag, or publish a release.
`just release [VERSION]` prepares a newer optional version and stops; otherwise,
it requires clean, pushed main and dispatches `.github/workflows/release.yml`
with an exact commit/version. Neither command commits or pushes. Version checks
in `scripts/version.py` reject malformed stable versions, downgrades, and versions
at or below any existing stable local/remote tag or GitHub release (including
drafts). The workflow validates again before building and publishing.
Publication waits for all four native targets to
pass tests, optimized enforcement, packaging, and archive smoke tests. Push/PR CI
never publishes. Keep the shared verification workflow and installer target mapping
aligned. Test installer/request scripts offline with `just test-release`; never
publish a release merely to test automation.
Installer tests build and install the native executable into temporary homes and
prefixes, covering profile installation without clobbering existing configs,
terminal output, and failure recovery. The installer asks via /dev/tty before
overwriting each shipped profile/guide separately, preserves versioned alternatives
on decline/no terminal, and accepts --overwrite-profiles for explicit replacement.
It copies profiles after replacing the executable; profile installation failure
must report a partial install and exit nonzero. Keep CI's
`scripts/test_release.py --binary PATH` pointing at its native
target binary, and never use real user configs as installer fixtures.
`just release-installer` separately publishes the local `install.sh` using
authenticated gcloud to `gs://muthanna.com/lilbox/install.sh`, with public-read
access, five-minute caching, shell syntax validation, and byte readback. User-facing
installation links use `https://mo.town/lilbox/install.sh`. Keep deployment tests
offline with stubbed gcloud; CI and binary releases must not upload to this bucket.
`target/` and `dist/` are ignored build outputs.

CI runs portable checks, host enforcement, and packaging on Linux/macOS. Do not claim an unexecuted platform or CI run was verified.
The disposable Ubuntu CI runners load a bwrap-specific AppArmor `userns` grant
and probe namespace startup; do not disable enforcement tests or add host-policy
changes to the installed CLI/installer to work around runner restrictions.

## Website

`www/index.html` is the single-page site at `https://mo.town/lilbox/`, with embedded
CSS/JavaScript, Google Fonts, and a generated icon. See `www/README.md` for the
icon prompt and editing instructions. `just serve` starts a local Python server;
`just test-www` validates asset fingerprints and tests deployment with fake gcloud.
`just upload` publishes only `www/assets/` and `www/index.html` to
`gs://muthanna.com/lilbox/`, assets first, without deleting old objects or touching
`install.sh`. Keep tests offline and user-facing URLs on mo.town. `just check`
includes the website checks. Browser-check desktop/mobile layout and the copy
button when changing the page, and report any unavailable browser verification.

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

## Documentation style

Don't write long verbose paragraphs. Write well structured human readable technical
docs -- use bullets, tables, code blocks, examples, and simple clear short sentences. The
README should be light and high-level, and details can be in the docs/ directory.

## Coding style

Write well commented human-readable code. Structure it for ease of human debugging,
audit, and interpretability. Make sure all constants, functions, structs, interfaces,
etc. are properly commented. Add a small note to the top of each file about the
contents of the file. Ideally, break up long functions into smaller ones, but alternatively
you can  make sure to comment long complex code bits clearly.

Try to keep dependencies at a minimum, use only well-known, highly trusted dependencies.
