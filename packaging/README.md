# Packaging

Four ways out of the build tree, and one thing that has to be dropped in by hand.

| What | Where | Built by |
|---|---|---|
| Windows portable zip | `packaging/windows/portable.ps1` | anywhere with the toolchain |
| Windows installer | `packaging/windows/rungstar.iss` | Inno Setup 6 |
| Flatpak | `packaging/linux/de.rungstar.RungStar.yml` | `flatpak-builder` on Linux |
| AppImage | `packaging/linux/appimage.sh` | Linux with `appimagetool` |

## yt-dlp

Both the Windows and the AppImage scripts fetch the latest yt-dlp and put it beside the
executable, so a fresh install can download a song immediately. The game looks there — after
the PATH, and after anything newer it has fetched itself into `%APPDATA%\RungStar	ools`.

It is bundled but not pinned, and both of those matter. Bundled, because a release that cannot
download until it has downloaded something else has a hole in it. Not pinned, because YouTube
changes its extraction often enough that a copy frozen at release time stops working within a
few months — which is the whole reason the game shells out to it rather than reimplementing it.

## The font

`assets/fonts/` is empty in the repository, and a packaged build looks there first —
`RungStar-Regular.ttf` and `RungStar-Bold.ttf`. Drop a face in before packaging.

It is not committed on purpose. A font binary is a megabyte of something nobody reviews in a
diff, and picking one is a licensing decision rather than a technical one. Any face whose
licence permits redistribution works; **DejaVu Sans** and **Noto Sans** are the obvious
candidates, both cover the Latin, Greek and Cyrillic a real song library contains, and both are
already on most Linux machines.

Without one the game still starts and borrows a system face — Segoe UI on Windows, DejaVu on
Linux — and says so if it cannot find either. That is fine for a developer and wrong for a
release: it makes the game look different on every machine, and a Flatpak has almost no system
fonts to borrow.

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
