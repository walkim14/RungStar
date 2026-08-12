#!/usr/bin/env bash
# Build the Flatpak and wrap it as a single installable file.
#
#     packaging/linux/build-flatpak.sh [output.flatpak]
#
# Run on any Linux with `flatpak` and `flatpak-builder` — including WSL, which is how the first
# Steam Deck build was made from a Windows machine. The host distribution does not matter: a
# Flatpak links against the `org.freedesktop.Platform` runtime rather than against whatever is
# installed here, so a bundle built on Ubuntu runs on SteamOS unchanged. That is the whole
# reason this is the delivery for a Deck.
#
# What comes out is a `.flatpak` bundle: one file, copied to the Deck and installed with
#
#     flatpak install --user ./RungStar.flatpak
#
# rather than a repository the Deck has to be able to reach.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="${1:-$root/target/package/RungStar.flatpak}"
app=de.rungstar.RungStar

need() { command -v "$1" >/dev/null || { echo "$1 is not installed" >&2; exit 1; }; }
need flatpak
need flatpak-builder
need python3

echo "==> runtime and SDK"
flatpak remote-add --if-not-exists --user flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install -y --noninteractive --user flathub \
    org.freedesktop.Platform//24.08 \
    org.freedesktop.Sdk//24.08 \
    org.freedesktop.Sdk.Extension.rust-stable//24.08 \
    org.freedesktop.Sdk.Extension.llvm18//24.08

# Which commit this is, written into the copy the build sees. The Flatpak build has no `.git`
# — the tree is copied without it — so the binary cannot work this out for itself, and a
# packaged build that cannot say what it is turns "did you rebuild after that fix?" into
# guesswork.
build_id="$(git -C "$root" describe --always --dirty 2>/dev/null || echo unknown)"
echo "==> building $build_id"

echo "==> pinned sources"
# flatpak-builder builds with **no network**, on purpose: a build that can reach the internet is
# a build that cannot be reproduced. So every dependency has to be a declared source with a
# checksum, which is what these three produce. Cargo sources are regenerated whenever the lock
# file changes; an ignored file left from yesterday otherwise looks valid until the offline
# build fails on the first newly added crate.
if [ ! -f "$here/cargo-sources.json" ] || [ "$root/Cargo.lock" -nt "$here/cargo-sources.json" ]; then
    generator=/tmp/flatpak-cargo-generator.py
    [ -f "$generator" ] || curl -fsSL -o "$generator" \
        https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
    echo "    generating cargo-sources.json (needs python3-aiohttp and python3-tomlkit)"
    (cd "$root" && python3 "$generator" Cargo.lock -o "$here/cargo-sources.json")
fi
[ -f "$here/ytdlp-source.json" ] || bash "$here/fetch-ytdlp.sh"
[ -f "$here/deno-source.json" ] || bash "$here/fetch-deno.sh"

# Built in a Linux filesystem rather than wherever the checkout is. flatpak-builder does a great
# deal of small-file work, and on a Windows drive mounted into WSL that is ten times slower —
# quite apart from the permissions such a mount cannot represent.
work="${RUNGSTAR_BUILD_DIR:-$HOME/.cache/rungstar-flatpak}"
if [ "${root#/mnt/}" != "$root" ]; then
    echo "==> copying the tree out of $root (a Windows mount is too slow to build on)"
    rm -rf "$work/src"
    mkdir -p "$work/src"
    # `vendor/` is 155 MB of Windows DLLs this build does not use, and `target/` is the other
    # platform's output.
    tar -C "$root" --exclude=./target --exclude=./vendor --exclude=./.git -cf - . |
        tar -C "$work/src" -xf -
    source_dir="$work/src"
else
    source_dir="$root"
fi
echo "$build_id" > "$source_dir/BUILD_ID"

# Windows line endings in anything that lands on a Linux machine. A `.desktop` with them fails
# validation, which is what stops the game appearing in the application menu and therefore in
# Steam; a shell script with them fails on its first line with `$'\r': command not found`. Both
# shipped that way once, and a checkout on Windows can reintroduce either at any time.
crlf=0
while IFS= read -r file; do
    if grep -qU $'\r' "$file"; then
        echo "$file has Windows line endings and would break on the target" >&2
        crlf=1
    fi
done < <(find "$source_dir/packaging" -type f \( -name '*.sh' -o -name '*.desktop' \
    -o -name '*.xml' -o -name '*.svg' \))
[ "$crlf" -eq 0 ] || exit 1

echo "==> building"
(cd "$source_dir" && flatpak-builder --user --force-clean --repo="$work/repo" \
    "$work/build" packaging/linux/"$app".yml)

echo "==> bundling"
mkdir -p "$(dirname "$out")"
rm -f "$out"
# `--runtime-repo` is what lets the Deck fetch the runtime it needs on first install without
# anybody having added Flathub by hand first.
flatpak build-bundle "$work/repo" "$out" "$app" \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo

size="$(du -h "$out" | cut -f1)"
echo
echo "built $out ($size)"
echo "sha256 $(sha256sum "$out" | cut -d' ' -f1)"
echo
echo "On the Deck, in Desktop Mode:"
echo "    flatpak install --user ./$(basename "$out")"
echo "    flatpak run $app --check      # prove it works before looking for it in Steam"
