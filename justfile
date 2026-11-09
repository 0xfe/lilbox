set positional-arguments

default:
    @just --list

# Build the development executable.
build:
    cargo build --locked

# Format Rust sources.
fmt:
    cargo fmt --all

# Formatting and warning-free lint checks.
lint:
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets --all-features -- -D warnings

# Portable unit and CLI tests (no host sandbox needed).
test:
    cargo test --locked

# Actual host enforcement; requires sandbox-exec or Linux bubblewrap/namespaces.
test-sandbox:
    #!/bin/sh
    set -eu
    case "$(uname -s)" in
        Darwin) suite=seatbelt ;;
        Linux) suite=bubblewrap ;;
        *) echo 'Sandbox tests require macOS or Linux' >&2; exit 1 ;;
    esac
    cargo test --locked --test "$suite" -- --ignored --test-threads=1

# All portable checks.
check: lint test test-release

# Offline installer and release automation regression tests.
test-release:
    cargo build --locked
    bash -n scripts/release.sh
    bash -n scripts/request-release.sh
    bash -n scripts/release-installer.sh
    sh -n install.sh
    python3 scripts/test_release.py --binary target/debug/lilbox

# Full verification including the current platform's enforcement.
verify: check test-sandbox

# Install in ~/.local/bin, or choose a different prefix: just install /usr/local.
install prefix=(env_var("HOME") + "/.local"):
    #!/bin/sh
    set -eu
    cargo build --release --locked --target-dir target
    install -d "$1/bin"
    install -m 755 target/release/lilbox "$1/bin/lilbox"
    echo "Installed $1/bin/lilbox"

# Run the debug build: just run --print-policy.
run *args:
    cargo run --locked -- "$@"

# Local archive + SHA-256 checksum in dist/; does not publish.
package target="":
    bash scripts/release.sh "$1"

# Prepare the next patch version, or an explicit newer version; no commit or push.
bump version="":
    python3 scripts/version.py bump "$1"

# Prepare an optional version, or publish the current clean, pushed main version.
release version="":
    bash scripts/request-release.sh "$1"

# Publish the local installer to https://mo.town/lilbox/install.sh using gcloud.
release-installer:
    bash scripts/release-installer.sh

# Print the package version used for releases.
version:
    @cargo pkgid --offline | sed 's/.*[#@]//'
