//! What can be done to a song.
//!
//! One enum rather than a method each, so the screen can bind a key to an operation, a test
//! can drive a list of them, and every one of them goes through the same undo.
//!
//! The rules that keep a song singable are enforced here rather than left to the person
//! editing. A note cannot be moved on top of the one before it, a duration cannot go to zero,
//! and a pitch cannot leave the range the format can write. An editor that lets you make an
//! unplayable song and only tells you when somebody tries to sing it is not helping.

use rungstar_song::{Line, LineBreak, Note, NoteKind};

use crate::{Editor, Selection};

/// Which end of a note, or which of a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Start,
    End,
}

/// An edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    // ---- moving about ----
    /// Move the cursor by this many notes, across line ends.
    Step(isize),
    /// Move the cursor by this many lines.
    StepLine(isize),
    /// Switch to another track of a duet.
    Track(usize),
    /// Grow or shrink the selection by this many notes.
    Extend(isize),
    /// Put the cursor exactly here.
    Put(Selection),

    // ---- the notes ----
    /// Slide the selected notes along the beat grid.
    Move(i32),
    /// Change how long the selected notes are, in beats.
    Resize(i32),
    /// Transpose the selected notes.
    Transpose(i32),
    /// Set what kind the selected notes are.
    SetKind(NoteKind),
    /// Replace the syllable of the note under the cursor.
    SetText(String),
    /// Cut the note under the cursor in two at this many beats in.
    Split(i32),
    /// Join the selected notes into one, concatenating their syllables.
    Merge,
    /// Add a note after the cursor, taking its pitch and length from it.
    Insert,
    /// Remove the selected notes.
    Delete,

    // ---- the lines ----
    /// Break the line at the cursor, so the notes from here start a new one.
    BreakLine,
    /// Join this line to the one after it.
    JoinLine,

    // ---- the song ----
    /// Shift the whole song against the audio, in milliseconds.
    Gap(i32),
    /// Change the tempo. Timestamps are scaled, so the song still lines up with the audio.
    ScaleBpm(f64),
    /// Set the tempo without touching the notes, for a file whose `#BPM` is simply wrong.
    SetBpm(f64),
    /// Mark where the medley starts or ends, at the cursor.
    Medley(Which),
    /// Clear the medley marks.
    ClearMedley,
    /// Set the preview point to the cursor.
    Preview,
}

/// The lowest and highest pitch worth allowing.
///
/// Not a format limit — the field is a plain integer — but a sanity one, and it was guessed
/// wrong first. Measured across 8,134 real songs the pitches run from **-12 to 74**: the
/// format's number is a semitone offset from the song's own baseline, and files pick that
/// baseline anywhere they like, so a symmetric ±60 refuses to transpose an ordinary song. One
/// byte either way is far past anything real and still catches a stuck key.
pub const PITCH_RANGE: std::ops::RangeInclusive<i32> = -128..=127;

/// Apply an operation. Returns whether it did anything.
pub fn apply(editor: &mut Editor, op: Op) -> bool {
    match op {
        Op::Step(by) => step(editor, by),
        Op::StepLine(by) => step_line(editor, by),
        Op::Track(track) => {
            if track >= editor.tracks() {
                return false;
            }
            editor.selection.track = track;
            editor.selection.line = 0;
            editor.selection.note = 0;
            editor.selection.run = 1;
            editor.clamp();
            true
        }
        Op::Extend(by) => {
            let notes = editor
                .lines()
                .get(editor.selection.line)
                .map_or(0, |line| line.notes.len());
            let room = notes.saturating_sub(editor.selection.note).max(1);
            let run = (editor.selection.run as isize + by).clamp(1, room as isize) as usize;
            let changed = run != editor.selection.run;
            editor.selection.run = run;
            changed
        }
        Op::Put(selection) => {
            editor.selection = selection;
            editor.clamp();
            true
        }

        Op::Move(by) => move_notes(editor, by),
        Op::Resize(by) => resize(editor, by),
        Op::Transpose(by) => transpose(editor, by),
        Op::SetKind(kind) => set_kind(editor, kind),
        Op::SetText(text) => set_text(editor, text),
        Op::Split(at) => split(editor, at),
        Op::Merge => merge(editor),
        Op::Insert => insert(editor),
        Op::Delete => delete(editor),

        Op::BreakLine => break_line(editor),
        Op::JoinLine => join_line(editor),

        Op::Gap(by) => {
            let gap = editor.song().headers.gap + i64::from(by);
            editor.song_mut().headers.gap = gap;
            true
        }
        Op::ScaleBpm(factor) => scale_bpm(editor, factor),
        Op::SetBpm(bpm) => {
            if bpm <= 0.0 {
                editor.refused = Some("a tempo has to be more than nothing".to_owned());
                return false;
            }
            editor.song_mut().headers.bpm = Some(rungstar_song::Bpm::new(bpm));
            true
        }
        Op::Medley(which) => medley(editor, which),
        Op::ClearMedley => {
            let headers = &mut editor.song_mut().headers;
            let had = headers.medleystartbeat.is_some() || headers.medleyendbeat.is_some();
            headers.medleystartbeat = None;
            headers.medleyendbeat = None;
            had
        }
        Op::Preview => {
            let Some(note) = editor.current().map(|n| n.start) else {
                return false;
            };
            let seconds = editor.seconds_at(f64::from(note));
            editor.song_mut().headers.previewstart = Some(seconds.max(0.0));
            true
        }
    }
}

