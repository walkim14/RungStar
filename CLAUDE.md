# RungStar — working notes

An UltraStar Deluxe–class karaoke game in Rust, targeting Windows and SteamOS/Steam Deck,
with an in-game USDB song browser replacing usdb_syncer.

Full plan: `C:\Users\walki\.claude\plans\fully-build-and-test-optimized-backus.md`

## Licensing

**GPL-3.0-or-later**, and this is forced, not chosen: behaviour is derived from UltraStar
Deluxe (GPL-2.0-or-later) and usdb_syncer (GPL-3.0-only). See `NOTICE.md`. No upstream source
is copied — everything is reimplemented from a written spec, which is what makes it testable.

## Reference checkouts

Both upstreams are cloned in the session scratchpad for consultation:

- `.../scratchpad/usdx` — UltraStar Deluxe (Free Pascal)
- `.../scratchpad/usdb_syncer` — usdb_syncer (Python)

Re-clone if the scratchpad is gone:
`git clone --depth 1 https://github.com/UltraStar-Deluxe/USDX.git` and
`git clone --depth 1 https://github.com/bohning/usdb_syncer.git`

## Commands

```bash
cargo run -p rungstar-app --bin rungstar-diagnostics   # mic/pitch/controller check
cargo run --release --example scale -p rungstar-library  # 30k-song scan/search timings
cargo run -p rungstar-app --bin rungstar-sing -- <song.txt> [--mic <name>]  # play and score
cargo run --release -p rungstar-app --bin rungstar        # the game
cargo run --release -p rungstar-app --bin rungstar -- --check   # start, draw every screen, exit
cargo run --release --example index -p rungstar-library -- <folder>  # scan a real library
cargo test --workspace                              # all tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
PROPTEST_CASES=5000 cargo test -p rungstar-song     # deep property run
```

Rust is at `~/.cargo/bin`; add it to `PATH` in each shell.

## Facts that are easy to get wrong

- **`#BPM` is a quarter of the real beat rate.** `time = GAP/1000 + beat*60/(bpm*4)`.
  Everything downstream depends on this; see `bpm.rs`.
- **Pitch matching is octave-agnostic.** The sung tone is folded to within ±6 semitones of
  the target before comparison, so only the pitch class matters.
- **Scoring pools**: 9000 points across notes weighted by `duration × score_factor`
  (freestyle 0, normal/rap 1, golden 2), plus 1000 spread evenly over non-empty lines.
- **Detection runs on a different clock than drawing** — `-0.5` beats plus a 140 ms mic delay.
- **Units are inconsistent in the format itself**: `#GAP` and `#END` are milliseconds,
  `#VIDEOGAP`, `#START` and `#PREVIEWSTART` are seconds.
- **Parsing is deliberately lenient.** Junk lines and malformed notes are skipped with a
  `Warning`; only a genuinely unusable file errors. Real libraries are full of broken files
  and refusing them is worse than skipping a line.
- **Empty header values are never written.** The parser ignores them, so writing one produces
  a file that does not read back as what was written.

## Library index

One row per `.txt` in SQLite, plus an FTS5 table carrying the same metadata *and the full
lyrics* — which nothing upstream indexes and which is what makes "find the song that goes like
this" work. FTS5 is configured with `remove_diacritics 2` and `prefix '2 3 4'`, so "bjork"
finds "Björk" and "bea" narrows without a scan.

A rescan only opens files whose size or timestamp changed; everything else is skipped without
being read. `ScanOptions::verify` forces a full re-read for when the index is not trusted.
Play counts live in the same row but are never written by a scan — they are the player's
history, not a property of the file, and a rescan must not wipe them.

When FTS returns nothing but something was typed, the query falls back to bounded Levenshtein
over artist and title. That is what turns "beatls jud" into "Hey Jude", and it only runs after
the fast path has already come back empty.

## Testing approach

