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
cargo run --release --example decode_check -p rungstar-audio -- <folder>     # audio codecs
cargo run --release --example preview_check -p rungstar-platform -- <folder> # browser previews
cargo run --release --example playback_check -p rungstar-video -- <folder>   # song videos
cargo test --workspace                              # all tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
PROPTEST_CASES=5000 cargo test -p rungstar-song     # deep property run
python tools/make-sounds.py                         # regenerate assets/sounds/*.wav
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

### Measured on the real library

8,134 songs. Numbers worth keeping because each one changed a decision:

| | |
|---|---|
| Parse failures | **0** |
| Playable (audio file resolves) | 7,865 |
| With a video | 6,233 |
| Duets | 193 |
| Audio codec | 99.9% Ogg Vorbis |
| Video codec | 88.8% AV1, 11.2% H.264 |
| Audio decode speed | ~1000x realtime |
| Video decode speed | ~20x realtime (AV1, software) |

The two codec rows are the ones that mattered. The decoder had been built with `mp3, aac,
isomp4, alac` and no Vorbis, so almost nothing could be decoded and it presented as "previews
are unreliable" rather than as a missing codec. The video row is why FFmpeg is vendored.

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

**FFmpeg 7.1** is vendored at `vendor/ffmpeg/` (Windows x64 shared, GPL build), for song video.
The deciding measurement: sampling 868 videos across a real library found **88.8% AV1** and
11.2% H.264. Rust has a practical decoder for H.264 alone, so `openh264` — which was tried and
does work — would have played one video in nine.

The version pairing is fussy and was found by elimination: FFmpeg master with `ffmpeg-next` 7
fails because FFmpeg 8 removed `avfft.h`; `ffmpeg-next` 8.1 does not match master either.
**FFmpeg 7.1 with `ffmpeg-next` 9** builds clean. Pinning to a tagged release rather than
master is also what makes the build reproducible.

`FFMPEG_DIR` lives in `.cargo/config.toml`, not in a build script: a build script cannot hand
an environment variable to another crate's build script, and `ffmpeg-sys` is the one that needs
it. Generating its bindings needs **libclang** (`winget install LLVM.LLVM`). Only the five
libraries the game links are vendored; `avfilter` and `avdevice` are not, which is why the
crate turns those features off.

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

## Microphone delay

`mic_delay_ms` shifts the whole scoring clock, so a wrong value shifts every hit — sing
perfectly, score badly, nothing on screen to say why. The 140 ms default is a guess, and a
Bluetooth speaker adds twice that on its own.

Options -> Sound -> **Measure it**, or `rungstar --calibrate [--mic <name>]`, plays a sweep and
listens for it. Four decisions in it:

- **Which gap.** The song clock comes from frames the output device has *consumed*, so SDL's
  output queue is already accounted for and counting it again would double it. What is left is
  the device's own buffer, the air, and the capture path — which is exactly what a loopback
  measures, provided the capture is drained immediately before the sweep is pushed. Those three
  lines in that order are the whole measurement.
- **A swept sine, not a click.** A click's energy is in one sample and a room loses it.
  Correlating a 300 Hz-3 kHz sweep concentrates it all back into one sharp peak.
- **Normalised correlation.** A raw correlation grows with how loud the recording is, so no
  threshold on it means anything. Divided by both energies it runs 0 to 1 whatever the level:
  noise peaks at 0.095, a sweep 12 dB *under* the noise still reaches 0.13, an ordinary room
  0.9. `CONFIDENT` is 0.12 and a test pins the gap it sits in.
- **Agreement decides, not confidence.** Five passes, median, and a majority must land within
  5 ms. Noise matches best at a different lag every time; a real delay lands in the same place
  every time. That separation is not close, where the confidence one is.

`Heard` carries the recording's peak level as well, because "the microphone recorded silence"
and "it recorded the room but not the sweep" are different faults — a dead device against
speakers pointing the wrong way — and both look like zero confidence.

