//! Editing a song, and taking it back.
//!
//! The property that matters most is the last one: whatever is done, undoing it restores the
//! song byte for byte. An editor that corrupts somebody's work an hour into a session is worse
//! than no editor.

use rungstar_editor::ops::{Op, Which};
use rungstar_editor::{Editor, Selection, Waveform};
use rungstar_song::{NoteKind, SongTxt};

const SONG: &str = "\
#TITLE:Waterloo
#ARTIST:Abba
#MP3:audio.ogg
#BPM:300
#GAP:1000
: 0 4 60 My~
: 4 4 62 wa
: 8 4 64 ter
- 12
: 16 4 60 loo
: 20 8 62 oo
E
";

fn editor() -> Editor {
    let parsed = SongTxt::parse_bytes(SONG.as_bytes()).expect("the fixture parses");
    Editor::over(parsed.song, std::path::PathBuf::from("song.txt"))
}

fn note(editor: &Editor, line: usize, index: usize) -> rungstar_song::Note {
    editor.lines()[line].notes[index].clone()
}

// ---------------------------------------------------------------- moving about

#[test]
fn the_cursor_walks_across_line_ends() {
    // A song is one stream of syllables; the line breaks are punctuation. Stopping at the end
    // of a line means arrowing to a line and then arrowing again into it.
    let mut editor = editor();
    assert_eq!(editor.current().unwrap().text, "My~");
    for expected in ["wa", "ter", "loo", "oo"] {
        editor.apply(Op::Step(1));
        assert_eq!(editor.current().unwrap().text, expected);
    }
    // And stops at the end rather than wrapping to the beginning.
    assert!(!editor.apply(Op::Step(1)));
    assert_eq!(editor.current().unwrap().text, "oo");

    for expected in ["loo", "ter", "wa", "My~"] {
        editor.apply(Op::Step(-1));
        assert_eq!(editor.current().unwrap().text, expected);
    }
    assert!(!editor.apply(Op::Step(-1)));
}

#[test]
fn a_selection_grows_and_shrinks_inside_its_line() {
    let mut editor = editor();
    assert_eq!(editor.selection.run, 1);
    editor.apply(Op::Extend(1));
    editor.apply(Op::Extend(1));
    assert_eq!(editor.selection.run, 3, "the whole first line");
    // There is no fourth note on that line, so it stops.
    assert!(!editor.apply(Op::Extend(1)));
    editor.apply(Op::Extend(-5));
    assert_eq!(editor.selection.run, 1);
}

// ---------------------------------------------------------------- notes

#[test]
fn a_note_cannot_be_moved_on_top_of_its_neighbours() {
    // A note that starts before the one in front of it ends is a song the scorer cannot make
    // sense of, and the repair pass would silently reorder it later.
    let mut editor = editor();
    editor.apply(Op::Step(1)); // "wa", beats 4..8
    assert!(!editor.apply(Op::Move(-1)), "it ran into the note before");
    assert!(editor.refused.is_some(), "and did not say why");
    assert_eq!(note(&editor, 0, 1).start, 4);

    assert!(!editor.apply(Op::Move(1)), "it ran into the note after");
    assert_eq!(note(&editor, 0, 1).start, 4);
}

#[test]
fn a_note_moves_into_the_room_there_is() {
    let mut editor = editor();
    editor.apply(Op::Step(4)); // "oo", the last note, beats 20..28
    assert!(editor.apply(Op::Move(4)));
    assert_eq!(note(&editor, 1, 1).start, 24);
    assert!(editor.apply(Op::Move(-4)));
    assert_eq!(note(&editor, 1, 1).start, 20);
}

#[test]
fn the_song_cannot_be_moved_before_beat_zero() {
    let mut editor = editor();
    assert!(!editor.apply(Op::Move(-1)));
    assert_eq!(note(&editor, 0, 0).start, 0);
}

#[test]
fn a_note_cannot_be_shortened_to_nothing() {
    // A zero-length note is legal in the format and the parser silently turns it into
    // freestyle, which is never what anybody meant by shortening a note.
    let mut editor = editor();
    assert!(editor.apply(Op::Resize(-3)));
    assert_eq!(note(&editor, 0, 0).duration, 1);
    assert!(!editor.apply(Op::Resize(-1)));
    assert_eq!(note(&editor, 0, 0).duration, 1);
}

