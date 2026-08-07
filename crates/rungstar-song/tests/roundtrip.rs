//! Property-based round-trip and normalisation checks.
//!
//! The fixtures pin known-difficult files; these cover the space between them. Note that the
//! invariant is not literal input equality — parsing normalises, giving a lyric-less note a
//! `~`, dropping a ` [DUET]` title suffix, merging repeated headers — so what is asserted is
//! that writing a parsed song and reading it back changes nothing.

use proptest::prelude::*;
use rungstar_song::{FixOptions, SongTxt, Warnings};

/// Lyric fragments, including the leading and trailing spaces that join syllables.
fn lyric() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9 \u{e4}\u{f6}\u{fc}\u{df}\u{e9}'~-]{0,10}").unwrap()
}

/// A free-text header value. Colons are allowed: only the first one separates key from value.
fn header_value() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9 :,.\u{e4}\u{f6}\u{fc}'-]{1,16}").unwrap()
}

/// A note without its absolute position: kind, gap before it, duration, pitch, lyric.
type NoteSpec = (char, i32, i32, i32, String);

fn note_spec() -> impl Strategy<Value = NoteSpec> {
    (
        prop::sample::select(vec![':', '*', 'F', 'R', 'G']),
        1i32..20,
        0i32..16,
        -40i32..40,
        lyric(),
    )
}

