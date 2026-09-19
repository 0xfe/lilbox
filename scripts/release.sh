#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $(uname -s) != Darwin ]]; then
    echo 'Release packaging currently requires macOS.' >&2
    exit 1
fi
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *) echo "Unsupported release target: $target" >&2; exit 1 ;;
esac

package_id=$(cargo pkgid --offline)
version=${package_id##*[#@]}
name="lilbox-${version}-${target}"
# Explicit target directory makes output independent of CARGO_TARGET_DIR.
cargo build --release --locked --target "$target" --target-dir target
mkdir -p dist
stage=$(mktemp -d "${TMPDIR:-/tmp}/lilbox-release.XXXXXXXX")
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"
install -m 755 "target/$target/release/lilbox" "$stage/$name/lilbox"
cp README.md LICENSE CHANGELOG.md "$stage/$name/"
cp -R profiles "$stage/$name/profiles"
cp -R docs "$stage/$name/docs"
# Stable tar metadata and gzip headers, given identical compiled input bytes.
# Python 3 is available with the macOS developer tools needed to build Rust.
python3 - "$stage" "$name" "dist/$name.tar.gz" <<'PY'
import gzip
import pathlib
import sys
import tarfile

stage, name, destination = sys.argv[1:]
root = pathlib.Path(stage) / name
with open(destination, "wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for path in [root, *sorted(root.rglob("*"))]:
                info = archive.gettarinfo(str(path), arcname=str(path.relative_to(stage)))
                info.uid = info.gid = info.mtime = 0
                info.uname = info.gname = ""
                info.mode = 0o755 if path.is_dir() or path.name == "lilbox" else 0o644
                if path.is_file():
                    with path.open("rb") as source:
                        archive.addfile(info, source)
                else:
                    archive.addfile(info)
PY
(
    cd dist
    shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"
)
echo "Created dist/$name.tar.gz and dist/$name.tar.gz.sha256"