Conformance is measured against usdb_syncer's own fixture corpus, copied unmodified into
`crates/rungstar-song/tests/fixtures/`:

- `normalized/` — parse then write must be the identity
- `deviant/` — `*_in.txt` must normalise to `*_out.txt` (parse + write, no repairs)
- `fixes/` — plus the full repair suite, with `FixOptions::usdx_style()`
- `invalid/` — must be rejected

These pass **byte for byte**. When changing the parser or the repair passes, that must stay
true; it is the only real compatibility signal available without a song library.

### Deliberate divergences from the reference

The reference implementations are a specification, not a standard: where they are wrong, we
fix it. Each divergence below is pinned by a regression test, and none of them change the
fixture output.

1. **Song origin is the earliest note, not the first listed one.** `Tracks::start()` takes the
   minimum. usdb_syncer takes `notes[0]`, so a file with out-of-order notes ends up shifted
   off beat zero once the reordering pass runs.
2. **Overlap repair actually sorts.** The reference does one pass of adjacent swaps, which
   cannot order anything worse than a single inversion. We redistribute the sorted timings
   across the syllables in written order — lyrics stay as typed, time moves forwards.
3. **Space normalisation is a fixed point.** The reference hands the last syllable of a line a
   trailing space unconditionally, so a syllable with no lyric gained one and the next run
   migrated it backwards, drifting a space per run.
4. **All-caps detection ignores line initials.** Judging on every letter made two passes
   fight: capitalising a line could leave a song with no lower-case letters, which the next
   run then flattened.
5. **Capitalisation is length-preserving.** German sharp s upper-cases to "SS", which changes
   the word and feeds defect 4.
6. **Empty header values are never written.** The parser ignores them, so emitting one
   produced a file that did not read back as what was written.
7. **The song clock cannot get stuck held.** When the audio falls behind, the clock holds
   rather than stepping back. UltraStar Deluxe releases the hold when the running average
   drift reaches zero — but an exponential average approaches zero from below and never
   arrives, so the clock can stay held indefinitely. Ours releases when drift re-enters the
   dead band.
8. **A freestyle-only line earns no line bonus.** UltraStar Deluxe excludes such lines from
   the bonus divisor but still pays them a full bonus, so a song containing one can score
   over 10,000. Ours cannot.
9. **Hovering does not move the browser cursor.** In the list and the roulette the cursor is
   centred and the songs scroll past it, so selecting on hover drags the list out from under
   the pointer and you click a different song than you aimed at. Click selects, a second
   click sings — so a stray click never starts a song either.
10. **A text search ranks by relevance, and a phrase beats scattered words.** UltraStar sorts
   alphabetically always, so searching only works if you already know the title. Two parts:
   an unsorted search ranks by bm25 rather than by artist, and — because bm25 scores term
   count and document length but has no notion of *adjacency* — a second pass promotes songs
   containing the words as a phrase. Without it, "never gonna give you up" returns every song
   containing those five words scattered about, in artist order. Picking a sort explicitly
   turns ranking off, because reordering a list somebody asked to be alphabetical is wrong.

Normalisation is now idempotent, verified by property test over 20k generated songs.

## Native dependencies

SDL3 3.4.14 is vendored at `vendor/sdl3/` as the official prebuilt Windows x64 binaries.
Building it from source needs a CMake toolchain that can find a C compiler, and the Visual
Studio generator fails to do so with only the Build Tools installed — a very common setup.
`crates/rungstar-platform/build.rs` points the linker at the vendored copy and puts `SDL3.dll`
next to the built executable. On Linux SDL3 comes from the system package; nothing is
vendored.

The diagnostics tool draws with SDL's own renderer and built-in debug font, not wgpu. It is a
tool rather than a screen, and it should keep working while the renderer is rewritten around
it. wgpu arrives with the sing screen.

## Performance

Measured on the dev machine (`cargo bench -p rungstar-pitch`). The budget is six players
analysed at 100 Hz, i.e. 600 detections per second, alongside decode and render.

