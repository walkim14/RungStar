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

## JavaScript runtime

Current YouTube extraction requires a JavaScript engine for player challenges. Every delivery
ships Deno beside yt-dlp, and the worker passes it explicitly with `--js-runtimes`. A system
Deno, Node, Bun or QuickJS is reused when present; otherwise the game atomically fetches the
official Deno archive into its data-directory `tools` folder. This is separate from the EJS
component itself, which yt-dlp may update from its official GitHub release when YouTube changes.

## ffmpeg

The Windows deliveries ship `ffmpeg.exe` beside the game, from the same FFmpeg 7.1 build the
linked DLLs come from. It is **not for the game** — that links the libraries directly and never
runs the program. It is for **yt-dlp**, which shells out to ffmpeg to pull audio out of a
container and to merge the separate video and audio streams YouTube serves above 360p, and
which looks for ffmpeg on the PATH and nowhere else. Naming it explicitly with
`--ffmpeg-location` is the whole fix for "ffmpeg is not installed" appearing at the end of an
otherwise successful download.

That costs `avfilter` and `avdevice` in the vendor tree, which the game itself does not link.
The alternative was an 80 MB static build; the shared one is 39 MB and reuses the DLLs already
there.

On Linux nothing is bundled. The Flatpak declares the optional `ffmpeg-full` extension and uses
the runtime's `/usr/bin/ffmpeg`. The 24.08 runtime was tested inside the finished bundle with
AV1, H.264, Vorbis and AAC support; a real yt-dlp run downloaded separate AV1 and Opus streams,
merged them, and decoded the result without `ffmpeg-full` mounted. Flathub-origin installs can
still auto-install the extension for its broader patented-codec set. Flatpak 1.14 does not pull
an optional extension across origins when installing a local `.flatpak`, which is safe here
because the required codecs and merger are already in the base runtime.

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
- `--filesystem=xdg-music`, `xdg-videos` and `/run/media` for song folders — the last because
  on a Deck a library of any size lives on the SD card
- `--share=network` for USDB

Not requested: `--filesystem=home`. A karaoke game does not need to read everything, and a song
folder somewhere else can be granted with `flatpak override` by whoever wants it there.

```bash
bash packaging/linux/build-flatpak.sh     # one .flatpak file, ready to copy to a Deck
bash packaging/linux/appimage.sh          # the AppImage, for everything else
```

`build-flatpak.sh` runs on **any** Linux with `flatpak` and `flatpak-builder`, including WSL —
which is how the first Deck build was made from a Windows machine. The host distribution does
not matter, because a Flatpak links against `org.freedesktop.Platform` rather than against
anything installed on the builder. It fetches the runtime, generates the pinned sources, builds,
and wraps the result as a single `.flatpak` bundle.

Two things the manifest has to do that are not obvious:

- **SDL3 is built from source as a module.** The 24.08 runtime ships SDL2 and nothing newer —
  SDL3 is younger than the runtime — so there is nothing to link against. It installs to
  `/app/lib` rather than CMake's default `lib64`, and `LIBRARY_PATH` points the linker there,
  because the `sdl3` crate emits a bare `-lSDL3` with a relative search path.
- **`llvm18` is an SDK extension**, for the libclang that `ffmpeg-sys` needs to generate its
  bindings. The base SDK has gcc and no clang at all.

FFmpeg itself comes from the runtime, and `/usr/bin/ffmpeg` is on the PATH inside the sandbox —
so yt-dlp can merge streams and downloads are full quality, with nothing bundled.

The AppImage exists for distributions where Flatpak is not set up. It bundles SDL3 and FFmpeg
and expects a system OpenGL/Vulkan driver, as every AppImage does.

### Installing it on a Steam Deck

In Desktop Mode, with the `.flatpak` file copied across:

```bash
flatpak install --user ./RungStar.flatpak
flatpak run de.rungstar.RungStar --check
```

Run `--check` first. It draws every screen and exits, and prints what the packaged build
actually found — which is how the assets being installed to `share/rungstar/assets` while the
game looked beside its own executable was caught. A build with that wrong still starts, borrows
a system font and plays nothing:

```
fonts    RungStar-Regular.ttf + RungStar-Fallback.ttf + DejaVuSans.ttf
sounds   6/6 loaded, device open, 4410 bytes queued
```

Then, in Game Mode, add it from the Steam library — Desktop Mode's application menu has an entry
for it, which "Add to Steam" picks up.

## Steam

`packaging/steam/launch.sh` is the launch command for a non-Steam shortcut or a depot. It sets
`SDL_VIDEODRIVER` only when it has to and otherwise stays out of the way — Gamescope handles
the rest, and overriding the video driver inside a Gamescope session is how a game ends up
windowed inside its own compositor.

Frame pacing is a setting rather than a guess: **Options → Graphics → Frame limit** and
**Vertical sync**. Under Gamescope, vsync on and no frame limit is right, because Gamescope is
already pacing and a second limiter beats against it.