**It measures every microphone a singer is assigned to**, taken from the saved assignment. The
first version measured whichever device the backend listed first, which on a laptop is usually a
headset nobody sings into: it measured the wrong hardware and said nothing about which.

**And it is a screen, stepped a few milliseconds at a time**, not a call that blocks. Five
passes across two microphones is fifteen seconds, and a game frozen that long with no meter and
no pass count cannot be told from one that has crashed. `Calibrator::tick` does one drain and
returns, so the screen names the microphone, counts the passes, and shows a live level — which
is the one thing that makes a dead device obvious while it happens rather than afterwards. One
value covers every microphone, so the median of the ones that answered is what gets set, and the
screen says so.

## Song loudness

A library assembled from a thousand uploads is not level with itself. Measured over sixty songs
from the real library: **21.6 dB between the loudest and the quietest**, mean -10.3 LUFS. That
is a factor of twelve, and it is why somebody reaches for the volume between every song.

`rungstar-audio/loudness.rs` is **EBU R128** — K-weighting, 400 ms blocks at 75% overlap, and
the two-stage gate. Peak is the wrong measure (one drum hit and a quiet track reads as loud) and
RMS is not much better (it counts bass the ear barely hears). The gate is what makes it a measure
of the *song*: without it a track with a long fade-out reads several decibels quieter than the
same track without one.

Three numbers were decided by measurement rather than by copying a standard:

- **The target is -14 LUFS**, not ReplayGain's -18 or broadcast's -23. Against a mean of -10.3,
  either of those would turn almost every song down by eight decibels and make the whole game
  quiet.
- **A boost stops where the headroom does.** The peak is stored beside the loudness, because
  loudness alone does not say how far a song can be turned up — a sparse recording can be quiet
  and still touch full scale, and a clipped chorus reads as a broken game where a quiet song
  reads as a quiet recording.
- **±12 dB, whatever the measurement says.** A file that is mostly silence measures very quiet,
  and an unclamped correction answers that with a bang.

The result: **21.6 dB becomes 2.9 dB**, every song landing between -16.9 and -14.0 LUFS.

**Measuring never happens during a scan.** It needs the whole song decoded — a fifth of a second
each, minutes across eight thousand songs, against a warm rescan that takes half of one. It
happens the first time the audio is decoded anyway, which is the first play or the first browser
preview, on its own thread. Decode runs a thousand times faster than playback, so the answer
usually lands during the same preview that paid for it. It is then kept in the song's row and,
like the play count, is never written by a scan.

## Backing tracks

Vocal removal — Demucs and friends — turns a library into a second library: the same songs, the
same folder names, one audio file in each and nothing else. Options -> Game -> **Backing tracks**
points at it, and **V** in the song list (right stick on a pad) switches between the record and
the backing track. Measured against the real library: 7,865 of 8,159 songs have one.

`rungstar-app/instrumental.rs` is the whole of it, and it is **one substitution at the moment the
audio file is opened**. The notes, the lyrics, the video, the cover, the play count and the
highscores all still come from the song's own folder — only the sound is different. Indexing the
instrumentals as songs of their own would double the library, split every song's history across
two rows, and make "the same song" something the browser has to work out; repointing one path
does none of that.

Four things had to be decided:

- **Matched by the song's own folder name**, which is what a tool writing one folder per song
  produces. Not by artist and title — those live in a header the backing-track folder does not
  have — and not by the audio file's name, because the tool renames what it writes. Inside the
  folder the song's own name wins when it is there, and otherwise the first audio file, because
  one track per folder is what these tools write.
- **One directory listing, at the root.** The browser asks whether every row has a backing track,
  so the answer has to be a hash lookup; a `stat` per song would be eight thousand of them, on
  Windows through the filter driver stack, on every scroll.
- **A song with no backing track is not in the list**, and if one is reached anyway it is refused
  rather than played. The mode makes exactly one promise, and a fall back to the recording breaks
  it quietly, one song in a hundred, in front of a room.
