//! End-to-end scoring, driven by a synthetic performance.
//!
//! This is how scoring gets verified without a microphone: build a song, decide what the
//! singer does on every beat, run it through the real scoring path, and check the number that
//! falls out. A perfect performance has to come to exactly ten thousand — that is the whole
//! contract, and every other rule is a way of dividing it up.

use rungstar_score::{
    fold_to_octave, Difficulty, Rating, ScoreTrack, ScoredNote, Scorer, MAX_SONG_SCORE,
};
use rungstar_song::SongTxt;

fn track_of(body: &str) -> ScoreTrack {
    let text = format!("#TITLE:t\n#ARTIST:a\n#BPM:300\n#GAP:0\n{body}E\n");
    let song = SongTxt::parse_str(&text).expect("fixture song should parse");
    ScoreTrack::from_lines(&song.tracks.track_1)
}

/// Play a whole track, asking `singer` what pitch class is sung on each note.
///
/// `None` means nothing usable was heard on that beat.
fn play(
    track: ScoreTrack,
    difficulty: Difficulty,
    singer: impl Fn(&ScoredNote, i32) -> Option<i32>,
) -> Scorer {
    let reference = track.clone();
    let mut scorer = Scorer::new(track, difficulty);
    for (index, line) in reference.lines().iter().enumerate() {
        for beat in line.start..line.end {
            let sung = reference
                .note_at(beat)
                .map(|i| reference.notes()[i])
                .and_then(|note| singer(&note, beat));
            scorer.sing_beat(beat, sung);
        }
        scorer.end_line(index);
    }
    scorer
}

/// Sings every note exactly.
fn perfectly(note: &ScoredNote, _beat: i32) -> Option<i32> {
    Some(note.tone.rem_euclid(12))
}

#[test]
fn a_perfect_performance_scores_exactly_ten_thousand() {
    let track = track_of(": 0 4 0 la\n* 8 4 3 la\n- 14\n: 16 4 7 la\nR 24 4 0 rap\n");
    let scorer = play(track, Difficulty::Medium, perfectly);
    let totals = scorer.totals();

    assert_eq!(totals.total, MAX_SONG_SCORE);
    assert_eq!(
        totals.notes + totals.golden + totals.line_bonus,
        totals.total
    );
    assert_eq!(
        totals.line_bonus, 1_000,
        "the whole line bonus should be earned"
    );
    assert_eq!(scorer.rating(), Rating::Ultrastar);
}

#[test]
fn singing_nothing_scores_nothing() {
    let track = track_of(": 0 4 0 la\n- 6\n: 8 4 3 la\n");
    let scorer = play(track, Difficulty::Medium, |_, _| None);
    assert_eq!(scorer.totals().total, 0);
    assert_eq!(scorer.rating(), Rating::ToneDeaf);
}

#[test]
fn golden_notes_are_worth_double() {
    // Equal lengths, one golden: the golden note takes two thirds of the note pool.
    let track = track_of(": 0 4 0 a\n* 8 4 0 b\n");
    let scorer = play(track, Difficulty::Medium, perfectly);
    let totals = scorer.totals();
    assert_eq!(totals.notes, 3_000);
    assert_eq!(totals.golden, 6_000);
    assert_eq!(totals.total, MAX_SONG_SCORE);
}

#[test]
fn every_held_beat_pays_out_separately() {
    // Two notes, one four times as long: it earns four times as much.
    let track = track_of(": 0 8 0 long\n: 10 2 0 short\n");
    let scorer = play(track, Difficulty::Medium, perfectly);
    assert_eq!(scorer.totals().notes, 9_000);

    // Sing only the short one and the split shows.
    let track = track_of(": 0 8 0 long\n: 10 2 0 short\n");
    let partial = play(track, Difficulty::Medium, |note, _| {
        (note.start == 10).then(|| note.tone.rem_euclid(12))
    });
    assert_eq!(
        partial.totals().notes,
        1_800,
        "two of ten beats, of the 9000 pool"
    );
}

#[test]
fn matching_ignores_octaves() {
    // The note is written two octaves up; the singer produces the same pitch class.
    let track = track_of(": 0 4 24 a\n");
    let scorer = play(track, Difficulty::Hard, |_, _| Some(0));
    assert_eq!(
        scorer.totals().notes,
        9_000,
        "same pitch class, different octave"
    );

    assert_eq!(fold_to_octave(0, 24), 24);
    assert_eq!(fold_to_octave(11, 0), -1, "folded to the nearer side");
}

