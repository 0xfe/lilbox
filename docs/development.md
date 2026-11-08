# Development and releases

See [AGENTS.md](../AGENTS.md) for repository conventions and security constraints,
and [design.md](design.md) for module responsibilities.

## Prerequisites

| Work | Requirements |
| --- | --- |
| Build and portable checks | Rust/Cargo 1.85+, rustfmt, Clippy, and Just. Use current stable without raising the declared minimum. |
| macOS enforcement | A host permitting `/usr/bin/sandbox-exec`; developer tools for the C certificate probe. |
| Linux enforcement | `/usr/bin/bwrap` 0.6.3+, permitted user namespaces and seccomp, Python 3, and public CA certificates. |
| Release packaging | Python 3.9+ and the target's Rust toolchain/linker. |

## Everyday commands

| Command | Purpose |
| --- | --- |
| `just` | List recipes. |
| `just build` | Build with locked dependencies. |
| `just run --help` | Run the debug executable. |
| `just run --print-policy` | Inspect the native sandbox policy. |
| `just fmt` | Format Rust sources. |
| `just install` | Install to `~/.local/bin`. |
| `just install /custom/prefix` | Install to `PREFIX/bin`. |

## Tests

| Command | Coverage |
| --- | --- |
| `just check` | Formatting, Clippy with warnings denied, unit/CLI tests, and offline installer/release-script tests. |
| `just test-sandbox` | Actual enforcement on the current host OS. |
| `just verify` | Both sets of checks. |
| `just test-release` | Offline installer and release-request regressions; Python 3.9+ required. |

Host tests use temporary homes and fake credentials. They cover filesystem denies,
symlinks, ancestor renames, networking, descendants, descriptors, signals, and
terminal behavior. The macOS [certificate probe](../tests/fixtures/trust_roots.c)
compares public trust settings without network access or private keys.

- Ordinary `cargo test` intentionally ignores host enforcement tests.
- Run `just verify` for permission, path, environment, launch, or network changes.
- Report host restrictions as limitations, not successful enforcement. Nested
  Seatbelt can fail with `sandbox_apply: Operation not permitted`; Linux hosts
  may block namespace or seccomp setup.
- For documentation-only changes, verify links, commands, source consistency,
  and `git diff --check`. Inspect the archive if its contents change.

For release-specific launch changes, also test the optimized executable:

```sh
# macOS
cargo test --release --locked --test seatbelt -- --ignored --test-threads=1
# Linux
cargo test --release --locked --test bubblewrap -- --ignored --test-threads=1
```

CI runs on every push and pull request. The shared
[verification workflow](../.github/workflows/verify.yml) uses native ARM64 and
x86_64 runners on both macOS and Linux. Each job checks formatting, Clippy, unit
and CLI tests, installer/release-script tests, real sandbox enforcement, and the
packaged executable. CI uploads temporary Actions artifacts; it never creates
GitHub releases. A successful local run does not verify other platforms or remote CI.

The macOS jobs use `macos-15` (ARM64) and `macos-15-intel`. On the disposable
Ubuntu 24.04 runners, CI loads an AppArmor profile granting `/usr/bin/bwrap` the
`userns` permission, then probes user/PID/network namespace startup before tests.
Ubuntu's global unprivileged-user-namespace restriction stays enabled. This is
runner provisioning only: lilbox and its installer never change host AppArmor
policy. All enforcement tests still run as the unprivileged runner user.

## Publish a release

