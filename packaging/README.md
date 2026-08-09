# Packaging

Four ways out of the build tree, and one thing that has to be dropped in by hand.

| What | Where | Built by |
|---|---|---|
| Windows portable zip | `packaging/windows/portable.ps1` | anywhere with the toolchain |
| Windows installer | `packaging/windows/rungstar.iss` | Inno Setup 6 |
| Flatpak | `packaging/linux/de.rungstar.RungStar.yml` | `flatpak-builder` on Linux |
| AppImage | `packaging/linux/appimage.sh` | Linux with `appimagetool` |

## yt-dlp

All three deliveries ship one, so a fresh install can download a song immediately. The game
looks for it in three places, in this order: the **PATH**, then anything **newer it fetched
itself** into its data directory, then the copy **beside the executable**.

| Delivery | How it gets there |
|---|---|
| Windows zip and installer | `portable.ps1` downloads it beside `rungstar.exe` |
| AppImage | `appimage.sh` downloads it into `usr/bin` |
| Flatpak | `fetch-ytdlp.sh` pins it by checksum; the manifest installs `/app/bin/yt-dlp` |

The Flatpak is the awkward one and the one that matters on a Steam Deck. `flatpak-builder`
builds with **no network** — deliberately, so a build can be reproduced — so a bundled binary
cannot be curled from a build command the way the other two do it. It has to be a declared
source with a checksum, which is what `fetch-ytdlp.sh` generates. Run it before
`flatpak-builder`, exactly as with `cargo-sources.json`; CI does both.

That copy is pinned at build time and can never update itself, because `/app` is read-only.
That is fine and is why the search order is what it is: the game fetches a current yt-dlp into
`~/.var/app/de.rungstar.RungStar/data/rungstar/tools` when it needs one, and prefers it over the bundled
copy from then on. YouTube changes its extraction often enough that a binary frozen at release
time stops working within a few months, which is the whole reason the game shells out to a
separate tool rather than reimplementing extraction.

The Linux asset is `yt-dlp_linux`, the standalone build, not the plain `yt-dlp` zipapp: the
zipapp needs a Python interpreter on the machine, and a Steam Deck in Game Mode may not have a
usable one.

## Fonts and sounds

`assets/` is committed and every delivery copies it whole — four fonts under `assets/fonts/`
and eight interface sounds under `assets/sounds/`. Nothing has to be dropped in at packaging
time, which is the point: the shipped build looks and sounds the way the one on a developer's
machine does.

Both are covered by tests rather than by trust — `cargo test -p rungstar-platform` asserts what
the faces can draw and that every sound file is a WAV the mixer will accept. `assets/fonts/`
and `assets/sounds/` each carry a README explaining what is in them and how to change them.

The game still starts without either: it borrows a system face and stays silent. That is fine
for a developer and wrong for a release, so every packaging script here copies `assets/`.

## Windows

```powershell
# A folder and a zip under target/package/
pwsh packaging/windows/portable.ps1

# Then, with Inno Setup 6 installed:
iscc packaging/windows/rungstar.iss
```

The portable build carries `SDL3.dll` and the five FFmpeg DLLs beside the executable, because
that is where Windows looks first and it means no install and no PATH. The installer is the
same tree plus a Start-menu entry and an uninstaller.

**Settings and songs are never inside the install directory.** They go to
`%APPDATA%\RungStar`, so an uninstall or an upgrade cannot take somebody's highscores with it.

## Linux

The Flatpak is the one that matters for a Steam Deck: SteamOS has an immutable root filesystem,
so nothing installs into `/usr` and a distribution package is not an option. The manifest asks
for the permissions the game actually needs and no more:

- `--socket=pulseaudio` for output **and microphone capture**
- `--device=all` for controllers, which `--device=dri` alone does not cover
- `--filesystem=xdg-music` and `xdg-videos` for song folders
- `--share=network` for USDB

Not requested: `--filesystem=home`. A karaoke game does not need to read everything, and a song
folder outside the music directory can be granted with `flatpak override` by whoever wants it
there.

```bash
flatpak-builder --user --install --force-clean build packaging/linux/de.rungstar.RungStar.yml
bash packaging/linux/appimage.sh          # the AppImage, for everything else
```

The AppImage exists for distributions where Flatpak is not set up. It bundles SDL3 and FFmpeg
and expects a system OpenGL/Vulkan driver, as every AppImage does.

## Steam

`packaging/steam/launch.sh` is the launch command for a non-Steam shortcut or a depot. It sets
`SDL_VIDEODRIVER` only when it has to and otherwise stays out of the way — Gamescope handles
the rest, and overriding the video driver inside a Gamescope session is how a game ends up
windowed inside its own compositor.

Frame pacing is a setting rather than a guess: **Options → Graphics → Frame limit** and
**Vertical sync**. Under Gamescope, vsync on and no frame limit is right, because Gamescope is
already pacing and a second limiter beats against it.
