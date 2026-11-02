#!/usr/bin/env bash
set -euo pipefail

# Run from a trusted local checkout. Piping remote scripts into a shell is unsupported.
if [[ -z "${BASH_SOURCE[0]:-}" || ! -f "${BASH_SOURCE[0]}" ]]; then
  echo "Run this installer from a trusted local checkout, not curl | bash." >&2
  exit 1
fi
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
command -v python3 >/dev/null || { echo "Python 3 is required." >&2; exit 1; }
exec python3 "$SCRIPT_DIR/bootstrap.py" --pre "$@"