| Operation | Time | 6 players @ 100 Hz |
|---|---|---|
| CAMDF detection | 12.9 us | 0.8% of a core |
| MPM detection | 101 us | 6% of a core |
| Push 512 samples | 0.39 us | negligible |

Library, 30,000 songs (`cargo run --release --example scale -p rungstar-library`):

| Operation | Time |
|---|---|
| Warm rescan (what every launch pays) | 1.2 s |
| Search: prefix | 3.3 ms |
| Search: two words | 8.1 ms |
| Search: lyrics | 44 ms |
| Search: fuzzy fallback (only on a miss) | 78 ms |
| Browse by artist | 6.2 ms |
| Cold scan (paid once) | 3.9 s |

The cold scan was 82 s until the real cause turned up, and it was **not** a cost — it was a
pathology. Writing a song ran an FTS5 `DELETE` before its `INSERT`. On a first scan every one
of those deletes matched nothing, but FTS5 buffers pending index terms in memory and **a
delete forces that buffer to flush**, so the index was built in thirty thousand small pieces
instead of a few large ones. Skipping the delete for rows being inserted for the first time
took the cold scan from 82 s to 3.9 s.

Two earlier hypotheses about this were wrong — first that syscalls dominated (batching the
index lookup and replacing four `stat` calls per song with one directory listing bought 21%
warm and only 8% cold), then that tokenising 36 MB of lyrics was simply slow. Tokenising is
not free, but it was never the bottleneck. The skip is only correct while "new" means "this
path has no index entry", so `tests/fts_sync.rs` pins the two ways that could stop being true.

The *verifying* rescan still pays the old price, because there every row genuinely needs its
delete. That path is the rare "I do not trust the index" case, so it has not been optimised.

### Measured on a real library

8,134 songs at `UltraStarPlaySongConverter/UltraStarPlaySongsToBeConverted`, via
`cargo run --release --example index -p rungstar-library`:

| Operation | Time |
|---|---|
| Cold scan, cold file cache | 14.1 s (1.7 ms/song) |
| Cold scan, warm file cache | 3.9 s |
| Warm rescan | 0.40 s |
| Search: prefix / two words | 3.3 / 6.2 ms |
| Search: lyric line | 2.0 ms |
| Search: fuzzy fallback | 26 ms |

**Zero parse failures across all 8,134 files**, including titles with curly quotes and
non-ASCII folder names. 35 languages, 354 genres, 295 editions.

## The user interface

`rungstar-ui` has **no graphics API in it**. Screens turn state into a `DrawList` of rectangles
and strings in design units; a backend turns that into pixels. Two things fall out: the whole
interface is testable without a window — the tests assert *commands*, which is stronger than a
screenshot — and the renderer can be replaced without touching a screen. `rungstar-platform`
consumes the list through SDL today; wgpu will be a sibling of that file, not a rewrite.

**Resolution independence is real, not a scale factor.** The design space is 1000 units tall
and as wide as the aspect ratio makes it, so a lyric line is 64 units on every display and a
wider screen gets more room rather than a stretched picture. Layout is composition of
rectangles (`cut_top`, `columns`, `anchored`), so a six-player screen is the same code as a
one-player screen with a different split. USDX hand-places every player count in 800x600
coordinates, which is why it ships one layout per count per theme.

**A theme sets only how things look** — colours, fonts, radius, spacing. Layout belongs to the
screen. So a theme is forty lines of TOML, cannot be broken by a resolution, and cannot break a
screen added after it was written. Skin and accent vary independently and every derived colour
(raised, sunken, on-accent) is computed, because asking theme authors to get contrast right by
hand is how themes end up with an invisible list cursor. The built-in theme is compiled in, so
a missing file cannot stop the game starting.

**Options pages are derived from the settings**, not written beside them: an option cannot
exist without a label and a help string, and a test walks every row of every page checking it
steps and comes back. USDX has ten hand-written options screens that disagree with each other.