#[test]
fn lengthening_a_note_stops_at_the_next_one() {
    let mut editor = editor();
    assert!(!editor.apply(Op::Resize(1)), "0..4 would reach into 4..8");
    editor.apply(Op::Step(4));
    assert!(editor.apply(Op::Resize(4)), "the last note has room");
    assert_eq!(note(&editor, 1, 1).duration, 12);
}

#[test]
fn transposing_stops_at_the_ends_of_the_staff() {
    let mut editor = editor();
    // Real songs run from -12 to 74 across the whole 8,134-song library, so a symmetric
    // range of sixty would refuse to transpose an ordinary one. A byte either way is far past
    // anything real and still catches a stuck key.
    assert!(editor.apply(Op::Transpose(5)));
    assert_eq!(note(&editor, 0, 0).pitch, 65);
    assert!(!editor.apply(Op::Transpose(1000)));
    assert_eq!(note(&editor, 0, 0).pitch, 65);
}

#[test]
fn a_selection_is_transposed_together() {
    let mut editor = editor();
    editor.apply(Op::Extend(2));
    editor.apply(Op::Transpose(-12));
    assert_eq!(note(&editor, 0, 0).pitch, 48);
    assert_eq!(note(&editor, 0, 1).pitch, 50);
    assert_eq!(note(&editor, 0, 2).pitch, 52);
    assert_eq!(note(&editor, 1, 0).pitch, 60, "the next line is untouched");
}

#[test]
fn a_note_splits_in_two_and_the_syllable_stays_with_the_first() {
    // Splitting the text guesses at where a word divides, and guessing wrong is worse than an
    // empty syllable somebody types into.
    let mut editor = editor();
    assert!(editor.apply(Op::Split(2)));
    assert_eq!(editor.lines()[0].notes.len(), 4);
    assert_eq!(note(&editor, 0, 0).duration, 2);
    assert_eq!(note(&editor, 0, 0).text, "My~");
    assert_eq!(note(&editor, 0, 1).start, 2);
    assert_eq!(note(&editor, 0, 1).duration, 2);
    assert_eq!(note(&editor, 0, 1).text, "");
    assert_eq!(note(&editor, 0, 1).pitch, 60, "the pitch carries over");
}

#[test]
fn a_note_with_no_room_does_not_split() {
    let mut editor = editor();
    editor.apply(Op::Resize(-3)); // one beat long
    assert!(!editor.apply(Op::Split(1)));
    assert!(editor.refused.is_some());
}

#[test]
fn merging_covers_the_whole_span_including_the_gap() {
    // Joining two notes with a rest between them should give one note that covers the rest,
    // not one that leaves a hole in the middle of a word.
    let mut editor = editor();
    editor.apply(Op::StepLine(1));
    editor.apply(Op::Extend(1)); // "loo" 16..20 and "oo" 20..28
    assert!(editor.apply(Op::Merge));
    assert_eq!(editor.lines()[1].notes.len(), 1);
    let merged = note(&editor, 1, 0);
    assert_eq!(merged.start, 16);
    assert_eq!(merged.duration, 12);
    assert_eq!(merged.text, "loooo");
}

#[test]
fn merging_keeps_a_golden_note_golden() {
    // Losing a golden note by joining it to a plain one takes points away that nobody asked
    // to lose.
    let mut editor = editor();
    editor.apply(Op::Step(1));
    editor.apply(Op::SetKind(NoteKind::Golden));
    editor.apply(Op::Step(-1));
    editor.apply(Op::Extend(1));
    editor.apply(Op::Merge);
    assert_eq!(note(&editor, 0, 0).kind, NoteKind::Golden);
}

#[test]
fn one_note_cannot_be_merged_with_itself() {
    let mut editor = editor();
    assert!(!editor.apply(Op::Merge));
    assert!(editor.refused.is_some());
}