// ---------------------------------------------------------------- moving about

fn step(editor: &mut Editor, by: isize) -> bool {
    if by == 0 {
        return false;
    }
    let mut line = editor.selection.line;
    let mut note = editor.selection.note as isize + by;
    let lines = editor.lines().len();
    if lines == 0 {
        return false;
    }
    // Walking off the end of a line goes to the next one rather than stopping, because a song
    // is one stream of syllables and the line breaks are punctuation.
    loop {
        let count = editor.lines()[line].notes.len() as isize;
        if note < 0 {
            if line == 0 {
                note = 0;
                break;
            }
            line -= 1;
            note += editor.lines()[line].notes.len() as isize;
            continue;
        }
        if note >= count {
            if line + 1 >= lines {
                note = (count - 1).max(0);
                break;
            }
            note -= count;
            line += 1;
            continue;
        }
        break;
    }
    let changed = line != editor.selection.line || note as usize != editor.selection.note;
    editor.selection.line = line;
    editor.selection.note = note.max(0) as usize;
    editor.selection.run = 1;
    changed
}

fn step_line(editor: &mut Editor, by: isize) -> bool {
    let lines = editor.lines().len();
    if lines == 0 {
        return false;
    }
    let line = (editor.selection.line as isize + by).clamp(0, lines as isize - 1) as usize;
    let changed = line != editor.selection.line;
    editor.selection.line = line;
    editor.selection.note = 0;
    editor.selection.run = 1;
    changed
}

// ---------------------------------------------------------------- the notes

/// The note before the selection and the one after it, across line boundaries.
///
/// Timing is a property of the whole song, not of a line: a note pushed past the end of its
/// line still collides with the first note of the next one.
fn neighbours(editor: &Editor) -> (Option<Note>, Option<Note>) {
    let mut flat: Vec<(usize, usize, Note)> = Vec::new();
    for (line_index, line) in editor.lines().iter().enumerate() {
        for (note_index, note) in line.notes.iter().enumerate() {
            flat.push((line_index, note_index, note.clone()));
        }
    }
    let first = flat
        .iter()
        .position(|(l, n, _)| *l == editor.selection.line && *n == editor.selection.note);
    let Some(first) = first else {
        return (None, None);
    };
    let last = first + editor.selection.run.max(1) - 1;
    let before = first
        .checked_sub(1)
        .and_then(|i| flat.get(i))
        .map(|(_, _, n)| n.clone());
    let after = flat.get(last + 1).map(|(_, _, n)| n.clone());
    (before, after)
}

fn move_notes(editor: &mut Editor, by: i32) -> bool {
    if by == 0 {
        return false;
    }
    let (before, after) = neighbours(editor);
    let selection = editor.selection.clone();
    let Some(line) = editor.lines_mut().get_mut(selection.line) else {
        return false;
    };
    let range = selection.notes();
    if range.end > line.notes.len() {
        return false;
    }
    let first_start = line.notes[range.start].start + by;
    let last_end = {
        let last = &line.notes[range.end - 1];
        last.start + by + last.duration
    };
    // Not over the neighbours. A note that starts before the one in front of it ends is a song
    // the scorer cannot make sense of, and the repair pass would silently reorder it later.
    if let Some(before) = before {
        if first_start < before.start + before.duration {
            editor.refused = Some("that would run into the note before it".to_owned());
            return false;
        }
    }
    if let Some(after) = after {
        if last_end > after.start {
            editor.refused = Some("that would run into the note after it".to_owned());
            return false;
        }
    }
    if first_start < 0 {
        editor.refused = Some("a song cannot start before beat zero".to_owned());
        return false;
    }
    for note in &mut line.notes[range] {
        note.start += by;
    }
    true
}