The three browse layouts (List, Chessboard, Roulette) are one state machine with three
placement functions, so switching keeps the cursor, filter and scroll position. Scrolling
animates by keeping cursor and view as separate values — the cursor never waits on an
animation, so input is never dropped during a fast scroll through 30,000 songs.

Fonts are rasterised with `fontdue` into a shelf-packed atlas per (face, whole-pixel size) and
drawn with colour modulation. **No font is vendored yet** — the game borrows a system face
(Segoe UI on Windows, DejaVu on Linux) and says so if it cannot. Bundling an OFL face belongs
with packaging.

## Status

- **Phase 0 — foundation**: done. Workspace, CI, licence, toolchain.
- **Phase 1 — `rungstar-song`**: done. Parser, writer, beat maths, meta tags, repair suite.
  30 tests, byte-exact fixture conformance.
- **Phase 2 — `rungstar-pitch` + `rungstar-score`**: done. Two detectors, the scoring
  engine, 31 tests, benchmarked.
- **Phase 3 — audio, input, a window**: mostly done. Master clock, capture routing, semantic
  input, SDL3 backend, and a runnable diagnostics tool. Still to do: audio playback/decode
  (Symphonia) and the wgpu renderer, both of which land with the sing screen in Phase 5.
- **Phase 4 — library index and search**: done. Incremental scanner, SQLite + FTS5 index over
  metadata *and* lyrics, fuzzy fallback, facets, sorts, `.upl` playlists.
- **Phase 5 — the sing screen**: playable. Decode, playback, clock sync, live capture,
  scoring, note/lyric rendering and a results summary all wired end to end in
  `rungstar-sing`. Still to come: multiple singers on screen, duet layout, video and image
  backgrounds, the five lyric effects, pause menu, and the proper results screen.

  Known simplification: `rungstar-sing` scores one singer from one capture device. The routing
  underneath already handles six across several devices — the diagnostics tool proves it — it
  is only the screen that assumes one.

  Device choice skips names that look virtual (Steam Streaming Microphone and friends). They
  sort to the top of SDL's list and deliver silence forever, which is indistinguishable from
  a broken setup. `--mic <substring>` overrides.

- **Phase 6 — song select, theme engine, options**: done. The `rungstar` binary is the game.
  Main menu, song browser with all three layouts, live search with an on-screen keyboard,
  sort picker, per-song context menu, detail panel with cover art, six options pages, audio
  previews while browsing, and the sing screen — all one window, all mouse- and
  controller-operable. Verified against a real 8,134-song library.

  `--check` starts everything a real launch does, draws one frame of every screen, and exits.
  That is what makes "the game starts" assertable on a build machine with nobody in front of
  it, and it is the first thing to run after touching a screen.

  **The sing screen is now a screen**, not a second binary: `rungstar-ui/singscreen.rs` is
  pure and `rungstar-app/session.rs` owns the clock, the microphones and the scorers. That
  split is what made multiple singers a layout question — the screen takes a slice of singers
  and splits the panel strip by its length, so one and six are the same code. `rungstar-sing`
  survives as a standalone tool for testing one song without the browser.

  **The scan runs off the main thread**, reporting progress the browser shows. Eight thousand
  songs is fourteen seconds on a cold file cache, and a frozen window is indistinguishable
  from a crash. The index is in WAL mode, so the browser reads while the scan writes.

  Still to come here: playlists in the browser, controller rebinding, a microphone setup
  screen, and the medley start point for "sing from the chorus".

  The input panel exists because silence and a dead microphone looked identical on the first
  version of this screen. It separates the three things that fail independently: no audio
  arriving at all, audio too quiet to clear the pitch gate, and audio that is fine but on the
  wrong note.

Later phases in the plan: audio/graphics/input bring-up, library index, sing screen, song
select and themes, profiles, game modes, USDB browser and downloads, editor, packaging.
