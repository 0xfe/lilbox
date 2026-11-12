# Development and releases

Repository conventions: [AGENTS.md](../AGENTS.md). Architecture and module map:
[design.md](design.md).

- [Build and run](#everyday-commands)
- [Test changes](#tests)
- [Publish a release](#publish-a-release)
- [Build a local archive](#build-a-local-archive)
- [Binary installer](#binary-installer)
- [Publish the hosted installer](#publish-the-hosted-installer)
- [Website](#website)

## Prerequisites

| Task | Requirements |
| --- | --- |
| Build and portable checks | Rust/Cargo 1.85+, rustfmt, Clippy, Just, and Python 3.9+ for script tests. |
| macOS enforcement | A host that permits `/usr/bin/sandbox-exec`; developer tools for the C certificate probe. |
| Linux enforcement | `/usr/bin/bwrap` 0.6.3+, permitted user namespaces and seccomp, Python 3, and public CA certificates. |
| Package an archive | Python 3.9+ and the target's Rust toolchain/linker. |
| Prepare or publish a release | Git, authenticated GitHub CLI, a clean checkout, and access to `origin` and the GitHub API. |
| Upload installer or website | Authenticated Google Cloud CLI with write access to the destination bucket. |

Use current stable Rust without silently raising the minimum in `Cargo.toml`.

## Everyday commands

| Command | Purpose |
| --- | --- |
| `just` | List recipes. |
| `just build` | Build the debug executable with locked dependencies. |
| `just run --help` | Show CLI help from the debug build. |
| `just run policy` | Inspect the native sandbox policy. |
| `just fmt` | Format Rust sources. |
| `just install` | Build and install the executable to `~/.local/bin`. |
| `just install /custom/prefix` | Install the executable to `/custom/prefix/bin`. |

The [binary installer](#binary-installer) also installs profiles and the agent guide.

## Tests

| Command | Coverage |
| --- | --- |
| `just check` | Formatting, Clippy with warnings denied, unit/CLI tests, installer/release tests, and website checks. |
| `just test-sandbox` | Actual sandbox enforcement on the current host OS. |
| `just verify` | `check` plus `test-sandbox`. |
| `just test-release` | Build the native executable and test installation/release scripts with offline fixtures. |
| `just test-www` | Validate website assets and test uploads with fake gcloud. |
| `just audit` | Query OSV for dependency advisories; requires network access. |

### Choose the checks

| Change | Verification |
| --- | --- |
| Permissions, paths, environment, launch, or networking | Add meaningful regression coverage and run `just verify`. |
| Release-specific launch behavior | Also run optimized enforcement tests below. |
| Documentation only | Check links, commands, source consistency, and `git diff --check`. |
| Archive contents | Inspect and [verify the archive](#verify-the-archive). |

```sh
# Optimized enforcement on macOS
cargo test --release --locked --test seatbelt -- --ignored --test-threads=1

# Optimized enforcement on Linux
cargo test --release --locked --test bubblewrap -- --ignored --test-threads=1
```

### Host enforcement

- Ordinary `cargo test` skips enforcement tests; run them explicitly.
- Fixtures use temporary homes and fake credentials. Never use real user profiles.
- Coverage includes filesystem denies, symlinks, ancestor renames, networking,
  descendants, descriptors, signals, and terminal behavior.
- The macOS [certificate probe](../tests/fixtures/trust_roots.c) compares public
  trust settings without network access or private keys.

| Host restriction | What to report |
| --- | --- |
| Nested Seatbelt returns `sandbox_apply: Operation not permitted` | Enforcement could not run in that environment. |
| Linux blocks namespace or seccomp setup | Enforcement could not run under that host policy. |
| Only one platform tested locally | Other platforms and remote CI remain unverified. |

Do not skip failures or report blocked tests as successful enforcement.

## Diagnostic regression coverage

| Suite | Checks |
| --- | --- |
| Portable CLI tests | Local/global selection, spelling suggestions, project creation, overwrite/symlink protection, and environment repair instructions. |
| Shared doctor test in both native suites | Actual startup, missing-executable remedies, trust failures, and launch output without the help-only coding-agent prompt. |

Doctor fixtures use disposable homes and configuration directories.

## Continuous integration

Pushes and pull requests use the shared
[verification workflow](../.github/workflows/verify.yml).

| Platform | Native runners |
| --- | --- |
| macOS | `macos-15` for ARM64; `macos-15-intel` for x86_64. |
| Linux | `ubuntu-24.04-arm` for ARM64; `ubuntu-24.04` for x86_64. |

Each platform runs portable tests, installer tests, host enforcement, packaging,
and archive smoke tests. Linux also checks static linking. A separate job checks
workflows, shell scripts, website deployment, and dependency advisories.

CI stores temporary Actions artifacts; it does not publish releases.

**Linux runner setup:**

- Grant only `/usr/bin/bwrap` AppArmor `userns` permission; keep Ubuntu's global restriction.
- Probe user/PID/network namespaces before tests; run enforcement as the unprivileged user.
- Apply this only to disposable CI runners. lilbox and its installer never change host AppArmor policy.

## Publish a release

### Prepare

Start with a clean checkout and authenticate GitHub CLI:

```sh
gh auth login
just version
just bump                 # prepare the next patch version
# Or: just bump X.Y.Z     # choose a newer stable version
```

The account must read releases and dispatch workflows. `release.yml` must exist on `main`.

[`scripts/version.py`](../scripts/version.py) prepares these files:

| File | Change |
| --- | --- |
| `Cargo.toml` | Package version; this is the version source. |
| `Cargo.lock` | Root package version only. |
| `README.md` | Version heading, pinned installer example, and optional startup-banner version. |
| `CHANGELOG.md` | Dated release entry. |

Review the generated notes: they come from `Unreleased`, or commit subjects since
the latest ancestor release tag. Dependencies and historical entries stay unchanged.
README's version heading and pinned installer example are required; the banner is optional.

### Verify and publish

```sh
just verify
just audit
git diff --check
git add Cargo.toml Cargo.lock README.md CHANGELOG.md
git commit -m "Bump version for release"
git push origin main
just release
```

Neither `just bump` nor `just release` commits or pushes for you.

| Command | Behavior |
| --- | --- |
| `just release` | Publish the current version from clean, pushed `main`. |
| `just release CURRENT_VERSION` | Same as above. |
| `just release NEWER_VERSION` | Prepare the version and stop. Verify, commit, push, then rerun to publish. |

Local HEAD must match live `origin/main`. [release.yml](../.github/workflows/release.yml)
receives that exact commit and version.

### Version rules

- Use stable `X.Y.Z`; no leading zeros, prerelease suffixes, or build metadata.
- Just commands accept a leading `v`. Each component must fit an unsigned 64-bit integer.
- A bump must increase the current package version.
- The proposed release must exceed every stable local tag, remote tag, and GitHub
  release, including drafts.
- If remote history is unavailable, preparation/publication stops.
- Validation runs again before building and immediately before publication.

### What the workflow does

1. Validate the requested main commit and version.
2. Run all four platforms' checks, including optimized host enforcement.
3. Verify the complete archive and SHA-256 file set.
4. Create `vVERSION` at the tested commit and publish the release as latest.

Release assets: four native archives, four SHA-256 files, and `install.sh`.

- Publish through `just release` or Actions `workflow_dispatch` with the full main SHA
  and version. Pushes, tags, and pull requests never publish.
- If main moves before dispatch, synchronize and request again.
- Builds are read-only. Publication uses `contents: write` and GitHub's built-in token;
  repository rules must permit tag creation.
- Existing tags/releases are never overwritten. A failed upload may leave a draft;
  inspect it before retrying.

Monitor the run:

```sh
gh run list --workflow release.yml
gh run watch RUN_ID
```

## Build a local archive

```sh
just package                         # native target; local output only
just package x86_64-apple-darwin       # explicit target
```

Cross-compilation needs the matching linker and Rust target:

```sh
rustup target add x86_64-apple-darwin
```

Supported targets in [scripts/release.sh](../scripts/release.sh):

| OS | Published targets | Additional local targets |
| --- | --- | --- |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | None. |
| Linux | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | Corresponding `unknown-linux-gnu` targets. |

- Published Linux binaries use static musl linking for glibc and musl distributions.
- Bubblewrap remains a system dependency; it is not bundled.
- `just package` does not tag, sign, notarize, or publish anything.

## Verify the archive

Output files in `dist/`:

```text
lilbox-VERSION-TARGET.tar.gz
lilbox-VERSION-TARGET.tar.gz.sha256
```

Replace the placeholders with your archive's version and target. From `dist/`:

```sh
# macOS
shasum -a 256 -c lilbox-VERSION-TARGET.tar.gz.sha256

# Linux
sha256sum -c lilbox-VERSION-TARGET.tar.gz.sha256
```

From the repository root, on a matching host:

```sh
python3 scripts/verify-archive.py dist/ARCHIVE.tar.gz
```

The check uses disposable homes/configs and local downloads. It verifies checksums,
contents, version flags, installed profile/guide bytes, and real sandbox launches
under `default` and `isolated`.

Archives contain:

- `lilbox`, `install.sh`, README, license, and changelog.
- Both platforms' default/isolated profiles and shared permissive profile.
- `examples/` and `docs/`, including `LILBOX-AGENTS.md` installed as config `AGENTS.md`.

| Packaging property | Behavior |
| --- | --- |
| Dependencies | Builds use `Cargo.lock`. |
| Metadata | File order, ownership, modes, timestamps, and gzip headers are normalized. |
| Reproducibility | Identical input bytes produce identical archives. Compiler/SDK changes can change the binary. |
| Signing | Archives are unsigned; macOS binaries are not notarized. |

## Binary installer

```sh
curl -fsSL https://mo.town/lilbox/install.sh | sh

# Choose a prefix
curl -fsSL https://mo.town/lilbox/install.sh | sh -s -- --prefix /custom/prefix
```

[`install.sh`](../install.sh) runs with POSIX `sh`, including when piped from curl.
It needs curl, tar, mktemp, and either sha256sum or shasum.

| Setting | Default or option |
| --- | --- |
| Release | Latest stable; `--version X.Y.Z` pins one. A leading `v` is accepted. |
| Executable | `~/.local/bin/lilbox`; `--prefix /absolute/path` selects `PREFIX/bin`. |
| Configuration | `${XDG_CONFIG_HOME:-$HOME/.config}/lilbox`. |
| Platforms | The four published targets above; other OS/architecture combinations fail. |
| Linux host | Requires bubblewrap and permitted namespaces/seccomp. See [backend requirements](design.md#linux-bubblewrap-backend). |

### Replacement and recovery

The installer:

1. Downloads the archive and checksum over HTTPS.
2. Verifies the checksum and extracts fixed member paths only.
3. Rejects symlinks, non-regular source files, and missing required bundled files.
4. Checks the executable's embedded version.
5. Replaces the executable atomically using a temporary file beside it.

- Download/validation failure: the existing executable stays intact.
- Profile-copy failure after replacement: report a partial install and exit nonzero.
  Fix the problem and rerun; earlier files may already be installed.

### Profile handling

The installer copies configuration directly from the verified archive.

| Destination state | Action |
| --- | --- |
| Missing shipped file | Install it. |
| Existing shipped file | Ask separately through `/dev/tty`, including with piped input. |
| Declined prompt or no terminal | Preserve it; write inactive `NAME.toml.VERSION` or `AGENTS.md.VERSION`. |
| Existing versioned alternative | Preserve it. |
| `--overwrite-profiles` or approved prompt | Replace regular shipped files; reject symlinks and non-regular destinations. |
| Custom named profile | Preserve it. |

- New config directories use mode 0700; staged files use mode 0600.
- New files use a hard link without clobbering; approved replacements use a rename.
- Select native files from `profiles/macos/` or `profiles/linux/`; legacy `profiles/` layouts work too.
- Copy only bundled files. Without a legacy agent guide, link to that version's profile docs.

### Output and limits

- Progress shows download, verification, extraction, and installation stages.
- Completion shows the box logo, executable/config paths, PATH reminder, and agent prompt.
- Color requires a stdout terminal and is disabled by `NO_COLOR` or `TERM=dumb`.
- No sudo, shell startup edits, or system package installation.
- Release checksums detect corruption; they are not an independent signing trust root.

### Installer tests

`just test-release` covers:

- Checksums, download failures, platform selection, upgrades, and partial failures.
- Fresh configs, missing files, custom XDG paths, and private file permissions.
- Existing files/symlinks, versioned alternatives, overwrite flags, and both prompt responses.
- Terminal colors and completion text.

Tests use each target's actual binary, temporary homes/prefixes, and fixture-backed
downloads. They never contact GitHub or modify real user configs.

## Publish the hosted installer

```sh
just test-release
just release-installer
```

| Property | Value |
| --- | --- |
| Script | [scripts/release-installer.sh](../scripts/release-installer.sh). |
| Public URL | `https://mo.town/lilbox/install.sh`. |
| Storage object | `gs://muthanna.com/lilbox/install.sh`. |
| Access and type | `publicRead`, `Content-Type: text/plain`. |
| Cache | `public,max-age=300,must-revalidate`. |

- Snapshot the local script, check shell syntax, upload, and compare GCS readback bytes.
- Fail on upload/readback errors. Allow five minutes for public caches to refresh.
- Avoid concurrent uploads. Tests use fake gcloud.
- `mo.town` and `muthanna.com` share the bucket; public links use mo.town.

This is separate from bump/commit/push and GitHub releases. CI and `just release`
never upload here. The hosted installer fetches binaries/checksums from GitHub Releases.

## Website

The static page lives in [www/index.html](../www/index.html), with embedded CSS/JS
and images in `www/assets/`. No build step is needed.

```sh
just serve       # http://127.0.0.1:8000/
just test-www    # offline asset and deployment checks
just upload      # publish with authenticated gcloud
```

Public URL: `https://mo.town/lilbox/`. Storage: `gs://muthanna.com/lilbox/`.

- Fingerprinted images upload first and cache for one day.
- `index.html` uploads last and caches for five minutes.
- Old assets and the separately deployed installer are preserved.
- See [www/README.md](../www/README.md) for editing, deployment, and icon provenance.

## Extend lilbox

- Represent configuration/network capabilities in the typed policy.
- Each backend must enforce a requested capability or reject it.
- A managed proxy needs lifecycle management and direct-connection restrictions;
  proxy environment variables alone do not enforce routing.
- Update the profile reference and design docs with implementation changes.

See [extension points](design.md#extending-the-system) and the historical
[implementation plan](../plan.md).

## Dependency advisory gate

```sh
just audit
```

- OSV checks cover every registry package/version in `Cargo.lock`, including
  dependencies for other platforms.
- CI/releases fail on active advisories, API errors, or incomplete responses.
  Withdrawn advisories do not block builds.
- [.github/dependabot.yml](../.github/dependabot.yml) schedules Cargo and Actions updates.
  Repository administrators manage GitHub alerts and security-update settings separately.
- Offline fixtures cover active/withdrawn advisories and API errors.

No exceptions are configured. Any exception needs review, an advisory ID,
unreachable-API evidence, an owner, and an expiry within 30 days. Implement expiry
enforcement first; never disable the gate or use an unbounded ignore list.
