#!/usr/bin/env bash
# Delegate release preparation/dispatch to the shared local and CI version checks.
set -euo pipefail
CDPATH='' cd -- "$(dirname "$0")/.."
exec python3 scripts/version.py release "$@"
