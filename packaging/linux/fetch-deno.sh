#!/usr/bin/env bash
# Pin the latest official Deno x86_64 Linux archive for an offline Flatpak build.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
asset=deno-x86_64-unknown-linux-gnu.zip
latest="https://github.com/denoland/deno/releases/latest/download/$asset"
temporary="$(mktemp)"
trap 'rm -f "$temporary"' EXIT

effective="$(curl -fsSL -o "$temporary" -w '%{url_effective}' "$latest")"
sha256="$(sha256sum "$temporary" | awk '{print $1}')"
cat > "$here/deno-source.json" <<JSON
{
    "type": "archive",
    "url": "$effective",
    "sha256": "$sha256",
    "dest": "deno-runtime"
}
JSON

echo "pinned $effective"
echo "sha256 $sha256"