fn resize(editor: &mut Editor, by: i32) -> bool {
    if by == 0 {
        return false;
    }
    let (_, after) = neighbours(editor);
    let selection = editor.selection.clone();
    let Some(line) = editor.lines_mut().get_mut(selection.line) else {
        return false;
    };
    let range = selection.notes();
    if range.end > line.notes.len() {
        return false;
    }
    // A zero-length note is legal in the format and is silently turned into freestyle by the
    // parser, which is never what anybody meant when they were shortening a note.
    if line.notes[range.clone()]
        .iter()
        .any(|note| note.duration + by < 1)
    {
        editor.refused = Some("a note has to last at least one beat".to_owned());
        return false;
    }
    let last_end = {
        let last = &line.notes[range.end - 1];
        last.start + last.duration + by
    };
    if let Some(after) = after {
        if last_end > after.start {
            editor.refused = Some("that would run into the note after it".to_owned());
            return false;
        }
    }
    for note in &mut line.notes[range] {
        note.duration += by;
    }
    true
}

fn transpose(editor: &mut Editor, by: i32) -> bool {
    if by == 0 {
        return false;
    }
    let selection = editor.selection.clone();
    let Some(line) = editor.lines_mut().get_mut(selection.line) else {
        return false;
    };
    let range = selection.notes();
    if range.end > line.notes.len() {
        return false;
    }
    if line.notes[range.clone()]
        .iter()
        .any(|note| !PITCH_RANGE.contains(&(note.pitch + by)))
    {
        editor.refused = Some("that is off the end of the staff".to_owned());
        return false;
    }
    for note in &mut line.notes[range] {
        note.pitch += by;
    }
    true
}

fn set_kind(editor: &mut Editor, kind: NoteKind) -> bool {
    let selection = editor.selection.clone();
    let Some(line) = editor.lines_mut().get_mut(selection.line) else {
        return false;
    };
    let range = selection.notes();
    if range.end > line.notes.len() {
        return false;
    }
    for note in &mut line.notes[range] {
        note.kind = kind;
    }
    true
}

fn set_text(editor: &mut Editor, text: String) -> bool {
    let selection = editor.selection.clone();
    let Some(note) = editor
        .lines_mut()
        .get_mut(selection.line)
        .and_then(|line| line.notes.get_mut(selection.note))
    else {
        return false;
    };
    if note.text == text {
        return false;
    }
    note.text = text;
    true
}

fn split(editor: &mut Editor, at: i32) -> bool {
    let selection = editor.selection.clone();
    let Some(line) = editor.lines_mut().get_mut(selection.line) else {
        return false;
    };
    let Some(note) = line.notes.get(selection.note).cloned() else {
        return false;
    };
    // Both halves have to be worth having. Splitting a one-beat note produces two notes of
    // nothing, which the parser turns into freestyle.
    if at < 1 || at >= note.duration {
        editor.refused = Some("there is no room to split that note".to_owned());
        return false;
    }
    let mut first = note.clone();
    first.duration = at;
    let mut second = note;
    second.start += at;
    second.duration -= at;
    // The syllable stays with the first half and the second gets nothing. Splitting the text
    // in the middle guesses at where a word divides, and guessing wrong is worse than an
    // empty syllable somebody types into.
    second.text = String::new();
    line.notes[selection.note] = first;
    line.notes.insert(selection.note + 1, second);
    true
}

fn merge(editor: &mut Editor) -> bool {
    let selection = editor.selection.clone();
    if selection.run < 2 {
        editor.refused = Some("select more than one note to join them".to_owned());
        return false;
    }
    let Some(line) = editor.lines_mut().get_mut(selection.line) else {
        return false;
    };
    let range = selection.notes();
    if range.end > line.notes.len() {
        return false;
    }
    let notes: Vec<Note> = line.notes[range.clone()].to_vec();
    let mut merged = notes[0].clone();
    let last = notes.last().expect("the run is at least two");
    // The whole span, including any gap between them: joining two notes with a rest in the
    // middle should give one note that covers the rest, not one that leaves a hole.
    merged.duration = last.start + last.duration - merged.start;
    merged.text = notes.iter().map(|note| note.text.as_str()).collect();
    // Golden wins, because losing a golden note by joining it to a plain one takes points
    // away that nobody asked to lose.
    if notes.iter().any(|note| note.kind.is_golden()) {
        merged.kind = NoteKind::Golden;
    }
    line.notes.splice(range, [merged]);
    editor.selection.run = 1;
    true
}

fn insert(editor: &mut Editor) -> bool {
    let selection = editor.selection.clone();
    let (_, after) = neighbours(editor);
    let Some(line) = editor.lines_mut().get_mut(selection.line) else {
        return false;
    };
    let Some(note) = line.notes.get(selection.note).cloned() else {
        // An empty line gets a first note rather than nothing to click on.
        line.notes.push(Note {
            kind: NoteKind::Regular,
            start: 0,
            duration: 4,
            pitch: 0,
            text: String::new(),
        });
        return true;
    };
    let start = note.start + note.duration;
    let room = after.map_or(note.duration, |after| after.start - start);
    if room < 1 {
        editor.refused = Some("there is no room for a note there".to_owned());
        return false;
    }
    line.notes.insert(
        selection.note + 1,
        Note {
            kind: note.kind,
            start,
            duration: room.min(note.duration.max(1)),
            pitch: note.pitch,
            text: String::new(),
        },
    );
    editor.selection.note += 1;
    editor.selection.run = 1;
    true
}