#[test]
fn inserting_puts_a_note_in_the_room_after_the_cursor() {
    let mut editor = editor();
    editor.apply(Op::Step(4)); // the last note
    assert!(editor.apply(Op::Insert));
    assert_eq!(editor.lines()[1].notes.len(), 3);
    let fresh = note(&editor, 1, 2);
    assert_eq!(fresh.start, 28, "right after the one before it");
    assert_eq!(fresh.pitch, 62, "the pitch of the note it came from");
    assert_eq!(fresh.text, "");
    assert_eq!(editor.selection.note, 2, "and the cursor follows it");
}

#[test]
fn inserting_where_there_is_no_room_refuses() {
    let mut editor = editor();
    // Between 0..4 and 4..8 there is nothing.
    assert!(!editor.apply(Op::Insert));
    assert!(editor.refused.is_some());
}

#[test]
fn deleting_the_last_note_of_a_line_takes_the_line_with_it() {
    // The parser drops empty sentences, so leaving one behind produces a song that does not
    // read back as what was written.
    let mut editor = editor();
    editor.apply(Op::StepLine(1));
    editor.apply(Op::Extend(1));
    assert!(editor.apply(Op::Delete));
    assert_eq!(editor.lines().len(), 1);
    assert_eq!(editor.selection.line, 0, "and the cursor comes back");
}

// ---------------------------------------------------------------- lines

#[test]
fn a_line_breaks_at_the_cursor_and_joins_again() {
    let mut editor = editor();
    editor.apply(Op::Step(1)); // "wa"
    assert!(editor.apply(Op::BreakLine));
    assert_eq!(editor.lines().len(), 3);
    assert_eq!(editor.lines()[0].notes.len(), 1);
    assert_eq!(editor.lines()[1].notes.len(), 2);
    assert_eq!(editor.selection.line, 1);
    assert_eq!(
        editor.lines()[0].line_break.unwrap().previous_line_out_time,
        4,
        "the break goes on the beat the new line starts"
    );

    editor.apply(Op::StepLine(-1));
    assert!(editor.apply(Op::JoinLine));
    assert_eq!(editor.lines().len(), 2);
    assert_eq!(editor.lines()[0].notes.len(), 3);
}

#[test]
fn a_line_cannot_be_broken_at_its_first_note() {
    let mut editor = editor();
    assert!(!editor.apply(Op::BreakLine), "that is not a break");
    assert_eq!(editor.lines().len(), 2);
}

#[test]
fn the_last_line_has_nothing_to_join_to() {
    let mut editor = editor();
    editor.apply(Op::StepLine(1));
    assert!(!editor.apply(Op::JoinLine));
    assert!(editor.refused.is_some());
}

// ---------------------------------------------------------------- the song

#[test]
fn the_gap_shifts_the_song_against_the_audio() {
    let mut editor = editor();
    assert!(editor.apply(Op::Gap(-250)));
    assert_eq!(editor.song().headers.gap, 750);
}

#[test]
fn doubling_the_tempo_scales_every_timestamp_with_it() {
    // The classic half-tempo file. Changing #BPM alone slides the whole song against the
    // recording, which is how a fixable file becomes an unsingable one.
    let mut editor = editor();
    let before = editor.seconds_at(f64::from(note(&editor, 1, 1).start));
    assert!(editor.apply(Op::ScaleBpm(2.0)));
    assert_eq!(editor.song().bpm().value(), 600.0);
    assert_eq!(note(&editor, 0, 0).start, 0);
    assert_eq!(note(&editor, 0, 1).start, 8);
    assert_eq!(note(&editor, 1, 1).start, 40);
    assert_eq!(note(&editor, 1, 1).duration, 16);
    // Every note is still at the same moment of the audio, which is the entire point.
    let after = editor.seconds_at(f64::from(note(&editor, 1, 1).start));
    assert!((before - after).abs() < 1e-9, "{before} then {after}");
    // And the line breaks moved with them.
    assert_eq!(
        editor.lines()[0].line_break.unwrap().previous_line_out_time,
        24
    );
}

#[test]
fn a_tempo_outside_anything_the_format_holds_is_refused() {
    let mut editor = editor();
    assert!(!editor.apply(Op::ScaleBpm(0.0)));
    assert!(!editor.apply(Op::ScaleBpm(-2.0)));
    assert!(!editor.apply(Op::ScaleBpm(100.0)));
    assert_eq!(editor.song().bpm().value(), 300.0);
}

