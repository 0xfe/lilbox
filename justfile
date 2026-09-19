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

# Actual Seatbelt enforcement; must run on an unsandboxed macOS host.
test-sandbox:
    #!/bin/sh
    set -eu
    test "$(uname -s)" = Darwin || { echo 'Seatbelt tests require macOS' >&2; exit 1; }
    cargo test --locked --test seatbelt -- --ignored --test-threads=1

# All portable checks.
check: lint test

# Full release verification including macOS enforcement.
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

# Versioned macOS archive + SHA-256 checksum in dist/; optional Rust target.
release target="":
    bash scripts/release.sh "$1"

# Print the package version used for releases.
version:
    @cargo pkgid --offline | sed 's/.*[#@]//'
