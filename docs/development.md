# Development and releases

See [AGENTS.md](../AGENTS.md) for repository conventions and security constraints,
and [design.md](design.md) for module responsibilities.

## Prerequisites

| Work | Requirements |
| --- | --- |
| Build and portable checks | Rust/Cargo 1.85+, rustfmt, Clippy, and Just. Use current stable without raising the declared minimum. |
| macOS enforcement | A host permitting `/usr/bin/sandbox-exec`; developer tools for the C certificate probe. |
| Linux enforcement | `/usr/bin/bwrap` 0.6.3+, permitted user namespaces and seccomp, Python 3, and public CA certificates. |
| Release packaging | Python 3 and the target's Rust toolchain/linker. |

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
| `just check` | Formatting, Clippy with warnings denied, unit tests, and CLI tests. |
| `just test-sandbox` | Actual enforcement on the current host OS. |
| `just verify` | Both sets of checks. |

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

CI is configured to run portable checks, host enforcement, and packaging on Linux
and macOS. A successful local run does not verify other platforms or remote CI.

## Build a release

1. Update the version in `Cargo.toml` and the release entry in `CHANGELOG.md`.
2. Refresh `Cargo.lock` and keep README version references consistent.
3. Run `just verify` and any required optimized enforcement tests.
4. Build the archive:

```sh
just version
just release                         # native target
just release x86_64-apple-darwin       # explicit target
```

[`scripts/release.sh`](../scripts/release.sh) supports these targets:

| OS | Architectures | Target families |
| --- | --- | --- |
| macOS | ARM64, x86_64 | `apple-darwin` |
| Linux | ARM64, x86_64 | `unknown-linux-gnu`, `unknown-linux-musl` |

Install the matching Rust target and linker before cross-compiling; for example,
`rustup target add x86_64-apple-darwin` installs the Rust target. Bubblewrap remains
a system runtime dependency and is not bundled.

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
license, changelog, `profiles/`, `examples/`, and `docs/`.

| Packaging property | Behavior |
| --- | --- |
| Dependencies | Build uses `Cargo.lock`. |
| Archive metadata | File order, ownership, modes, timestamps, and gzip headers are normalized. |
| Reproducibility | Identical input bytes produce identical archives; compiler/SDK changes can change the binary. |
| Signing | Archives are unsigned and not notarized. |
| Publishing | `just release` does not commit, tag, publish, or upload. |

## Extend lilbox

- Configuration and network capabilities belong in the typed policy.
- Each backend must enforce every requested capability or reject the profile.
- A managed proxy needs lifecycle management and direct-connection restrictions;
  proxy environment variables alone do not enforce routing.
- Update the profile reference and design docs with implementation changes.

See [extension points](design.md#extending-the-system) and the historical
[implementation plan](../plan.md).