- **Nothing is measured for loudness while the mode is on.** An instrumental is a few decibels
  quieter than the record it came from, and the measurement is kept in the song's own row — so
  measuring one would leave the song permanently too loud once the mode was off. The song's own
  figure still applies and is close enough: taking the vocal out lowers every song by roughly the
  same amount, so they stay level with each other.

The setting is the truth and the browser mirrors it, because the same mode is on the options page
and two places each holding their own copy of one answer is how they come to disagree — the
screen asks to switch and is told the result. A folder that is not currently there (an external
drive, say) turns the mode off for now without forgetting the preference.

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
drawn with colour modulation. **Poppins is bundled** (OFL) at `assets/fonts/`, in three
weights, with **Fira Sans behind it as a coverage fallback**. A system face is still borrowed
when the folder is missing, so a source build starts, but a release ships its own.

Two things decided this. **fontdue does not apply variable-font axes**, which rules out Inter,
Outfit, Figtree and Baloo 2 — all variable-only from Google Fonts, so every weight loads as the
default instance and bold comes out identical to regular. And no face worth shipping covers a
real library: 99.94% of the text is ASCII, but the remainder is 160,908 curly quotes, 27,868
accented letters, 202 Cyrillic characters, a few hundred CJK brackets and some Hangul. So
`Face` carries a **fallback chain**, which is also the permanent fix for the class of bug that
drew the USDB star ratings as empty squares.

The chain is greedy, not exhaustive: a candidate joins it only if it draws a character from a
ten-character probe set the chain cannot already draw, and the search stops when the set is
covered. The first version added every readable font on the machine — ten faces per role, three
roles, the chosen face among them twice — parsing thirty megabytes at startup for coverage it
already had.

**Interface sounds** are six WAVs at `assets/sounds/`, generated by `tools/make-sounds.py`
rather than downloaded: the generator is reviewable in a diff and there is no licence to
honour. Pentatonic on A, because a scale with no semitone clashes lands on top of a song in any
key as percussion rather than as a wrong note.

**Menu music is synthesised at startup**, not shipped: a thirty-second loop is 2.6 MB of WAV
against a hundred lines of arithmetic that runs in 32 ms, and rendering it means the loop can be
as long as the music wants rather than as long as a repository tolerates. It is a chiptune
because the constraints of one are what make it bearable under an interface — two pulse
channels, a stepped triangle and noise, the NES's arrangement, which forces the music to stay
thin and leaves the middle of the mix free. A minor, matching the sounds. It fades out whenever
anything else is making noise, because music under a song preview is two pieces of music at once.
`cargo run --release --example menu_music -p rungstar-platform` writes it out to listen to.

Both are played from a `Chime`, which is an event and not a sound, emitted by `rungstar-ui` and
turned into audio by `rungstar-platform`. That is what keeps the UI crate free of an audio API
and makes the sounds testable without a sound card — `tests/sounds.rs` asserts that holding a
direction at the end of a list is *silent*, which is the failure worth catching. `Cursor` and
`Browser` emit for themselves, so a screen added later gets sounds for free; hovering and a
list re-sorting under the cursor deliberately do not.

**Nothing chimes during a song.** A golden note landing and a line sung well both had one, and
both were distracting: an interface sound is heard *instead of* whatever else is happening,
which is right in a menu and wrong over the thing somebody came to listen to. The screen says
both already. Starting and finishing a song still chime, because neither lands on top of any
singing.

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
- **Phase 5 — the sing screen**: done. Decode, playback, clock sync, live capture, scoring,
  notes, lyrics, up to six singers, duet layout, song video, all five lyric effects, pause
  menu and results.

  **The staff draws one line at a time with a sweeping playhead**, not a scrolling window, and
  the pitch scale is the whole song's. Both were reported as bugs and both came from the same
  mistake: a window that scrolls recomputes its scale from whatever is inside it, so a note
  changes height as the view moves and you cannot tell whether you are above or below it.

  **What the display shows has to agree with what scored.** Matching is octave-agnostic and
  the tolerance is up to two semitones, so a sung pitch is drawn folded into the target's
  octave, and a hit is drawn *on* the note rather than where the singer actually was. Drawing
  the truth put the marker beside the bubble it had just awarded points for. Runs merge on the
  note rather than the pitch for the same reason — a held note wobbles across the band and
  every beat of it counts.

  `rungstar-sing` survives as a standalone tool for one song without the browser.

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