#[test]
fn setting_the_tempo_alone_leaves_the_notes_where_they_are() {
    // For a file whose #BPM is simply wrong, as against one that is half what it should be.
    let mut editor = editor();
    assert!(editor.apply(Op::SetBpm(150.0)));
    assert_eq!(editor.song().bpm().value(), 150.0);
    assert_eq!(note(&editor, 0, 1).start, 4, "untouched");
}

#[test]
fn the_medley_is_marked_at_the_cursor_and_cannot_end_before_it_starts() {
    let mut editor = editor();
    editor.apply(Op::StepLine(1)); // "loo" at 16
    assert!(editor.apply(Op::Medley(Which::Start)));
    assert_eq!(editor.song().headers.medleystartbeat, Some(16));

    editor.apply(Op::Step(1)); // "oo" at 20..28
    assert!(editor.apply(Op::Medley(Which::End)));
    assert_eq!(editor.song().headers.medleyendbeat, Some(28));

    // Marking an end before the start drops the start rather than keeping a medley that runs
    // backwards. Fixed here rather than refused, because refusing means guessing which of the
    // two the person meant to change.
    editor.selection = Selection {
        track: 0,
        line: 0,
        note: 0,
        run: 1,
    };
    editor.apply(Op::Medley(Which::End)); // beat 4, before the start at 16
    assert_eq!(editor.song().headers.medleyendbeat, Some(4));
    assert_eq!(editor.song().headers.medleystartbeat, None);
}

#[test]
fn the_preview_point_is_set_in_seconds_from_the_cursor() {
    let mut editor = editor();
    editor.apply(Op::StepLine(1)); // beat 16
    assert!(editor.apply(Op::Preview));
    // gap 1000 ms plus 16 beats at 300 BPM: 1.0 + 16 * 60 / 1200 = 1.8 seconds.
    let at = editor.song().headers.previewstart.unwrap();
    assert!((at - 1.8).abs() < 1e-9, "{at}");
}

// ---------------------------------------------------------------- undo

#[test]
fn undoing_every_kind_of_edit_restores_the_song_exactly() {
    // The property the whole editor rests on. Every operation gets applied to a fresh copy and
    // taken back, and the written file has to match to the byte.
    let original = editor().song().to_string();
    let edits: Vec<Op> = vec![
        Op::Move(0),
        Op::Transpose(3),
        Op::Resize(-1),
        Op::SetKind(NoteKind::Golden),
        Op::SetText("Mine".to_owned()),
        Op::Split(2),
        Op::Insert,
        Op::Delete,
        Op::BreakLine,
        Op::JoinLine,
        Op::Gap(500),
        Op::ScaleBpm(2.0),
        Op::SetBpm(123.0),
        Op::Medley(Which::Start),
        Op::Preview,
        Op::Merge,
    ];
    for edit in edits {
        let mut editor = editor();
        // Somewhere in the middle, so operations that need a neighbour have one.
        editor.apply(Op::Step(1));
        editor.apply(Op::Extend(1));
        let did = editor.apply(edit.clone());
        if !did {
            continue;
        }
        assert!(editor.dirty(), "{edit:?} changed nothing measurable");
        assert!(editor.undo(), "{edit:?} could not be undone");
        assert_eq!(
            editor.song().to_string(),
            original,
            "undoing {edit:?} did not restore the song"
        );
        assert!(!editor.dirty(), "{edit:?} left the song looking changed");
    }
}

#[test]
fn a_long_session_undoes_all_the_way_back_and_redoes_all_the_way_forward() {
    let mut editor = editor();
    let original = editor.song().to_string();
    let mut steps = 0;
    for _ in 0..30 {
        if editor.apply(Op::Transpose(1)) {
            steps += 1;
        }
        editor.apply(Op::Step(1));
    }
    assert!(steps > 3);
    let finished = editor.song().to_string();

    while editor.undo() {}
    assert_eq!(editor.song().to_string(), original);
    while editor.redo() {}
    assert_eq!(editor.song().to_string(), finished);
}

