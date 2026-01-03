#!/usr/bin/env bash
set -euo pipefail

# Configuration (override via env or args)
HOST="${HOST:-https://git.xserv.pp.ua}"
OWNER="${OWNER:-xxanqw}"
REPO="${REPO:-jorik-cli}"
TARGET="${TARGET:-x86_64-unknown-linux-gnu}"
DEST="${DEST:-/usr/local/bin/jorik}"
TOKEN="${GITEA_TOKEN:-}"
# If you want a specific tag, set TAG=vX.Y.Z (otherwise uses /releases/latest)
TAG="${TAG:-}"

API="${HOST}/api/v1/repos/${OWNER}/${REPO}/releases"
AUTH=()
if [[ -n "$TOKEN" ]]; then
  AUTH=(-H "Authorization: token ${TOKEN}")
fi

tmp="$(mktemp -d)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

echo "Fetching release metadata..."
if [[ -n "$TAG" ]]; then
  rel_json="$(curl -fsSL "${AUTH[@]}" "${API}/tags/${TAG}")"
else
  rel_json="$(curl -fsSL "${AUTH[@]}" "${API}/latest")"
fi

asset_url="$(
  jq -r --arg t "$TARGET" '
    .assets[]? | select(.name|endswith($t + ".tar.gz")) | .browser_download_url
  ' <<<"$rel_json"
)"

if [[ -z "$asset_url" || "$asset_url" == "null" ]]; then
  echo "Error: No asset found ending with ${TARGET}.tar.gz" >&2
  exit 1
fi

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

echo "Done. Version:"
"$DEST" --help >/dev/null 2>&1 && "$DEST" --version || true