- **Phase 7 - profiles, statistics, filters**: done. `rungstar-profile` is players, per-song
  highscores, the four statistics views and an importer for an existing `Ultrastar.db`. Plus
  the singer picker, the browser's filter panel, and a statistics wipe.

  **Who is singing is asked before the song, not after.** When more than one microphone is
  assigned, or the song is a duet, starting a song opens the singer screen first. Afterwards
  is too late: the score has nowhere to go. With exactly one profile and nobody chosen, that
  profile is the singer - otherwise somebody who made a profile and never opened the screen
  sings as "Player 1" and their score is discarded, which is indistinguishable from the
  highscore table not working.

  The panel count comes from `Session::players()` rather than from the microphone count. They
  disagree the moment somebody picks fewer singers than there are microphones, and a panel
  with no scorer behind it sits at zero for the whole song.

  **The browser filters on what the index already held.** Genre, language, decade, edition,
  folder and creator, values within a category OR and the categories AND. Values come from the
  library, and counts are of the whole library rather than of the current results - a filter
  list that empties itself as you use it cannot widen a search again. Decade is computed
  rather than stored and is the one facet ordered newest first, because a decade list is a
  timeline.

  **The song list has ends.** It used to wrap, so the last song sat above the first, holding a
  direction never arrived anywhere, and a library smaller than the view drew the same song
  twice. The slots past either end are now empty, which is what says you have reached the end
  rather than gone round again.

  **Sung marks ease along a per-singer frontier.** Scoring lands a whole beat at a time and a
  beat is a wide piece of a staff, so a mark drawn straight from the score steps across the
  screen. Two things were tried and rejected first: extending the newest mark to the playhead
  (hides the microphone delay, then lurches back when detection lands) and easing each run's
  end separately (leaves the last few units to snap on when the next run starts). The ease is
  in beats, not frames.

  Skipping the outro records the song; giving up does not. The singing is finished when the
  outro starts, so that score is a whole one; an abandoned song's is not.

