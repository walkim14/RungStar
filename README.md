# RungStar

A karaoke game in the UltraStar tradition, written in Rust for Windows and SteamOS.

It aims at feature parity with [UltraStar Deluxe](https://github.com/UltraStar-Deluxe/USDX)
while folding in an in-game replacement for
[usdb_syncer](https://github.com/bohning/usdb_syncer), so browsing and downloading songs no
longer means leaving the game.

**Status: early.** The song-format crate is complete and tested; the game around it is not
built yet. See `CLAUDE.md` for what works today.

## What it is meant to do differently

- **Controller-first.** Every screen, including the song editor, drivable from a gamepad —
  UltraStar Deluxe translates a single controller into fake keystrokes and no more.
- **Instant library.** A persistent SQLite index with full-text search over lyrics as well as
  metadata, instead of re-parsing every `.txt` at each launch.
- **Downloads that do not interrupt.** Songs become playable as soon as the text and audio
  land; artwork and video stream in behind them.
- **Up to 6 singers on 4+ microphones**, including the stereo splitting that lets one device
  carry two players.

## Compatibility

Reads and writes the UltraStar `.txt` format, `.upl` playlists, and USDB's `#VIDEO` meta-tag
convention, and imports existing `Ultrastar.db` highscores.

## Licence

GPL-3.0-or-later. See `LICENSE` and `NOTICE.md`.
