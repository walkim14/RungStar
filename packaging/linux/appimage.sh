#!/usr/bin/env bash
# Build an AppImage.
#
# The Flatpak is the right answer on SteamOS and on any distribution that has Flatpak set up.
# This exists for the ones that do not, and for "download one file and run it", which is still
# the shortest path from a link to a running game.
#
# SDL3 and FFmpeg are bundled; the graphics driver is not, as in every AppImage — a driver
# belongs to the machine, not to the program.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/target/package"
appdir="$out/RungStar.AppDir"

command -v appimagetool >/dev/null 2>&1 || {
    echo "appimagetool is not on the path. Get it from" >&2
    echo "https://github.com/AppImage/AppImageKit/releases and put it somewhere on PATH." >&2
    exit 1
}

cd "$root"
cargo build --release -p rungstar-app

rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/share/rungstar"
install -Dm755 target/release/rungstar "$appdir/usr/bin/rungstar"
[ -x target/release/rungstar-diagnostics ] &&
    install -Dm755 target/release/rungstar-diagnostics "$appdir/usr/bin/rungstar-diagnostics"

cp -r assets "$appdir/usr/share/rungstar/"
install -Dm644 LICENSE "$appdir/usr/share/licenses/rungstar/LICENSE"
install -Dm644 NOTICE.md "$appdir/usr/share/licenses/rungstar/NOTICE.md"
install -Dm644 packaging/linux/de.rungstar.RungStar.desktop "$appdir/de.rungstar.RungStar.desktop"
install -Dm644 packaging/linux/rungstar.svg "$appdir/de.rungstar.RungStar.svg"

# The shared libraries the executable actually needs, minus the ones that belong to the
# machine. Bundling libGL or libasound is how an AppImage stops working on the next distro.
keep_out='libc\.|libm\.|libdl\.|libpthread\.|librt\.|ld-linux|libGL|libEGL|libGLX|libX11|libxcb|libwayland|libasound|libpulse|libdrm|libgbm'
ldd target/release/rungstar | awk '/=> \// {print $3}' | while read -r lib; do
    if ! echo "$lib" | grep -Eq "$keep_out"; then
        cp -Lv "$lib" "$appdir/usr/lib/" 2>/dev/null || true
    fi
done

# yt-dlp beside the executable, so a download works out of the box. The standalone build, not
# the zipapp: a Deck in Game Mode may have no usable Python. The game fetches a newer one into
# its data directory when it needs to.
if curl -fsSL -o "$appdir/usr/bin/yt-dlp"     https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux; then
    chmod +x "$appdir/usr/bin/yt-dlp"
else
    echo "could not bundle yt-dlp; the game will fetch it on first download" >&2
    rm -f "$appdir/usr/bin/yt-dlp"
fi

# Deno is the supported JS runtime for yt-dlp's YouTube challenge solver. Keep it beside the
# extractor so Game Mode needs neither a system Node installation nor a first-run repair.
deno_zip="$out/deno-linux.zip"
if command -v unzip >/dev/null 2>&1 && curl -fsSL -o "$deno_zip"     https://github.com/denoland/deno/releases/latest/download/deno-x86_64-unknown-linux-gnu.zip; then
    unzip -p "$deno_zip" deno > "$appdir/usr/bin/deno"
    chmod +x "$appdir/usr/bin/deno"
else
    echo "could not bundle Deno; the game will fetch it on first download" >&2
    rm -f "$appdir/usr/bin/deno"
fi
rm -f "$deno_zip"

cat > "$appdir/AppRun" <<'RUN'
#!/usr/bin/env bash
here="$(dirname "$(readlink -f "${0}")")"
export LD_LIBRARY_PATH="$here/usr/lib:${LD_LIBRARY_PATH:-}"
# Assets are found beside the executable, which is where the font loader looks first.
cd "$here/usr/share/rungstar" 2>/dev/null || true
exec "$here/usr/bin/rungstar" "$@"
RUN
chmod +x "$appdir/AppRun"

appimagetool "$appdir" "$out/RungStar-x86_64.AppImage"
echo "built $out/RungStar-x86_64.AppImage"