1. Update `Cargo.toml`, refresh `Cargo.lock`, and update `CHANGELOG.md` and README.
2. Run `just verify`, then commit and push the changes to `main`.
3. Install [GitHub CLI](https://cli.github.com/) and authenticate with `gh auth login`
   using an account permitted to run this repository's Actions workflows.
4. Run:

```sh
just release
```

[`scripts/request-release.sh`](../scripts/request-release.sh) requires a clean
checkout on `main`, checks that HEAD equals the live `origin/main`, rejects an
existing version tag, and dispatches the
[release workflow](../.github/workflows/release.yml) with the exact commit and
version. It does not commit or push your changes. The workflow must first be
present on GitHub's default branch (`main`).

Only this explicit `workflow_dispatch` publishes: pushes, tags, and pull requests
do not request releases. The Actions UI can also dispatch the same workflow with
the full main commit SHA and version. If main moves before dispatch, validation
fails; synchronize and request again.

The release workflow repeats the full four-platform verification and adds
optimized host enforcement tests. After every job succeeds, it verifies the
complete archive/checksum set and creates `vVERSION` at the tested commit,
attaches all four archives, their SHA-256 files, and `install.sh`, and publishes
the GitHub release as latest. Builds have read-only repository permissions; only
the final publication job has `contents: write`, using GitHub's built-in token.
No personal publishing token is needed. Repository rules must permit the workflow
token to create release tags. A failed upload can leave a draft; inspect it before
retrying. An existing tag/release is never overwritten automatically.

```sh
gh run list --workflow release.yml
gh run watch RUN_ID
```

## Build a local archive

```sh
just version
just package                         # native target; no publication
just package x86_64-apple-darwin       # explicit target
```

[`scripts/release.sh`](../scripts/release.sh) supports these targets:

| OS | Published targets | Additional local targets |
| --- | --- | --- |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | None |
| Linux | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | The corresponding `unknown-linux-gnu` targets |

The published Linux binaries use musl static linking so they work on glibc and
musl distributions without requiring the build runner's glibc version. Bubblewrap
remains a system runtime dependency and is not bundled. Install the matching Rust
target and linker before local cross-compilation; for example,
`rustup target add x86_64-apple-darwin` installs that Rust target.

## Verify the archive

Artifacts are written to `dist/`:

- `lilbox-VERSION-TARGET.tar.gz`
- `lilbox-VERSION-TARGET.tar.gz.sha256`

From that directory, verify the checksum:

```sh
# macOS
shasum -a 256 -c lilbox-VERSION-TARGET.tar.gz.sha256
# Linux
sha256sum -c lilbox-VERSION-TARGET.tar.gz.sha256
```

Then extract and smoke-test the executable on a matching host, using temporary
home/config directories. Confirm the archive includes the executable, README,
license, changelog, `install.sh`, `profiles/`, `examples/`, and `docs/`.
The automated host check is `python3 scripts/verify-archive.py dist/ARCHIVE.tar.gz`;
it verifies checksums, contents, version flags, and launches with both profiles.

| Packaging property | Behavior |
| --- | --- |
| Dependencies | Build uses `Cargo.lock`. |
| Archive metadata | File order, ownership, modes, timestamps, and gzip headers are normalized. |
| Reproducibility | Identical input bytes produce identical archives; compiler/SDK changes can change the binary. |
| Signing | Archives are unsigned and not notarized. |
| Publishing | `just package` is local only; `just release` explicitly requests GitHub publication. |

## Binary installer

[`install.sh`](../install.sh) defaults to the latest stable GitHub release and
`~/.local/bin`. `--version X.Y.Z` (optionally prefixed with `v`) pins a release;
`--prefix /absolute/path` installs to `PREFIX/bin`. The script works with POSIX
`sh`, including when piped from curl, and needs curl, tar, mktemp, and either
sha256sum or shasum. It supports the four published targets above and refuses
other operating systems/architectures.

Downloads use HTTPS. Before replacement, the installer checks the archive hash,
extracts only the executable, and verifies its embedded version. A temporary file
in the destination directory permits atomic replacement; failed downloads/checks
leave an existing installation intact. Destination symlinks/directories are
rejected. No sudo, profile initialization, shell startup edits, or system package
installation is performed. Missing Linux bubblewrap is reported with installation
instructions; kernel restrictions must still allow actual execution.

Checksums detect damaged or mismatched downloads; they are distributed from the
same GitHub release and are not an independent signing trust root. macOS binaries
are unsigned and not notarized.

## Extend lilbox

- Configuration and network capabilities belong in the typed policy.
- Each backend must enforce every requested capability or reject the profile.
- A managed proxy needs lifecycle management and direct-connection restrictions;
  proxy environment variables alone do not enforce routing.
- Update the profile reference and design docs with implementation changes.

See [extension points](design.md#extending-the-system) and the historical
[implementation plan](../plan.md).
