#!/usr/bin/env bash
set -euo pipefail

# Update to latest prerelease or release
HOST="${HOST:-https://api.github.com}"
OWNER="${OWNER:-FENTTEAM}"
REPO="${REPO:-jorik-cli}"
TARGET="${TARGET:-x86_64-unknown-linux-gnu}"
DEST="${DEST:-/usr/local/bin/jorik}"
TOKEN="${GITHUB_TOKEN:-}"

API="${HOST}/repos/${OWNER}/${REPO}/releases"
AUTH=()
if [[ -n "$TOKEN" ]]; then
  AUTH=(-H "Authorization: token ${TOKEN}")
fi

echo "Fetching releases..."
# GitHub returns a list. We want the first one (latest by date, including prereleases)
releases_json="$(curl -fsSL "${AUTH[@]}" "${API}")"

# Find the first release that has an asset for our target
# jq logic: iterate over releases, find first where assets contain matching name
asset_url="$(
  jq -r --arg t "$TARGET" '
    .[] | .assets[]? | select(.name|endswith($t + ".tar.gz")) | .browser_download_url
  ' <<<"$releases_json" | head -n 1
)"

if [[ -z "$asset_url" || "$asset_url" == "null" ]]; then
  echo "Error: No matching asset found in recent releases." >&2
  exit 1
fi

tmp="$(mktemp -d)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

echo "Downloading: $asset_url"
curl -fsSL "${AUTH[@]}" -o "$tmp/jorik.tar.gz" "$asset_url"

echo "Extracting..."
tar -C "$tmp" -xzf "$tmp/jorik.tar.gz"

if [[ ! -f "$tmp/jorik" ]]; then
  echo "Error: expected binary 'jorik' inside the archive." >&2
  exit 1
fi

echo "Installing to $DEST (sudo if needed)..."
install_cmd=(install -m 0755 "$tmp/jorik" "$DEST")
if [[ ! -w "$(dirname "$DEST")" ]] || ([[ -e "$DEST" ]] && [[ ! -w "$DEST" ]]); then
  sudo "${install_cmd[@]}"
else
  "${install_cmd[@]}"
fi

echo "Done. Updated to:"
"$DEST" --version