- **Phase 8 - game modes**: done. `rungstar-party` holds the rules; the screens drive them.

  **UltraStar's fourteen challenge modes are Lua plugins**, one `.usdx` file each, every one
  re-implementing the same walk-the-lines-and-compare-the-scores loop against a scripting API.
  They are data, not programs: each is some combination of *hide something*, *stop early* and
  *put somebody out*. So they are a table of `Effects` read by one `Watch`, and dropping the
  scripting engine loses no mode. A plugin can only be tested by singing into a microphone; a
  `Watch` is fed beats and scores by a test in a millisecond.

  Three rules that had to be decided rather than copied:

  - **Hardcore counts silent lines in a row, and only against the other singers.** Counted
    absolutely, a hard verse everybody fluffs ends most rounds halfway through. Counted in
    total, three bad lines across a long song is a human being. A lone singer is never knocked
    out, because there is nobody to be worse than.
  - **Hold The Line measures a running average against the rising bar.** The bar is the thing
    that rises; what it measures should not also be jumpy, or every round ends on a cough.
  - **A tied party round shares the placing** - both get first, nobody gets second, which is
    what happens on a podium and what the reference leaves undefined. A drawn tournament match
    still sends somebody through: "sing it again" is not an answer at half past eleven.

  Medley finally wires up "sing from the chorus": `#MEDLEYSTARTBEAT`, else `#PREVIEWSTART`
  (somebody's answer to "the bit worth hearing", which is the same question), else a third of
  the way in. A plan narrows the song and never widens it, so a medley cannot run past `#END`.

  The jukebox reuses the sing screen with its panels off. The scorer still runs underneath -
  a second path through the session would be two things to keep right - but nothing it
  produces leaves the screen, and no score screen interrupts.

  Deliberate divergence 11: **a party screen has four stages, not ten screens.** UltraStar has
  Party Options, Player, Rounds, NewRound, Score and Win, plus four more for Tournament, and
  they disagree with each other. They are the same four questions - who is playing, what is
  being sung, how did that go, who won - so one state machine serves both the party and the
  bracket.

- **Phase 9 - USDB and downloads**: done bar an account. `rungstar-usdb` is the protocol,
  `rungstar-download` the pipeline, and the browser is a screen like any other.

  **All the markup knowledge is in `parse.rs`.** USDB is a PHP site from 2008 with no API:
  everything is `index.php` dispatched by a `link=` parameter, and when it changes that one
  file breaks. Two facts about it drive everything else:

  - **Errors arrive as HTML bodies with a 200 status.** A request for a song that does not
    exist returns a perfectly ordinary 200 whose text says so, so every response is checked
    before it is read. A client that trusts status codes reports an empty catalog.
  - **The page language follows the account**, so labels are matched against a table detected
    from the response rather than configured once.

  The transport is a trait, so the whole protocol is tested against usdb_syncer's own saved
  pages with no network and no credentials. Writing those tests found four things:

  1. An ordinary song page carries "You are not logged in" **inside its comment box**. Treating
     that as a refusal makes every song unreadable to anybody browsing signed out. Inside a
     form it is a label; outside one it is the answer.
  2. A song with no cover still gets an `<img>` pointing at a placeholder, so taking it at face
     value files a picture of the words "no cover" as the artwork.
  3. A video link copied from USDB itself is `watch?feature=player_detailpage&v=...`, and old
     comments use the 2008 `/v/` embed. Requiring `watch?v=` misses both.
  4. Folding accents before lower-casing means "Ö" never matches the "ö" arm, so "BJÖRK" stops
     finding Björk.

  **The download order is the feature.** Note file, then audio - at which point the song is
  singable and moves into the library - then artwork, then video, which is ninety per cent of
  the bytes. Everything lands in a temporary folder and moves in with one rename, so a
  download killed halfway leaves nothing for the scanner to index: half a song in the library
  is worse than none, because it indexes, opens, and somebody tries to sing it.

  **Files are remembered by a blake3 of their contents**, not by filename plus mtime plus
  source URL. Timestamps drift through cloud sync and the whole library looks changed; a hash
  also catches an interrupted download, which leaves a file that exists, opens, and plays four
  seconds of a song. That is what "repair library" checks. Folders with no sidecar are left
  alone - repairing a song nobody downloaded means deciding what it should have been.

  Rate limited with a token bucket and retried with jittered backoff. The reference sends about
  a thousand sequential POSTs at one volunteer-run box with neither.

  The USDB password goes to the **OS keyring**, never to `settings.toml`; only the username is
  a setting. A stored password that stops working is deleted rather than retried silently on
  every launch.

  **There is no keyring on a Steam Deck in Game Mode.** Windows and macOS always have one; on
  Linux it is a D-Bus Secret Service, which is a *desktop session* service, so Game Mode, a
  kiosk, a container or a TTY launch have none. The fallback keeps the **session cookie**
  instead of the password, and that is a real difference rather than a smaller version of the
  same risk: a cookie expires, is worth nothing anywhere else, and cannot be used to take the
  account over. What it costs is signing in again when the session runs out, which the screen
  says at the time rather than as a standing warning.

  The cookie is loaded first on *every* platform, so a machine with a keyring only reads the
  password when the session has actually expired. Encrypting the password with a key derived
  from the machine was considered and rejected: the key is in the binary, so it is obfuscation
  dressed as security, and the only thing it reliably defeats is the reader understanding what
  they are looking at.

  **A duet is the same song without USDB's marker.** The list page appends `[DUET]` to the
  title; the note file it hands out does not, so a library holding nine hundred of them showed
  every one as new. The marker is stripped from both sides of the name key — bracketed only,
  and wherever in the title it sits, because "The Cigarette Duet" is a title and the marker is
  not always last. Against the real library: 366 duets recognised where none were.

  Deliberate divergence 12: **a missing optional resource does not fail the song.** The
  reference refuses to deliver a singable song whose background art 404ed.

  **What still needs the account**: the two requests that fetch a note file, and therefore any
  real download. Everything above them is finished and tested.

- **Phase 10 - the song editor**: done. `rungstar-editor` is the document and the operations;
  `editorscreen.rs` is a piano roll with the waveform behind it.

  **Undo is by snapshot, not by inverse.** Every operation could carry its own undo, and
  getting one of them subtly wrong is how an editor silently corrupts an evening's work. A
  whole song is a few hundred kilobytes and two hundred of them is less than one video frame,
  so the obviously correct thing is affordable. A test applies every kind of edit, undoes it,
  and asserts the written file is unchanged to the byte.

  The rules that keep a song singable are enforced by the operations rather than left to the
  person editing: no note on top of its neighbour, no note shorter than a beat, nothing before
  beat zero. Each refusal says why, because a rule that stops an edit silently reads as a
  broken key.

  **`PITCH_RANGE` was guessed wrong first.** A symmetric ±60 refuses to transpose an ordinary
  song: measured across the 8,134-song library, real pitches run **-12 to 74**, because the
  format's number is a semitone offset from a baseline each file picks for itself. It is now
  one byte either way.

  Doubling the tempo scales every timestamp with it, so the notes stay on the same *moment* of
  the audio - changing `#BPM` alone is how a fixable half-tempo file becomes an unsingable one.
  `#GAP` is the other control and shifts the whole song instead.

  The waveform is a **peak envelope at 100 buckets a second**, computed once when the editor
  opens. Ten million samples per song against a thousand-unit staff means ten thousand reads
  per pixel per frame otherwise. Peak rather than average, or it flattens into a band as you
  zoom out and stops showing where anything starts.

  Escape with unsaved work offers to save, and Save is what the cursor starts on: somebody who
  pressed Escape almost always meant to keep it.

- **Phase 11 - packaging**: done. `packaging/` holds four ways out of the build tree.

  Windows: a portable zip assembled by `packaging/windows/portable.ps1` and an Inno Setup
  installer over the same tree. Verified by running the packaged folder standalone - all six
  DLLs beside the executable, `--check` green. Linux: a **Flatpak**, which is the only delivery
  that works on SteamOS's immutable rootfs, plus an AppImage for distributions without it.

  **Settings and songs are never inside the install directory**, portable build included. An
  uninstall or an unzip-over-the-top must not be able to take somebody's highscores with it.

  The Flatpak asks for what it needs and no more. `--socket=pulseaudio` carries capture as well
  as output; `--device=all` is required for controllers because `--device=dri` is only the GPU;
  and `--filesystem=home` is deliberately **not** requested - a karaoke game has no business
  reading everything, and a folder elsewhere is one `flatpak override` away.

  **A Steam Deck is 1600x1000 design units**, because the design space is a thousand tall and
  a Deck is 16:10 - so one design unit is 0.8 physical pixels. `tests/deck.rs` draws every
  screen at that shape and asserts two things: nothing outside the window, since a control half
  off the bottom is unreachable with no mouse, and no text below twelve physical pixels. There
  is no Deck layout, only the layout, checked at the Deck's shape.

  **Controller hints name the right buttons.** A Deck's face buttons are an Xbox pad's, but its
  shoulders are L1/L2 rather than LB/LT and its two menu buttons have no letters at all.
  Detected from `SteamDeck=1` and `/etc/os-release` rather than configured, with a setting for
  the case detection cannot cover.

  **No font is committed.** `assets/fonts/` is where a packaged build looks first, and the
  binary is dropped in at packaging time: a megabyte nobody reviews in a diff, and picking one
  is a licensing decision. Without it the game borrows a system face and says so.

Later phases in the plan: extras.
