#!/bin/sh
# Download, verify, and atomically replace a native binary, then install profiles.
# Usage: curl -fsSL https://mo.town/lilbox/install.sh | sh
# Keep execution inside a function so a truncated download cannot partially install.

# Print one POSIX-shell literal for copyable recovery instructions.
shell_quote() {
    printf "'"
    printf '%s' "$1" | sed "s/'/'\\\\''/g"
    printf "'"
}

# Report a completed or starting installation step with optional terminal styling.
log_step() {
    printf '  %s→%s %s\n' "$accent" "$reset" "$*"
}

# Check only exact, fixed member names in the verified release archive.
archive_has() {
    grep -Fxq "$name/$1" "$scratch/members"
}

# Extract a selected regular file into private staging; never unpack the whole archive.
extract_file() {
    tar -xzf "$scratch/$name.tar.gz" -C "$scratch" "$name/$1" || return 1
    if [ ! -f "$scratch/$name/$1" ] || [ -L "$scratch/$name/$1" ]; then
        printf 'Invalid release file: %s\n' "$1" >&2
        return 1
    fi
}

# Stage the native profiles and guide before replacing the executable. Older
# archives have shared profiles at their root and may predate the agent guide.
stage_profiles() {
    tar -tzf "$scratch/$name.tar.gz" > "$scratch/members" || return 1
    mkdir "$scratch/profiles" || return 1
    case "$os" in Darwin) platform=macos ;; Linux) platform=linux ;; esac
    if archive_has "profiles/$platform/default.toml"; then
        profile_root=profiles/$platform
        shipped_files='default.toml permissive.toml isolated.toml AGENTS.md'
    else
        profile_root=profiles
        shipped_files='default.toml isolated.toml'
        if archive_has profiles/permissive.toml; then shipped_files="$shipped_files permissive.toml"; fi
        if archive_has docs/LILBOX-AGENTS.md; then shipped_files="$shipped_files AGENTS.md"; fi
    fi
    for profile_file in $shipped_files; do
        case "$profile_file" in
            AGENTS.md) member=docs/LILBOX-AGENTS.md ;;
            permissive.toml) member=profiles/permissive.toml ;;
            *) member=$profile_root/$profile_file ;;
        esac
        extract_file "$member" || return 1
        cp "$scratch/$name/$member" "$scratch/profiles/$profile_file" || return 1
    done
}

# Publish complete mode-0600 files on the destination filesystem. Hard-linking
# reserves a new name without clobbering it; explicit replacement uses rename.
write_profile() {
    profile_target=$config_directory/$profile_file
    replace=$overwrite_profiles
    if [ -e "$profile_target" ] || [ -L "$profile_target" ]; then
        if [ "$replace" = false ] && ( : <> /dev/tty ) 2>/dev/null; then
            printf 'Replace %s? [y/N] ' "$profile_target" > /dev/tty
            if IFS= read -r reply < /dev/tty; then
                case "$reply" in y|Y|yes|YES) replace=true ;; esac
            fi
        fi
        if [ "$replace" = false ]; then
            profile_target=$profile_target.$version
        fi
    fi
    if [ "$replace" = true ]; then
        if [ -L "$profile_target" ] || { [ -e "$profile_target" ] && [ ! -f "$profile_target" ]; }; then
            printf 'Refusing to overwrite non-regular file: %s\n' "$profile_target" >&2
            return 1
        fi
    elif [ -e "$profile_target" ] || [ -L "$profile_target" ]; then
        printf 'Preserved %s\n' "$profile_target"
        return 0
    fi
    profile_pending=$(mktemp "$config_directory/.lilbox-profile.XXXXXXXX") || return 1
    cp "$scratch/profiles/$profile_file" "$profile_pending" || return 1
    if [ "$replace" = true ]; then
        mv -f "$profile_pending" "$profile_target" || return 1
    elif ! ln "$profile_pending" "$profile_target" 2> "$scratch/profile-error"; then
        if [ -e "$profile_target" ] || [ -L "$profile_target" ]; then
            rm -f "$profile_pending"
            profile_pending=
            printf 'Preserved %s\n' "$profile_target"
            return 0
        fi
        cat "$scratch/profile-error" >&2
        return 1
    fi
    rm -f "$profile_pending"
    profile_pending=
    printf 'Wrote %s\n' "$profile_target"
}

# Profile failures after binary replacement are reported as partial installations.
install_profiles() {
    (umask 077; mkdir -p "$config_directory") || return 1
    for profile_file in $shipped_files; do
        write_profile || return 1
    done
}

# Install only after input/download validation; file failures never announce success.
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
    config_directory=${XDG_CONFIG_HOME:-$HOME/.config}/lilbox
    case "$config_directory" in /*) ;; *) echo 'Configuration directory must be absolute.' >&2; exit 1 ;; esac
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
    log_step "Preparing lilbox $version"
    name=lilbox-$version-$target
    scratch=$(mktemp -d "${TMPDIR:-/tmp}/lilbox-install.XXXXXXXX")
    pending=
    profile_pending=
    trap 'rm -rf "$scratch"; if [ -n "$pending" ]; then rm -f "$pending"; fi; if [ -n "$profile_pending" ]; then rm -f "$profile_pending"; fi' EXIT
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
    log_step 'Checking bundled profiles and agent guide'
    stage_profiles || exit 1
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
    # Ask through the controlling terminal: stdin may contain this script.
    log_step 'Installing profiles and agent guide'
    if ! install_profiles; then
        printf '\nInstalled the binary at %s, but profile installation failed.\n' "$destination" >&2
        printf 'Fix the error, then rerun the installer with --prefix ' >&2
        shell_quote "$prefix" >&2
        printf ' --version %s\n' "$version" >&2
        exit 1
    fi
    agent_guide=$config_directory/AGENTS.md
    if [ ! -f "$agent_guide" ]; then
        agent_guide=$repository/blob/v$version/docs/profiles.md
    fi
    printf '\n%s' "$accent"
    printf '       +-------+\n'
    printf '      /       /|\n'
    printf '     +-------+ |   %slilbox %s%s\n' "$bold" "$version" "$accent"
    printf '     |       | +   %sInstalled and ready%s\n' "$green" "$accent"
    printf '     |       |/\n'
    printf '     +-------+%s\n\n' "$reset"
    printf 'Executable:  %s\n' "$destination"
    printf 'Configs:     %s\n\n' "$config_directory"
    printf 'Make sure to add %s/bin to your PATH.\n\n' "$prefix"
    printf 'To get started, prompt your coding agent with:\n\n'
    printf 'Read %s and create a new profile for [..this project..] and show me what it does and how to use it.\n' "$agent_guide"
}
main "$@"
