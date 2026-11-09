#!/usr/bin/env bash
# Publish the standalone installer using the same GCS deployment as mo.town.
set -euo pipefail

if [[ $# -gt 0 ]]; then
  if [[ $# == 1 && ( $1 == --help || $1 == -h ) ]]; then
    echo 'Usage: just release-installer'
    echo 'Publish the local install.sh to https://mo.town/lilbox/install.sh.'
    echo 'Requires authenticated gcloud with write access to the muthanna.com bucket.'
    exit 0
  fi
  echo 'Usage: just release-installer' >&2
  exit 2
fi

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
command -v gcloud >/dev/null || { echo 'Install and authenticate the Google Cloud CLI first.' >&2; exit 1; }
destination='gs://muthanna.com/lilbox/install.sh'
public_url='https://mo.town/lilbox/install.sh'

# Freeze the exact bytes to validate, publish, and compare, even if edited meanwhile.
staging=$(mktemp -d "${TMPDIR:-/tmp}/lilbox-installer-upload.XXXXXXXX")
trap 'rm -rf -- "$staging"' EXIT
cp install.sh "$staging/install.sh"
sh -n "$staging/install.sh"

printf 'Uploading installer to %s\n' "$public_url"
gcloud storage cp "$staging/install.sh" "$destination" \
  --predefined-acl=publicRead \
  '--cache-control=public,max-age=300,must-revalidate' \
  --content-type=text/plain

printf 'Verifying uploaded installer bytes\n'
gcloud storage cat "$destination" > "$staging/uploaded.sh"
if ! cmp -s "$staging/install.sh" "$staging/uploaded.sh"; then
  echo 'Uploaded installer does not match the staged file; check for a concurrent upload.' >&2
  exit 1
fi
printf 'Published %s\nPublic caches may take up to five minutes to refresh.\n' "$public_url"