/// A whole song file. Note order is randomised so the repair passes have to cope with it.
fn song_text() -> impl Strategy<Value = String> {
    (
        header_value(),
        header_value(),
        1u32..600,
        -5000i64..60000,
        prop::option::of(header_value()),
        prop::option::of(header_value()),
        prop::collection::vec(prop::collection::vec(note_spec(), 1..4), 1..5),
        any::<bool>(),
    )
        .prop_map(
            |(title, artist, bpm, gap, genre, language, lines, shuffle)| {
                let mut out = format!("#TITLE:{title}\n#ARTIST:{artist}\n#BPM:{bpm}\n#GAP:{gap}\n");
                if let Some(genre) = genre {
                    out.push_str(&format!("#GENRE:{genre}\n"));
                }
                if let Some(language) = language {
                    out.push_str(&format!("#LANGUAGE:{language}\n"));
                }
                let mut cursor = 0i32;
                let count = lines.len();
                for (i, notes) in lines.into_iter().enumerate() {
                    let mut rendered = Vec::new();
                    for (kind, gap_before, duration, pitch, text) in notes {
                        let start = cursor + gap_before;
                        cursor = start + duration;
                        rendered.push(format!("{kind} {start} {duration} {pitch} {text}"));
                    }
                    // Some real files list a line's notes out of order; make sure that path runs.
                    if shuffle && rendered.len() > 1 {
                        rendered.reverse();
                    }
                    for line in rendered {
                        out.push_str(&line);
                        out.push('\n');
                    }
                    if i + 1 < count {
                        cursor += 1;
                        out.push_str(&format!("- {cursor}\n"));
                    }
                }
                out.push_str("E\n");
                out
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Writing a parsed song and parsing it again must produce an identical song.
    #[test]
    fn write_then_parse_is_identity(text in song_text()) {
        let Ok(song) = SongTxt::parse_str(&text) else { return Ok(()) };
        let written = song.to_string();
        let reparsed = SongTxt::parse_str(&written)
            .map_err(|e| TestCaseError::fail(format!("rewritten song failed to parse: {e}")))?;
        prop_assert_eq!(&reparsed, &song, "round trip changed the song\n--- written ---\n{}", written);
        prop_assert_eq!(reparsed.to_string(), written);
    }

    /// Normalisation must be a fixed point: applying it twice changes nothing the second time.
    #[test]
    fn fixing_is_idempotent(text in song_text()) {
        let Ok(mut song) = SongTxt::parse_str(&text) else { return Ok(()) };
        let options = FixOptions::default();
        song.fix(&options, &mut Warnings::new());
        let once = song.to_string();

        let mut again = SongTxt::parse_str(&once)
            .map_err(|e| TestCaseError::fail(format!("fixed song failed to parse: {e}")))?;
        again.fix(&options, &mut Warnings::new());
        prop_assert_eq!(again.to_string(), once);
    }

    /// The same must hold for the UltraStar line-break style.
    #[test]
    fn fixing_is_idempotent_usdx_style(text in song_text()) {
        let Ok(mut song) = SongTxt::parse_str(&text) else { return Ok(()) };
        let options = FixOptions::usdx_style();
        song.fix(&options, &mut Warnings::new());
        let once = song.to_string();

        let mut again = SongTxt::parse_str(&once).unwrap();
        again.fix(&options, &mut Warnings::new());
        prop_assert_eq!(again.to_string(), once);
    }

    /// Normalisation must leave the song in a state the engine can rely on.
    #[test]
    fn fixing_establishes_the_engine_invariants(text in song_text()) {
        let Ok(mut song) = SongTxt::parse_str(&text) else { return Ok(()) };
        song.fix(&FixOptions::default(), &mut Warnings::new());

        prop_assert_eq!(song.tracks.start(), 0, "song must start on beat zero");
        prop_assert!(song.bpm().value() > 200.0, "tempo should have been raised: {}", song.bpm());

        for track in song.tracks.all_tracks() {
            let notes: Vec<_> = track.iter().flat_map(|l| l.notes.iter()).collect();
            for pair in notes.windows(2) {
                let (current, next) = (pair[0], pair[1]);
                prop_assert!(
                    next.start - (current.start + current.duration) >= 1,
                    "notes at {} and {} are not separated by a beat",
                    current.start,
                    next.start
                );
            }
        }
    }

    /// Beat and time conversions must invert each other.
    #[test]
    fn beat_and_time_are_inverse(bpm in 1.0f64..1000.0, gap in -5000i64..60000, beat in 0.0f64..5000.0) {
        let text = format!("#TITLE:t\n#ARTIST:a\n#BPM:{bpm}\n#GAP:{gap}\n: 0 1 0 x\nE\n");
        let song = SongTxt::parse_str(&text).unwrap();
        let secs = song.beat_to_time(beat);
        prop_assert!((song.time_to_beat(secs) - beat).abs() < 1e-6);
    }
}

/// Regression: notes listed out of order used to leave the song shifted off beat zero.
///
/// `fix_first_timestamp` took the *first listed* note as the song's origin, but the pass that
/// reorders overlapping notes runs later and could then expose an earlier one. Taking the
/// true minimum instead fixes it. The reference implementation still has this behaviour.
#[test]
fn out_of_order_notes_still_land_on_beat_zero() {
    let text = "#TITLE:a\n#ARTIST:A\n#BPM:1\n#GAP:0\n: 643 0 0 x\n: 0 0 0 y\nE\n";
    let options = FixOptions::default();

    let mut song = SongTxt::parse_str(text).unwrap();
    song.fix(&options, &mut Warnings::new());
    let once = song.to_string();

    assert_eq!(song.tracks.start(), 0);
    assert!(song.tracks.track_1[0]
        .notes
        .windows(2)
        .all(|w| w[0].start <= w[1].start));

    let mut again = SongTxt::parse_str(&once).unwrap();
    again.fix(&options, &mut Warnings::new());
    assert_eq!(again.to_string(), once);
}

/// Regression: a lyric-less syllable used to accumulate a space on every normalisation pass.
///
/// The final syllable of a line is given a trailing space so lines concatenate cleanly. When
/// that syllable had no lyric it still got one, which the next pass then read as a leading
/// space and migrated backwards, and so on. Empty stays empty now.
#[test]
fn whitespace_only_lyrics_do_not_drift() {
    let text = "#TITLE:a\n#ARTIST:A\n#BPM:300\n#GAP:0\n: 0 1 0 x\nF 4 1 0 \nE\n";
    let options = FixOptions::default();

    let mut song = SongTxt::parse_str(text).unwrap();
    song.fix(&options, &mut Warnings::new());
    let once = song.to_string();

    let mut again = SongTxt::parse_str(&once).unwrap();
    again.fix(&options, &mut Warnings::new());
    assert_eq!(again.to_string(), once);
}
