#!/usr/bin/env bash
# Pin the current yt-dlp for the Flatpak build.
#
# flatpak-builder runs with **no network**, on purpose: a build that can reach the internet is
# a build that cannot be reproduced. So a bundled yt-dlp cannot be curled from a build command
# the way the AppImage does it — it has to be declared as a source with a checksum, which means
# knowing which release is being pinned.
#
# This fetches the current one, hashes it, and writes the source stanza the manifest includes.
# Run it before flatpak-builder, exactly as with `cargo-sources.json`.
#
# Pinning at build time is fine and does not contradict "never pin yt-dlp": the copy in the
# Flatpak is the one that gets you a first download working out of the box, and `/app` is
# read-only so it could never update itself anyway. The game keeps a newer one in its data
# directory — `~/.var/app/de.rungstar.RungStar/data/tools` — and prefers that over this.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
asset="yt-dlp_linux"
url="https://github.com/yt-dlp/yt-dlp/releases/latest/download/${asset}"
out="$here/ytdlp-source.json"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "fetching $url"
curl -fsSL -o "$tmp/$asset" "$url"

# The standalone build, not the zipapp. Plain `yt-dlp` needs a Python interpreter on the
# machine and a Steam Deck in Game Mode may not have a usable one.
size="$(wc -c < "$tmp/$asset")"
if [ "$size" -lt 500000 ]; then
    echo "what GitHub sent back was not a program ($size bytes)" >&2
    exit 1
fi
sha="$(sha256sum "$tmp/$asset" | cut -d' ' -f1)"

# The URL is the `latest` redirect, so the checksum is what actually pins it. Resolve it to the
# versioned URL as well, so a rebuild of this exact manifest fetches this exact file rather
# than whatever "latest" has become since.
resolved="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$url")"

cat > "$out" <<JSON
{
    "type": "file",
    "url": "${resolved}",
    "sha256": "${sha}",
    "dest-filename": "yt-dlp"
}
JSON

echo "pinned ${resolved}"
echo "sha256 ${sha}"
echo "wrote  $out"
