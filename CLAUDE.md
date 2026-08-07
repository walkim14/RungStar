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
| Cold scan (paid once) | 82 s |

**The cold scan is dominated by FTS5 indexing the lyrics**, not by parsing or by the
filesystem. Roughly 36 MB of lyric text across thirty thousand small documents is simply slow
to tokenise, and it is the price of being able to search by a line you half-remember. Batching
the index lookup and replacing four `stat` calls per song with one directory listing bought
21% on the warm path and only 8% on the cold one — measured, not assumed, and the earlier
guess that syscalls dominated was wrong.

If it needs to come down further, in order of expected value: skip the FTS delete for rows
being inserted rather than updated (it is a no-op on a fresh index but still a statement per
song), bulk-load the FTS table in one pass after the main insert, and consider an
external-content FTS table to stop storing the lyrics twice.

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

  The input panel exists because silence and a dead microphone looked identical on the first
  version of this screen. It separates the three things that fail independently: no audio
  arriving at all, audio too quiet to clear the pitch gate, and audio that is fine but on the
  wrong note.

Later phases in the plan: audio/graphics/input bring-up, library index, sing screen, song
select and themes, profiles, game modes, USDB browser and downloads, editor, packaging.
