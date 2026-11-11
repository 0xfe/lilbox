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
| `just run policy` | Inspect the native sandbox policy. |
| `just fmt` | Format Rust sources. |
| `just install` | Install to `~/.local/bin`. |
| `just install /custom/prefix` | Install to `PREFIX/bin`. |

## Tests

| Command | Coverage |
| --- | --- |
| `just check` | Formatting, Clippy with warnings denied, unit/CLI tests, and offline installer/release-script tests. |
| `just test-sandbox` | Actual enforcement on the current host OS. |
| `just verify` | Both sets of checks. |
| `just test-release` | Build the native executable; offline installer and release-request regressions, including real config initialization; Python 3.9+ required. |

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

Install [GitHub CLI](https://cli.github.com/) and authenticate with `gh auth login`
using an account permitted to read releases and run this repository's Actions
workflows. Version preparation needs Python 3.9+, Git, a clean checkout, and access
to `origin` and the GitHub API.

Prepare the next patch version, or choose an explicit newer version:

```sh
just bump                  # increment the patch component
just bump 0.4.0            # alternatively, choose a version (v0.4.0 also works)
```

[`scripts/version.py`](../scripts/version.py) updates `Cargo.toml`, the root
package in `Cargo.lock`, README version references, and `CHANGELOG.md`. An
`Unreleased` heading becomes a dated release entry; otherwise, commit subjects
since the latest available ancestor release tag seed a new entry. Review these
generated notes before publishing. Dependency versions and old changelog entries
remain unchanged.

Then review, verify, commit, and push manually:

```sh
just verify
git add Cargo.toml Cargo.lock README.md CHANGELOG.md
git commit -m "Bump version for release"
git push origin main
just release
```

`just release [VERSION]` accepts an optional version. If it is newer than the
current version, it prepares the same four files and stops with instructions to
verify, commit, and push manually. Run it again after those steps to publish.
With no argument, or with the current version, it requires a clean checkout on
`main` whose HEAD equals the live `origin/main`, then dispatches the
[release workflow](../.github/workflows/release.yml) for that exact commit and
version. Neither command commits or pushes. Local verification is manual;
publication always waits for the workflow's full verification. The workflow must
first be present on GitHub's default branch (`main`).

Versions must be stable `X.Y.Z` numbers without leading zeros, prerelease suffixes,
or build metadata; a leading `v` is accepted by the Just commands. Each component
must fit Cargo's unsigned 64-bit limit. A bump must increase the current version.
Both preparation and publication require a version greater than every existing
stable local tag, remote tag, and GitHub release, including drafts. An unavailable
remote history blocks the command. The workflow repeats these checks before
building and immediately before publication, so using the Actions UI cannot
bypass version validation.

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
license, changelog, `install.sh`, all three profiles, `examples/`, and `docs/`,
including `docs/LILBOX-AGENTS.md`. Init installs the guide as config `AGENTS.md`.
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

```sh
curl -fsSL https://mo.town/lilbox/install.sh | sh
```

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
rejected. After replacement, the installer invokes that exact executable's `init`
command to install default/permissive/isolated profiles and `AGENTS.md` in
`${XDG_CONFIG_HOME:-$HOME/.config}/lilbox`. Existing shipped files trigger one
confirmation prompt via `/dev/tty`, which works with piped installer input.
Declining or having no usable controlling terminal invokes `init --versioned`:
existing files/symlinks are preserved, and alternatives such as
`default.toml.0.4.0` and `AGENTS.md.0.4.0` are written without clobbering prior
alternatives. These are inactive until reviewed and renamed.

Pinned pre-0.4 releases use their original non-overwriting init without the new
flags or guide; an overwrite request for those versions is rejected before install.

Passing `--overwrite-profiles` to the installer opts into `init --overwrite-profiles`
without prompting. It replaces shipped regular files atomically, refuses symlink
or non-regular destinations, and preserves custom named profiles. Ordinary direct
`lilbox init` still only fills missing files. Init prints the customization-guide
path, as do launches. Initialization requires no backend. Failure reports a
partial installation and a retry command; earlier files may already be written.

Progress messages identify platform selection, release lookup, archive/checksum
downloads, checksum verification, extraction, copying, executable validation,
installation, and profile initialization. The completion screen includes a box
logo, executable/config paths, a quoted PATH
command when needed, quick-start commands, and a reminder of the default profile's
broad access. Color follows stdout's terminal status (so `curl | sh` still uses
color interactively), and is disabled by `NO_COLOR` (even empty) or `TERM=dumb`.
No sudo, shell startup edits, or system package installation is performed. Missing
Linux bubblewrap is reported with installation instructions; kernel restrictions
must still allow actual execution.

`just test-release` tests checksums, download failures, platform selection, upgrades,
initialization failure reporting, terminal colors, and PATH quoting. It also
installs the actual built executable into temporary prefixes and homes to verify
fresh configs, missing-profile creation, custom XDG paths, and preservation of
existing files/symlinks, versioned alternatives, explicit overwrite, and both
interactive prompt responses. CI runs these tests with each native target's binary.
Downloads are fixture-backed; tests never contact GitHub or modify real profiles.

Checksums detect damaged or mismatched downloads; they are distributed from the
same GitHub release and are not an independent signing trust root. macOS binaries
are unsigned and not notarized.

## Publish the hosted installer

```sh
just test-release
just release-installer
```

The standalone installer is served at **https://mo.town/lilbox/install.sh**.
[`scripts/release-installer.sh`](../scripts/release-installer.sh) follows
[Jungle's upload script](https://github.com/0xfe/jungle/blob/main/upload.sh): it
uses authenticated Google Cloud CLI (`gcloud storage cp`) to upload the current
local `install.sh` to `gs://muthanna.com/lilbox/install.sh`. `mo.town` and
`muthanna.com` serve the same bucket; use the `mo.town` URL in installation commands.
Install and authenticate gcloud with an account allowed to write that object.

The script stages a snapshot, checks its shell syntax, uploads only that object
with `publicRead`, `Content-Type: text/plain`, and
`Cache-Control: public,max-age=300,must-revalidate`, then reads it back from GCS
and compares the bytes. A failed upload or mismatch exits unsuccessfully. Public
caches may take up to five minutes to refresh. Avoid concurrent installer uploads.

This command publishes the local installer independently of binary releases; it
does not bump, commit, push, or dispatch GitHub Actions. Ordinary CI and
`just release` do not upload to this bucket. The hosted installer still downloads
binaries/checksums from GitHub Releases. Deployment tests stub gcloud and never
write to the real bucket.

## Website

The landing page at **https://mo.town/lilbox/** lives in `www/index.html`, with
embedded CSS/JavaScript and a generated icon in `www/assets/`. It needs no build
step. Run `just serve` to preview at `http://127.0.0.1:8000/`, `just test-www` for
static/deployment checks, and `just upload` to publish using authenticated gcloud.
The upload target is `gs://muthanna.com/lilbox/`. Fingerprinted images upload
before `index.html`, with one-day image caching and five-minute page caching.
Existing bucket objects, including the separately published installer, are
preserved. See the [website README](https://github.com/0xfe/lilbox/blob/main/www/README.md)
for details and icon provenance.

## Extend lilbox

- Configuration and network capabilities belong in the typed policy.
- Each backend must enforce every requested capability or reject the profile.
- A managed proxy needs lifecycle management and direct-connection restrictions;
  proxy environment variables alone do not enforce routing.
- Update the profile reference and design docs with implementation changes.

See [extension points](design.md#extending-the-system) and the historical
[implementation plan](../plan.md).
