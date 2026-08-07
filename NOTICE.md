# Notices and attribution

RungStar is licensed under the **GNU General Public License, version 3 or later**
(see `LICENSE`).

## Why GPL-3.0-or-later

RungStar is an independent implementation, but its file formats, scoring semantics and
network protocol are derived from the specifications of two copyleft projects:

| Project | License | What RungStar derives from it |
|---|---|---|
| [UltraStar Deluxe](https://github.com/UltraStar-Deluxe/USDX) | GPL-2.0-**or-later** | The UltraStar `.txt` song format, beat/BPM semantics, the CAMDF pitch-detection parameters, the scoring model and rating tiers, the `.upl` playlist format, and the microphone channel→player routing model. |
| [usdb_syncer](https://github.com/bohning/usdb_syncer) | GPL-3.0-**only** | The USDB (usdb.animux.de) request protocol, the `#VIDEO:` meta-tag mini-format, the song-text normalisation rules, and the download/sync-meta model. |

GPL-2.0-or-later permits relicensing under GPL-3.0; GPL-3.0-only does not permit anything
else. The combined work therefore must be, and is, **GPL-3.0-or-later**.

No source code from either project is copied into RungStar. Both are Pascal and Python
respectively; RungStar is written from scratch in Rust against a written specification
distilled from reading them. Behavioural compatibility is verified by tests, not by shared code.

## Test fixtures

Song-text test fixtures under `crates/rungstar-song/tests/fixtures/` and USDB HTML fixtures
under `crates/rungstar-usdb/tests/fixtures/` originate from the usdb_syncer test suite and
remain under GPL-3.0-only. They are used unmodified so that RungStar's behaviour can be
checked against the reference implementation's own expectations.

## Runtime dependencies

RungStar invokes, but does not link or redistribute modifications of:

- **yt-dlp** (Unlicense) — downloaded and self-updated at runtime into the user data directory.
- **FFmpeg** (LGPL-2.1-or-later / GPL depending on build) — dynamically linked. Distributed
  builds ship an LGPL FFmpeg; see `docs/third-party.md` in release artifacts.

Rust crate dependencies retain their own licenses; run `cargo about` or `cargo deny` to
regenerate the full manifest.

## Trademarks

"UltraStar Deluxe", "USDX" and "usdb_syncer" are the names of their respective projects and
are used here only to describe compatibility. RungStar is not affiliated with or endorsed by
either project, nor by usdb.animux.de.
