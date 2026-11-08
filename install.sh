#!/bin/sh
# Usage: curl -fsSL https://raw.githubusercontent.com/0xfe/lilbox/main/install.sh | sh
# Keep execution inside a function so a truncated download cannot partially install.
main() {
    set -eu
    repository=https://github.com/0xfe/lilbox
    version=
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
            -h|--help)
                echo 'Usage: sh install.sh [--prefix /absolute/path] [--version X.Y.Z]'
                echo 'Installs PREFIX/bin/lilbox (default: ~/.local/bin), using the latest stable release.'
                return ;;
            *) echo "Unknown option: $1" >&2; exit 1 ;;
        esac
    done
    case "$prefix" in /*) ;; *) echo 'Installation prefix must be absolute.' >&2; exit 1 ;; esac
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64) arch=x86_64 ;;
        arm64|aarch64) arch=aarch64 ;;
        *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
    esac
    case "$os" in
        Darwin) target=$arch-apple-darwin ;;
        Linux) target=$arch-unknown-linux-musl ;;
        *) echo "Unsupported operating system: $os" >&2; exit 1 ;;
    esac
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
    name=lilbox-$version-$target
    scratch=$(mktemp -d "${TMPDIR:-/tmp}/lilbox-install.XXXXXXXX")
    pending=
    trap 'rm -rf "$scratch"; if [ -n "$pending" ]; then rm -f "$pending"; fi' EXIT
    trap 'exit 1' HUP INT TERM
    for suffix in .tar.gz .tar.gz.sha256; do
        curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 \
            "$repository/releases/download/v$version/$name$suffix" -o "$scratch/$name$suffix"
    done
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
    tar -xzf "$scratch/$name.tar.gz" -C "$scratch" "$name/lilbox"
    binary=$scratch/$name/lilbox
    [ -f "$binary" ] && [ ! -L "$binary" ] || { echo 'Invalid release executable.' >&2; exit 1; }
    destination=$prefix/bin/lilbox
    [ ! -L "$destination" ] && [ ! -d "$destination" ] || {
        echo "Refusing to replace a symlink or directory: $destination" >&2; exit 1;
    }
    mkdir -p "$prefix/bin"
    pending=$(mktemp "$prefix/bin/.lilbox.XXXXXXXX")
    cp "$binary" "$pending"
    chmod 755 "$pending"
    # Validate on the destination filesystem; TMPDIR may be mounted noexec.
    [ "$("$pending" --version)" = "lilbox $version" ] || { echo 'Release executable version mismatch.' >&2; exit 1; }
    mv -f "$pending" "$destination"
    pending=
    echo "Installed lilbox $version to $destination"
    case ":${PATH:-}:" in
        *":$prefix/bin:"*) ;;
        *) printf 'Add %s/bin to your PATH.\n' "$prefix" ;;
    esac
    if [ "$os" = Linux ] && [ ! -x /usr/bin/bwrap ]; then
        echo 'Linux execution requires /usr/bin/bwrap (bubblewrap 0.6.3+), user namespaces, and seccomp.'
        echo 'On Debian/Ubuntu: sudo apt-get install bubblewrap ca-certificates'
    fi
}
main "$@"