#[test]
fn an_edit_that_does_nothing_does_not_fill_the_history() {
    // Otherwise holding a key that has stopped having an effect quietly throws away the
    // history behind it.
    let mut editor = editor();
    editor.apply(Op::Transpose(1));
    for _ in 0..300 {
        editor.apply(Op::Transpose(1000));
    }
    assert!(editor.undo());
    assert!(!editor.can_undo(), "the real edit was pushed out");
}

#[test]
fn a_new_edit_ends_the_redo_branch() {
    let mut editor = editor();
    editor.apply(Op::Transpose(1));
    editor.undo();
    assert!(editor.can_redo());
    editor.apply(Op::Gap(10));
    assert!(
        !editor.can_redo(),
        "redo would reach a song nobody has seen"
    );
}

#[test]
fn the_history_is_bounded() {
    let mut editor = editor();
    for _ in 0..(rungstar_editor::UNDO_DEPTH + 50) {
        editor.apply(Op::Gap(1));
    }
    let mut taken = 0;
    while editor.undo() {
        taken += 1;
    }
    assert_eq!(taken, rungstar_editor::UNDO_DEPTH);
}

// ---------------------------------------------------------------- files

#[test]
fn a_song_saves_atomically_and_reads_back_as_what_was_written() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("song.txt");
    std::fs::write(&path, SONG).unwrap();

    let mut editor = Editor::open(&path).unwrap();
    editor.apply(Op::Transpose(2));
    assert!(editor.dirty());
    editor.save().unwrap();
    assert!(!editor.dirty(), "saving did not settle it");

    // Nothing left behind, and what is there parses back to the same song.
    let left: Vec<String> = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["song.txt".to_owned()], "a part file survived");

    let read_back = Editor::open(&path).unwrap();
    assert_eq!(read_back.song(), editor.song());
    assert_eq!(read_back.lines()[0].notes[0].pitch, 62);
}

#[test]
fn opening_something_that_is_not_a_song_says_so() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.txt");
    std::fs::write(&path, "this is not a song").unwrap();
    let error = Editor::open(&path).unwrap_err().to_string();
    assert!(error.contains("notes.txt"), "{error}");

    let missing = Editor::open(directory.path().join("nothing.txt"))
        .unwrap_err()
        .to_string();
    assert!(missing.contains("nothing.txt"));
}

// ---------------------------------------------------------------- waveform

#[test]
fn a_waveform_is_a_peak_envelope_at_a_fixed_resolution() {
    // Drawing from samples means reading ten thousand of them per pixel every frame. The
    // envelope is computed once and every zoom level reads from it.
    let rate = 1000;
    let mut samples: Vec<i16> = vec![0; rate as usize * 2];
    // A loud moment one second in.
    samples[rate as usize + 10] = i16::MAX;
    let wave = Waveform::from_samples(&samples, 1, rate);

    assert!((wave.seconds() - 2.0).abs() < 0.01);
    assert!(wave.peak_between(0.0, 0.5) < 0.01, "silence stays silent");
    assert!(wave.peak_between(0.99, 1.05) > 0.9, "the peak is found");

    // And resampling to columns keeps it: an average would flatten it away.
    let columns = wave.columns(0.0, 2.0, 20);
    assert_eq!(columns.len(), 20);
    assert!(columns.iter().fold(0.0_f32, |a, b| a.max(*b)) > 0.9);
    assert!(columns[0] < 0.01);
}

#[test]
fn a_waveform_takes_the_louder_channel_rather_than_the_average() {
    // A vocal panned to one side would otherwise be drawn at half its height.
    let samples: Vec<i16> = vec![0, i16::MAX, 0, i16::MAX];
    let wave = Waveform::from_samples(&samples, 2, 2);
    assert!(wave.peak_between(0.0, 1.0) > 0.9);
}

#[test]
fn an_empty_waveform_answers_rather_than_panicking() {
    let wave = Waveform::default();
    assert!(wave.is_empty());
    assert_eq!(wave.peak_between(0.0, 10.0), 0.0);
    assert!(wave.columns(0.0, 1.0, 10).iter().all(|v| *v == 0.0));
    assert!(
        wave.columns(1.0, 0.0, 10).is_empty(),
        "backwards is nothing"
    );
}
