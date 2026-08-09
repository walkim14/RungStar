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
#
# By **tag**, not by following the redirect. `latest/download/...` redirects to a signed asset
# URL with an expiry an hour away, so writing that into the manifest produces a build that
# works this afternoon and 403s tomorrow — which is the opposite of pinning.
#
# Two things about parsing it. GitHub sends the whole release as a single line, so a greedy
# pattern over it finds the *last* `tag_name` in the document rather than the first and there
# are several — hence splitting on commas. And the JSON is fetched to a file rather than piped
# into `grep -m1`: `grep` closing the pipe after its first match hands curl a SIGPIPE, which
# under `pipefail` kills the script one line after it has already got the answer it wanted.
curl -fsSL -o "$tmp/release.json" https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest
tag="$(tr ',' '
' < "$tmp/release.json" | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
if [ -z "$tag" ]; then
    echo "could not work out which release is current" >&2
    exit 1
fi
resolved="https://github.com/yt-dlp/yt-dlp/releases/download/${tag}/${asset}"

# And check that the tag really serves the bytes that were just hashed, rather than assuming
# `latest` and the tag are the same thing at the moment this runs.
curl -fsSL -o "$tmp/by-tag" "$resolved"
if [ "$(sha256sum "$tmp/by-tag" | cut -d' ' -f1)" != "$sha" ]; then
    echo "$tag does not serve what latest/ served; try again" >&2
    exit 1
fi

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
