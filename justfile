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
check: lint test

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

# Versioned macOS/Linux archive + SHA-256 checksum in dist/; optional Rust target.
release target="":
    bash scripts/release.sh "$1"

# Print the package version used for releases.
version:
    @cargo pkgid --offline | sed 's/.*[#@]//'