#[test]
fn difficulty_sets_how_far_off_still_counts() {
    for (difficulty, semitones_off, should_score) in [
        (Difficulty::Easy, 2, true),
        (Difficulty::Easy, 3, false),
        (Difficulty::Medium, 1, true),
        (Difficulty::Medium, 2, false),
        (Difficulty::Hard, 0, true),
        (Difficulty::Hard, 1, false),
    ] {
        let track = track_of(": 0 4 0 a\n");
        let scorer = play(track, difficulty, |_, _| Some(semitones_off));
        let scored = scorer.totals().notes > 0;
        assert_eq!(
            scored, should_score,
            "{difficulty:?} at {semitones_off} semitones off should score: {should_score}"
        );
    }
}

#[test]
fn rap_notes_score_on_presence_not_pitch() {
    let track = track_of("R 0 4 0 rap\n");
    // Wildly wrong pitch, still counts, because there is no pitch to be wrong about.
    let scorer = play(track, Difficulty::Hard, |_, _| Some(6));
    assert_eq!(scorer.totals().notes, 9_000);

    // Silence still fails: something has to be sung.
    let track = track_of("R 0 4 0 rap\n");
    let silent = play(track, Difficulty::Hard, |_, _| None);
    assert_eq!(silent.totals().notes, 0);
}

#[test]
fn freestyle_notes_never_score() {
    let track = track_of("F 0 4 0 free\n: 8 4 0 real\n");
    let scorer = play(track, Difficulty::Medium, perfectly);
    // The freestyle note contributes nothing to the pool, so the real note takes all of it.
    assert_eq!(scorer.totals().notes, 9_000);
}

#[test]
fn a_freestyle_only_line_cannot_inflate_the_total() {
    // UltraStar Deluxe leaves such lines out of the bonus divisor but still pays them a full
    // bonus, which pushes the total past ten thousand. It must not.
    let track = track_of("F 0 4 0 free\n- 6\n: 8 4 0 a\n- 14\n: 16 4 0 b\n");
    let scorer = play(track, Difficulty::Medium, perfectly);
    let totals = scorer.totals();
    assert_eq!(totals.total, MAX_SONG_SCORE);
    assert_eq!(totals.line_bonus, 1_000);
}

#[test]
fn a_half_sung_line_earns_a_proportional_bonus() {
    let track = track_of(": 0 4 0 a\n: 8 4 0 b\n- 14\n: 16 4 0 c\n: 24 4 0 d\n");
    // Sing the first line only.
    let scorer = play(track, Difficulty::Medium, |note, _| {
        (note.start < 14).then(|| note.tone.rem_euclid(12))
    });
    let totals = scorer.totals();
    assert_eq!(totals.notes, 4_500, "half the notes");
    assert_eq!(totals.line_bonus, 500, "one of two lines earned its bonus");
    assert_eq!(totals.total, 5_000);
}

#[test]
fn line_ratings_run_from_zero_to_eight() {
    let track = track_of(": 0 8 0 a\n");
    let reference = track.clone();
    let mut scorer = Scorer::new(track, Difficulty::Medium);
    for beat in 0..8 {
        scorer.sing_beat(beat, Some(0));
    }
    let result = scorer.end_line(0).expect("a scorable line");
    assert_eq!(result.rating, 8);
    assert!((result.perfection - 1.0).abs() < 1e-9);
    assert_eq!(reference.scorable_lines(), 1);
}

#[test]
fn the_sung_line_is_recorded_for_drawing() {
    let track = track_of(": 0 4 0 a\n");
    let mut scorer = Scorer::new(track, Difficulty::Medium);
    for beat in 0..2 {
        scorer.sing_beat(beat, Some(0));
    }
    for beat in 2..4 {
        scorer.sing_beat(beat, Some(6));
    }
    let sung = scorer.sung_notes();
    assert_eq!(sung.len(), 2, "a hit run then a miss run");
    assert_eq!((sung[0].start, sung[0].duration, sung[0].hit), (0, 2, true));
    assert_eq!(
        (sung[1].start, sung[1].duration, sung[1].hit),
        (2, 2, false)
    );
}

#[test]
fn rating_bands_match_the_published_thresholds() {
    for (score, expected) in [
        (0, Rating::ToneDeaf),
        (2_009, Rating::ToneDeaf),
        (2_010, Rating::Amateur),
        (4_010, Rating::Wannabe),
        (5_010, Rating::Hopeful),
        (6_010, Rating::RisingStar),
        (7_510, Rating::LeadSinger),
        (8_510, Rating::Superstar),
        (9_010, Rating::Ultrastar),
        (10_000, Rating::Ultrastar),
    ] {
        assert_eq!(Rating::from_score(score), expected, "score {score}");
    }
}