fn delete(editor: &mut Editor) -> bool {
    let selection = editor.selection.clone();
    let lines = editor.lines_mut();
    let Some(line) = lines.get_mut(selection.line) else {
        return false;
    };
    let range = selection.notes();
    if range.end > line.notes.len() {
        return false;
    }
    line.notes.drain(range);
    // A line with nothing left in it is not a line. The parser drops empty sentences anyway,
    // so leaving one here would produce a song that does not read back as what was written.
    if line.notes.is_empty() && lines.len() > 1 {
        lines.remove(selection.line);
    }
    editor.selection.run = 1;
    editor.clamp();
    true
}

// ---------------------------------------------------------------- the lines

fn break_line(editor: &mut Editor) -> bool {
    let selection = editor.selection.clone();
    let lines = editor.lines_mut();
    let Some(line) = lines.get_mut(selection.line) else {
        return false;
    };
    if selection.note == 0 || selection.note >= line.notes.len() {
        return false;
    }
    let rest: Vec<Note> = line.notes.split_off(selection.note);
    // The break goes on the beat the new line starts, which is what the format means by it.
    let at = rest[0].start;
    let carried = line.line_break;
    line.line_break = Some(LineBreak {
        previous_line_out_time: at,
        next_line_in_time: None,
    });
    lines.insert(
        selection.line + 1,
        Line {
            notes: rest,
            line_break: carried,
        },
    );
    editor.selection.line += 1;
    editor.selection.note = 0;
    editor.selection.run = 1;
    true
}

fn join_line(editor: &mut Editor) -> bool {
    let selection = editor.selection.clone();
    let lines = editor.lines_mut();
    if selection.line + 1 >= lines.len() {
        editor.refused = Some("there is no line after this one".to_owned());
        return false;
    }
    let next = lines.remove(selection.line + 1);
    let line = &mut lines[selection.line];
    line.notes.extend(next.notes);
    line.line_break = next.line_break;
    editor.selection.run = 1;
    true
}

// ---------------------------------------------------------------- the song

fn scale_bpm(editor: &mut Editor, factor: f64) -> bool {
    if factor <= 0.0 || (factor - 1.0).abs() < f64::EPSILON {
        return false;
    }
    let bpm = editor.song().bpm().value() * factor;
    if !(1.0..=3000.0).contains(&bpm) {
        editor.refused = Some("that tempo is outside anything the format holds".to_owned());
        return false;
    }
    // Every timestamp moves with the tempo, so the notes stay on the same *moment* of the
    // audio. Changing `#BPM` alone slides the whole song against the recording, which is the
    // classic way a half-tempo file gets "fixed" into an unsingable one.
    let scale = |beat: i32| (f64::from(beat) * factor).round() as i32;
    let song = editor.song_mut();
    song.headers.bpm = Some(rungstar_song::Bpm::new(bpm));
    for track in [Some(&mut song.tracks.track_1), song.tracks.track_2.as_mut()]
        .into_iter()
        .flatten()
    {
        for line in track.iter_mut() {
            for note in &mut line.notes {
                note.start = scale(note.start);
                note.duration = scale(note.duration).max(1);
            }
            if let Some(brk) = &mut line.line_break {
                brk.previous_line_out_time = scale(brk.previous_line_out_time);
                if let Some(next) = &mut brk.next_line_in_time {
                    *next = scale(*next);
                }
            }
        }
    }
    for beat in [
        song.headers.medleystartbeat.as_mut(),
        song.headers.medleyendbeat.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        *beat = scale(*beat);
    }
    true
}

fn medley(editor: &mut Editor, which: Which) -> bool {
    let Some(note) = editor.current().cloned() else {
        return false;
    };
    let beat = match which {
        Which::Start => note.start,
        Which::End => note.start + note.duration,
    };
    let headers = &mut editor.song_mut().headers;
    match which {
        Which::Start => headers.medleystartbeat = Some(beat),
        Which::End => headers.medleyendbeat = Some(beat),
    }
    // A medley that ends before it starts is not a medley. Fixing it here rather than
    // refusing, because the fix is unambiguous and refusing means guessing which one is wrong.
    if let (Some(start), Some(end)) = (headers.medleystartbeat, headers.medleyendbeat) {
        if end <= start {
            match which {
                Which::Start => headers.medleyendbeat = None,
                Which::End => headers.medleystartbeat = None,
            }
        }
    }
    true
}
