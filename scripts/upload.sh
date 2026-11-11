#!/usr/bin/env bash
# Upload website assets before the page; preserve the installer and older assets.
set -euo pipefail
if [[ $# -gt 0 ]]; then
  echo 'Usage: just upload' >&2
  exit 2
fi
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
command -v gcloud >/dev/null || { echo 'Install and authenticate the Google Cloud CLI first.' >&2; exit 1; }
python3 scripts/check-www.py

# Validate a frozen copy too, so edits during upload cannot mix page/asset versions.
staging=$(mktemp -d "${TMPDIR:-/tmp}/lilbox-site-upload.XXXXXXXX")
trap 'rm -rf -- "$staging"' EXIT
cp www/index.html "$staging/index.html"
cp -R www/assets "$staging/assets"
python3 scripts/check-www.py "$staging"

destination='gs://muthanna.com/lilbox/'
printf 'Uploading website assets to https://mo.town/lilbox/\n'
gcloud storage rsync "$staging/assets/" "${destination}assets/" \
  --recursive --checksums-only --predefined-acl=publicRead \
  '--cache-control=public,max-age=86400,immutable'
printf 'Publishing index.html\n'
gcloud storage cp "$staging/index.html" "${destination}index.html" \
  --predefined-acl=publicRead \
  '--cache-control=public,max-age=300,must-revalidate' \
  --content-type=text/html
gcloud storage cat "${destination}index.html" > "$staging/uploaded.html"
cmp -s "$staging/index.html" "$staging/uploaded.html" || {
  echo 'Uploaded page does not match; check for a concurrent upload.' >&2
  exit 1
}
printf 'Published https://mo.town/lilbox/\nPublic caches may take up to five minutes to refresh.\n'
