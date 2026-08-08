#!/usr/bin/env bash
# The launch command for a Steam shortcut or a depot.
#
# Deliberately almost nothing. Under Gamescope the compositor already decides the resolution,
# the refresh rate and the frame pacing, and a launcher that overrides SDL_VIDEODRIVER is how
# a game ends up windowed inside its own compositor with tearing it would not otherwise have.
#
# Frame pacing is a setting rather than a guess: Options -> Graphics -> Frame limit and
# Vertical sync. Under Gamescope, vsync on and no frame limit is right, because a second
# limiter beats against the one already running.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# A Flatpak install is the normal case on SteamOS; a loose build is what a developer has.
if command -v flatpak >/dev/null 2>&1 && flatpak info de.rungstar.RungStar >/dev/null 2>&1; then
    exec flatpak run de.rungstar.RungStar "$@"
fi

for candidate in "$here/rungstar" "$here/../../target/release/rungstar" "$(command -v rungstar || true)"; do
    if [ -x "$candidate" ]; then
        exec "$candidate" "$@"
    fi
done

echo "RungStar is not installed here. Build it with 'cargo build --release -p rungstar-app'," >&2
echo "or install the Flatpak: see packaging/README.md." >&2
exit 1
