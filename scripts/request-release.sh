#!/usr/bin/env bash
set -euo pipefail
CDPATH='' cd -- "$(dirname "$0")/.."
exec python3 scripts/version.py release "$@"
