#!/bin/sh
# Download, verify, and atomically replace a native binary, then initialize profiles.
# Usage: curl -fsSL https://mo.town/lilbox/install.sh | sh
# Keep execution inside a function so a truncated download cannot partially install.

# Print one POSIX-shell literal for copyable recovery and PATH instructions.
shell_quote() {
    printf "'"
    printf '%s' "$1" | sed "s/'/'\\\\''/g"
    printf "'"
}

# Report a completed or starting installation step with optional terminal styling.
log_step() {
    printf '  %s→%s %s\n' "$accent" "$reset" "$*"
}

# Install only after input/download validation; init failure reports a partial install.
main() {
    set -eu
    accent='' green='' bold='' reset=''
    if [ -t 1 ] && [ "${NO_COLOR+x}" != x ] && [ "${TERM:-}" != dumb ]; then
        accent=$(printf '\033[1;36m')
        green=$(printf '\033[1;32m')
        bold=$(printf '\033[1m')
        reset=$(printf '\033[0m')
    fi
    # All release discovery and asset downloads must stay under this repository.
    repository=https://github.com/0xfe/lilbox
    version=
    overwrite_profiles=false
    prefix=${HOME:?HOME must be set}/.local
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --prefix|--version)
                [ "$#" -ge 2 ] || { echo "Missing value for $1" >&2; exit 1; }
                case "$1" in
                    --prefix) prefix=$2 ;;
                    --version) version=${2#v} ;;
                esac
                shift 2 ;;
            --overwrite-profiles) overwrite_profiles=true; shift ;;
            -h|--help)
                echo 'Usage: sh install.sh [--prefix /absolute/path] [--version X.Y.Z] [--overwrite-profiles]'
                echo 'Installs PREFIX/bin/lilbox (default: ~/.local/bin), using the latest stable release.'
                echo 'Installs starter profiles and AGENTS.md; asks before overwriting existing files.'
                echo 'Declining (or no terminal) writes versioned alternatives. --overwrite-profiles replaces them.'
                return ;;
            *) echo "Unknown option: $1" >&2; exit 1 ;;
        esac
    done
    case "$prefix" in /*) ;; *) echo 'Installation prefix must be absolute.' >&2; exit 1 ;; esac
    log_step 'Checking platform and installation tools'
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64) arch=x86_64 ;;
        arm64|aarch64) arch=aarch64 ;;
        *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
    esac
    # Linux releases use musl so one archive per CPU works across libc variants.
    case "$os" in
        Darwin) target=$arch-apple-darwin ;;
        Linux) target=$arch-unknown-linux-musl ;;
        *) echo "Unsupported operating system: $os" >&2; exit 1 ;;
    esac
    log_step "Selected platform: $target"
    for tool in curl tar mktemp; do
        command -v "$tool" >/dev/null 2>&1 || { echo "Required tool missing: $tool" >&2; exit 1; }
    done
    if command -v sha256sum >/dev/null 2>&1; then
        checksum_tool=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        checksum_tool=shasum
    else
        echo 'Install sha256sum or shasum to verify downloads.' >&2
        exit 1
    fi
    if [ -z "$version" ]; then
        log_step 'Finding the latest stable release on GitHub'
        release_url=$(curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
            --retry 3 -o /dev/null -w '%{url_effective}' "$repository/releases/latest")
        case "$release_url" in
            "$repository"/releases/tag/v*) version=${release_url##*/v} ;;
            *) echo 'Cannot determine the latest stable release.' >&2; exit 1 ;;
        esac
    fi
    printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || {
        echo 'Version must be a stable release such as 0.3.0.' >&2; exit 1;
    }
    # Older pinned binaries do not understand the new init modes. Keep their
    # original no-clobber init behavior, and reject an unsupported overwrite opt-in.
    legacy_init=false
    case "$version" in 0.[0-3].*) legacy_init=true ;; esac
    if [ "$legacy_init" = true ] && [ "$overwrite_profiles" = true ]; then
        echo '--overwrite-profiles requires lilbox 0.4.0 or newer.' >&2
        exit 1
    fi
    log_step "Preparing lilbox $version"
    name=lilbox-$version-$target
    scratch=$(mktemp -d "${TMPDIR:-/tmp}/lilbox-install.XXXXXXXX")
    pending=
    trap 'rm -rf "$scratch"; if [ -n "$pending" ]; then rm -f "$pending"; fi' EXIT
    trap 'exit 1' HUP INT TERM
    for suffix in .tar.gz .tar.gz.sha256; do
        log_step "Downloading $name$suffix"
        curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 \
            "$repository/releases/download/v$version/$name$suffix" -o "$scratch/$name$suffix"
    done
    log_step 'Verifying the archive SHA-256 checksum'
    # Verify both the digest syntax and the intended archive name before extraction.
    read -r expected checksum_name < "$scratch/$name.tar.gz.sha256"
    if [ "$checksum_name" != "$name.tar.gz" ] || \
        ! printf '%s\n' "$expected" | grep -Eq '^[0-9a-f]{64}$'; then
        echo 'Invalid release checksum file.' >&2
        exit 1
    fi
    if [ "$checksum_tool" = sha256sum ]; then
        actual=$(sha256sum "$scratch/$name.tar.gz")
    else
        actual=$(shasum -a 256 "$scratch/$name.tar.gz")
    fi
    [ "${actual%% *}" = "$expected" ] || { echo 'Checksum verification failed.' >&2; exit 1; }
    log_step 'Extracting the executable'
    tar -xzf "$scratch/$name.tar.gz" -C "$scratch" "$name/lilbox"
    binary=$scratch/$name/lilbox
    if [ ! -f "$binary" ] || [ -L "$binary" ]; then
        echo 'Invalid release executable.' >&2
        exit 1
    fi
    destination=$prefix/bin/lilbox
    if [ -L "$destination" ] || [ -d "$destination" ]; then
        echo "Refusing to replace a symlink or directory: $destination" >&2
        exit 1
    fi
    mkdir -p "$prefix/bin"
    # Stage beside the destination so the final rename stays on one filesystem.
    # Failures before that rename leave an existing installation intact.
    pending=$(mktemp "$prefix/bin/.lilbox.XXXXXXXX")
    log_step "Copying the executable to $prefix/bin"
    cp "$binary" "$pending"
    chmod 755 "$pending"
    # Validate on the destination filesystem; TMPDIR may be mounted noexec.
    log_step 'Checking the executable version'
    [ "$("$pending" --version)" = "lilbox $version" ] || { echo 'Release executable version mismatch.' >&2; exit 1; }
    log_step "Installing $destination"
    mv -f "$pending" "$destination"
    pending=
    # The script itself arrives on stdin in curl | sh. Ask through the controlling
    # terminal instead; without one, preserve files and write versioned alternatives.
    config_directory=${XDG_CONFIG_HOME:-$HOME/.config}/lilbox
    init_option=--versioned
    profile_action='existing files preserved; versioned alternatives written when needed'
    existing=false
    for profile_file in default.toml permissive.toml isolated.toml AGENTS.md; do
        if [ -e "$config_directory/$profile_file" ] || [ -L "$config_directory/$profile_file" ]; then
            existing=true
        fi
    done
    installed_files='default.toml, permissive.toml, isolated.toml, AGENTS.md'
    if [ "$legacy_init" = true ]; then
        init_option=
        installed_files='default.toml and isolated.toml (legacy release)'
        profile_action='existing files preserved by legacy init'
    elif [ "$overwrite_profiles" = true ]; then
        init_option=--overwrite-profiles
    elif [ "$existing" = true ] && ( : <> /dev/tty ) 2>/dev/null; then
        printf 'Replace existing shipped profiles and AGENTS.md in %s? [y/N] ' "$config_directory" > /dev/tty
        if IFS= read -r reply < /dev/tty; then
            case "$reply" in y|Y|yes|YES) init_option=--overwrite-profiles ;; esac
        fi
    fi
    if [ "$init_option" = --overwrite-profiles ]; then
        profile_action='shipped profiles and guide replaced; custom named profiles preserved'
    fi
    # Invoke this exact executable, even if another lilbox is already on PATH.
    set -- init
    if [ -n "$init_option" ]; then
        set -- "$@" "$init_option"
        log_step 'Initializing profiles and agent guide'
    else
        log_step 'Initializing profiles (legacy release)'
    fi
    if ! "$destination" "$@"; then
        printf '\nInstalled the binary at %s, but profile initialization failed.\n' "$destination" >&2
        printf 'Fix the error above, then retry: ' >&2
        shell_quote "$destination" >&2
        printf ' init %s\n' "$init_option" >&2
        exit 1
    fi
    printf '\n%s' "$accent"
    printf '       +-------+\n'
    printf '      /       /|\n'
    printf '     +-------+ |   %slilbox %s%s\n' "$bold" "$version" "$accent"
    printf '     |       | +   %sInstalled and ready%s\n' "$green" "$accent"
    printf '     |       |/\n'
    printf '     +-------+%s\n\n' "$reset"
    printf '  Executable  %s\n' "$destination"
    printf '  Configs     %s\n' "$config_directory"
    printf '              %s\n' "$installed_files"
    printf '              %s.\n\n' "$profile_action"
    case ":${PATH:-}:" in
        *":$prefix/bin:"*) ;;
        *) printf '%sAdd to PATH%s (run now; add to your shell startup file to keep it):\n  export PATH=' "$bold" "$reset"
           shell_quote "$prefix/bin"
           # Expand PATH when the user runs the printed command, not during installation.
           # shellcheck disable=SC2016
           printf ':"$PATH"\n\n' ;;
    esac
    printf '%sGet started%s\n' "$bold" "$reset"
    printf '  lilbox --help             Explore commands\n'
    printf '  lilbox check              Inspect profiles and permissions\n'
    printf '  lilbox shell              Open a sandboxed shell\n'
    printf '  lilbox codex              Run Codex (install your agent separately)\n\n'
    printf 'The default profile allows network access and broad filesystem access,\n'
    printf 'with specific private-path denies. Edit profiles to customize permissions.\n'
    if [ "$os" = Linux ] && [ ! -x /usr/bin/bwrap ]; then
        printf '\n%sLinux prerequisite%s\n' "$bold" "$reset"
        echo 'Linux execution requires /usr/bin/bwrap (bubblewrap 0.6.3+), user namespaces, and seccomp.'
        echo 'On Debian/Ubuntu: sudo apt-get install bubblewrap ca-certificates'
    fi
    printf '\nDocs: https://github.com/0xfe/lilbox#readme\n'
}
main "$@"
