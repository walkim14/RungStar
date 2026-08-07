//! Notes, line breaks, lines and tracks — the singable body of a song file.

use std::fmt;

use crate::error::Warnings;

/// What kind of note this is, which decides how it scores and whether it has a pitch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteKind {
    /// `:` — an ordinary note.
    Regular,
    /// `*` — a golden note, worth double.
    Golden,
    /// `F` — freestyle: displayed, never scored.
    Freestyle,
    /// `R` — rap: scored on presence, not on pitch.
    Rap,
    /// `G` — golden rap: presence-scored and worth double.
    GoldenRap,
}

impl NoteKind {
    pub fn from_char(c: u8) -> Option<Self> {
        Some(match c {
            b':' => Self::Regular,
            b'*' => Self::Golden,
            b'F' => Self::Freestyle,
            b'R' => Self::Rap,
            b'G' => Self::GoldenRap,
            _ => return None,
        })
    }

    pub fn as_char(self) -> char {
        match self {
            Self::Regular => ':',
            Self::Golden => '*',
            Self::Freestyle => 'F',
            Self::Rap => 'R',
            Self::GoldenRap => 'G',
        }
    }

    /// Whether the note's pitch is meaningful. Rap and freestyle notes carry a pitch field
    /// in the file, but nothing reads it.
    pub fn has_pitch(self) -> bool {
        matches!(self, Self::Regular | Self::Golden)
    }

    pub fn is_golden(self) -> bool {
        matches!(self, Self::Golden | Self::GoldenRap)
    }

    /// Whether hitting this note requires singing the right pitch.
    ///
    /// Rap notes score on presence alone, which is why they bypass the tolerance check.
    pub fn requires_pitch_match(self) -> bool {
        matches!(self, Self::Regular | Self::Golden)
    }

    /// Score weight, matching UltraStar Deluxe's `ScoreFactor`.
    pub fn score_factor(self) -> u32 {
        match self {
            Self::Freestyle => 0,
            Self::Regular | Self::Rap => 1,
            Self::Golden | Self::GoldenRap => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid note: '{0}'")]
pub struct InvalidNote(pub String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid line break: '{0}'")]
pub struct InvalidLineBreak(pub String);

/// One syllable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub kind: NoteKind,
    /// First beat of the note.
    pub start: i32,
    /// Length in beats. Zero-length notes exist in the wild and are tolerated.
    pub duration: i32,
    /// Semitone relative to the song's own baseline; only meaningful when
    /// [`NoteKind::has_pitch`].
    pub pitch: i32,
    /// Lyric fragment. Leading and trailing spaces are significant — they are how syllables
    /// join into words — so this is never trimmed during parsing.
    pub text: String,
}

impl Note {
    /// Parse one note line.
    ///
    /// The accepted shape is `<kind>[:] <start> <duration> <pitch>[ <text>]`, with runs of
    /// spaces allowed between fields. A stray `:` directly after the kind character is
    /// tolerated because some editors emit `*:` for golden notes.
    pub fn parse(value: &str) -> Result<Self, InvalidNote> {
        let invalid = || InvalidNote(value.to_owned());
        let b = value.as_bytes();

        let kind = b
            .first()
            .copied()
            .and_then(NoteKind::from_char)
            .ok_or_else(invalid)?;
        let mut i = 1;
        if b.get(i) == Some(&b':') {
            i += 1;
        }

        i = skip_spaces_required(b, i).ok_or_else(invalid)?;
        let (start, next) = parse_int(b, i, true).ok_or_else(invalid)?;
        i = skip_spaces_required(b, next).ok_or_else(invalid)?;
        // Duration is unsigned in the grammar: a negative duration makes the line invalid
        // rather than being clamped.
        let (duration, next) = parse_int(b, i, false).ok_or_else(invalid)?;
        i = skip_spaces_required(b, next).ok_or_else(invalid)?;
        let (pitch, next) = parse_int(b, i, true).ok_or_else(invalid)?;
        i = next;

        let raw_text = if i == b.len() {
            ""
        } else if b[i] == b' ' {
            // Exactly one space separates the pitch from the lyric; anything beyond it
            // belongs to the lyric.
            &value[i + 1..]
        } else {
            return Err(invalid());
        };

        let mut text = raw_text.to_owned();
        // A scored note with no lyric would render as a gap, so it gets the conventional
        // "hold" marker instead.
        if kind != NoteKind::Freestyle && text.trim().is_empty() {
            text.insert(0, '~');
        }
        // A bare hyphen would be indistinguishable from a line break to some parsers.
        if text.trim() == "-" {
            text = text.replace('-', "~");
        }

        Ok(Self {
            kind,
            start,
            duration,
            pitch,
            text,
        })
    }

    /// One beat past the last beat of the note.
    pub fn end(&self) -> i32 {
        self.start + self.duration
    }

    /// Number of empty beats between this note and `other`.
    pub fn gap(&self, other: &Note) -> i32 {
        other.start - self.end()
    }

    /// Move the start later, shortening the note to keep its end fixed (minimum length 1).
    pub fn shift_start(&mut self, beats: i32) {
        self.start += beats;
        self.duration = (self.duration - beats).max(1);
    }

    pub fn shorten(&mut self, beats: i32) {
        self.duration = (self.duration - beats).max(1);
    }

    pub fn swap_timings(&mut self, other: &mut Note) {
        std::mem::swap(&mut self.start, &mut other.start);
        std::mem::swap(&mut self.duration, &mut other.duration);
    }

    pub fn score_value(&self) -> i64 {
        i64::from(self.duration) * i64::from(self.kind.score_factor())
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {}",
            self.kind.as_char(),
            self.start,
            self.duration,
            self.pitch,
            self.text
        )
    }
}

/// The `-` marker separating two lines of lyrics.
///
/// Most files carry a single beat (when the previous line disappears). The two-value form
/// additionally says when the next line appears; UltraStar Deluxe only reads the second value
/// in relative mode, but other editors emit it generally, so it is preserved either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineBreak {
    pub previous_line_out_time: i32,
    pub next_line_in_time: Option<i32>,
}

impl LineBreak {
    /// Parse a `-` line, returning the break and any trailing content crammed onto the same
    /// line (some files inline the next note after the break).
    pub fn parse(value: &str) -> Result<(Self, Option<&str>), InvalidLineBreak> {
        let invalid = || InvalidLineBreak(value.to_owned());
        let b = value.as_bytes();
        if b.first() != Some(&b'-') {
            return Err(invalid());
        }
        let mut i = skip_spaces(b, 1);
        let (out, next) = parse_int(b, i, true).ok_or_else(invalid)?;
        i = skip_spaces(b, next);
        let in_time = match parse_int(b, i, true) {
            Some((v, next)) => {
                i = next;
                Some(v)
            }
            None => None,
        };
        i = skip_spaces(b, i);
        let rest = if i < b.len() { Some(&value[i..]) } else { None };
        Ok((
            Self {
                previous_line_out_time: out,
                next_line_in_time: in_time,
            },
            rest,
        ))
    }

    pub fn shift(&mut self, offset: i32) {
        self.previous_line_out_time += offset;
        if let Some(t) = self.next_line_in_time.as_mut() {
            *t += offset;
        }
    }

    pub fn multiply(&mut self, factor: i32) {
        self.previous_line_out_time *= factor;
        if let Some(t) = self.next_line_in_time.as_mut() {
            *t *= factor;
        }
    }
}

impl fmt::Display for LineBreak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.next_line_in_time {
            Some(t) => write!(f, "- {} {}", self.previous_line_out_time, t),
            None => write!(f, "- {}", self.previous_line_out_time),
        }
    }
}

/// One displayed line of lyrics: its notes plus the break that ends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub notes: Vec<Note>,
    /// `None` marks the final line of a track.
    pub line_break: Option<LineBreak>,
}

impl Line {
    /// Consume notes until the line is terminated by a `-` break, a `P2` marker, an `E`, or
    /// the end of the file.
    ///
    /// Unparseable lines are reported and skipped rather than aborting: real song files
    /// routinely contain stray text, and dropping the song over it would be worse than
    /// dropping the line.
    pub fn parse(cursor: &mut LineCursor<'_>, warnings: &mut Warnings) -> Self {
        let mut notes = Vec::new();
        let mut line_break = None;
        let mut terminated = false;

        while let Some(raw) = cursor.next() {
            let txt = raw.trim_start();
            if matches!(txt.trim_end(), "E" | "P2") {
                terminated = true;
                break;
            }
            if txt.starts_with('-') {
                match LineBreak::parse(txt) {
                    Ok((lb, rest)) => {
                        line_break = Some(lb);
                        if let Some(rest) = rest {
                            cursor.push_front(rest);
                        }
                        terminated = true;
                        break;
                    }
                    Err(err) => {
                        warnings.warn(err.to_string());
                        continue;
                    }
                }
            }
            match Note::parse(txt) {
                Ok(note) => {
                    if note.duration == 0 {
                        warnings.warn(format!("zero-length note: '{txt}'"));
                    }
                    notes.push(note);
                }
                Err(err) => warnings.warn(err.to_string()),
            }
        }

        if !terminated {
            warnings.warn("unterminated line");
        }
        Self { notes, line_break }
    }

    /// Whether this is the last line of its track.
    pub fn is_last(&self) -> bool {
        self.line_break.is_none()
    }

    /// Start beat of the line, i.e. of its first note.
    ///
    /// # Panics
    /// Panics if the line has no notes. Stored lines always have at least one.
    pub fn start(&self) -> i32 {
        self.notes[0].start
    }

    /// End beat of the line, i.e. the end of its last note.
    ///
    /// # Panics
    /// Panics if the line has no notes.
    pub fn end(&self) -> i32 {
        self.notes[self.notes.len() - 1].end()
    }

    pub fn shift(&mut self, offset: i32) {
        for note in &mut self.notes {
            note.start += offset;
        }
        if let Some(lb) = self.line_break.as_mut() {
            lb.shift(offset);
        }
    }

    pub fn multiply(&mut self, factor: i32) {
        for note in &mut self.notes {
            note.start *= factor;
            note.duration *= factor;
        }
        if let Some(lb) = self.line_break.as_mut() {
            lb.multiply(factor);
        }
    }

    /// The line's lyrics with syllable markers removed.
    pub fn text(&self) -> String {
        self.notes.iter().map(|n| n.text.replace('~', "")).collect()
    }

    /// Total scoring weight of this line, i.e. `Σ duration × score_factor`.
    pub fn score_value(&self) -> i64 {
        self.notes.iter().map(Note::score_value).sum()
    }

    /// A line worth no points — usually entirely freestyle — is excluded from the line bonus.
    pub fn is_empty_sentence(&self) -> bool {
        self.score_value() == 0
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, note) in self.notes.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{note}")?;
        }
        if let Some(lb) = &self.line_break {
            if !self.notes.is_empty() {
                f.write_str("\n")?;
            }
            write!(f, "{lb}")?;
        }
        Ok(())
    }
}

/// The note body of a song: one track, or two for a duet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracks {
    pub track_1: Vec<Line>,
    pub track_2: Option<Vec<Line>>,
}

impl Tracks {
    /// Parse both player tracks. Returns `None` if there are no notes at all.
    pub fn parse(cursor: &mut LineCursor<'_>, warnings: &mut Warnings) -> Option<Self> {
        let track_1 = player_lines(cursor, warnings);
        if track_1.is_empty() {
            return None;
        }
        let track_2 = player_lines(cursor, warnings);
        let track_2 = if track_2.is_empty() {
            None
        } else {
            Some(track_2)
        };
        Some(Self { track_1, track_2 })
    }

    pub fn is_duet(&self) -> bool {
        self.track_2.is_some()
    }

    pub fn all_tracks(&self) -> impl Iterator<Item = &Vec<Line>> {
        std::iter::once(&self.track_1).chain(self.track_2.iter())
    }

    pub fn all_tracks_mut(&mut self) -> impl Iterator<Item = &mut Vec<Line>> {
        std::iter::once(&mut self.track_1).chain(self.track_2.iter_mut())
    }

    pub fn all_lines(&self) -> impl Iterator<Item = &Line> {
        self.all_tracks().flat_map(|t| t.iter())
    }

    pub fn all_lines_mut(&mut self) -> impl Iterator<Item = &mut Line> {
        self.all_tracks_mut().flat_map(|t| t.iter_mut())
    }

    pub fn all_notes(&self) -> impl Iterator<Item = &Note> {
        self.all_lines().flat_map(|l| l.notes.iter())
    }

    pub fn all_notes_mut(&mut self) -> impl Iterator<Item = &mut Note> {
        self.all_lines_mut().flat_map(|l| l.notes.iter_mut())
    }

    /// First beat of the song.
    ///
    /// This is the earliest note anywhere, not the first one listed. Files do exist whose
    /// notes are written out of order, and taking the first listed one would put the song's
    /// origin in the wrong place.
    pub fn start(&self) -> i32 {
        self.all_notes().map(|n| n.start).min().unwrap_or(0)
    }

    /// Last beat of the song, i.e. the latest note end anywhere.
    pub fn end(&self) -> i32 {
        self.all_notes().map(Note::end).max().unwrap_or(0)
    }

    /// Whether the lyrics look like they were typed with caps lock on.
    ///
    /// The first letter of each line is excluded from the judgement, because it is legitimately
    /// upper case in ordinary prose — and because the capitalisation pass puts it there. Judging
    /// on every letter, as the reference implementation does, makes the two passes fight: one
    /// capitalises a line, the next sees a song with no lower-case letters left and flattens it.
    pub fn is_all_caps(&self) -> bool {
        let mut considered = 0usize;
        for line in self.all_lines() {
            let mut sentence_case_initial_skipped = false;
            for (index, note) in line.notes.iter().enumerate() {
                for ch in note.text.chars() {
                    if !ch.is_alphabetic() {
                        continue;
                    }
                    // Only the first note of a line carries the capitalised initial.
                    if index == 0 && !sentence_case_initial_skipped {
                        sentence_case_initial_skipped = true;
                        continue;
                    }
                    if ch.is_lowercase() {
                        return false;
                    }
                    considered += 1;
                }
            }
        }
        // With nothing but line initials there is no evidence either way, so leave it alone.
        considered > 0
    }

    /// Total scoring weight of a track, i.e. UltraStar Deluxe's `Tracks[x].ScoreValue`.
    pub fn score_value(&self, track: usize) -> i64 {
        self.track(track)
            .map_or(0, |lines| lines.iter().map(Line::score_value).sum())
    }

    pub fn track(&self, index: usize) -> Option<&Vec<Line>> {
        match index {
            0 => Some(&self.track_1),
            1 => self.track_2.as_ref(),
            _ => None,
        }
    }

    pub fn track_count(&self) -> usize {
        if self.track_2.is_some() {
            2
        } else {
            1
        }
    }
}

impl fmt::Display for Tracks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.track_2 {
            Some(track_2) => {
                f.write_str("P1\n")?;
                write_lines(f, &self.track_1)?;
                f.write_str("\nP2\n")?;
                write_lines(f, track_2)?;
            }
            None => write_lines(f, &self.track_1)?,
        }
        f.write_str("\nE")
    }
}

fn write_lines(f: &mut fmt::Formatter<'_>, lines: &[Line]) -> fmt::Result {
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            f.write_str("\n")?;
        }
        write!(f, "{line}")?;
    }
    Ok(())
}

/// Reads one player's block of lines, stopping at the track or file terminator.
fn player_lines(cursor: &mut LineCursor<'_>, warnings: &mut Warnings) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    // Consume a leading `P1`/`P2` marker if present. No note kind starts with `P`, so this
    // cannot swallow real content.
    if cursor.peek().is_some_and(|l| l.starts_with('P')) {
        cursor.next();
    }
    while !cursor.is_empty() {
        let line = Line::parse(cursor, warnings);
        let is_last = line.is_last();
        if !line.notes.is_empty() {
            lines.push(line);
        }
        if is_last {
            break;
        }
    }
    // A trailing break would point past the end of the track — e.g. when the final note was
    // itself unparseable and got dropped.
    if let Some(last) = lines.last_mut() {
        last.line_break = None;
    }
    lines
}

/// A rewindable cursor over the note section's lines.
///
/// The single push-back slot exists because a `-` break may carry the next note inline; the
/// remainder is pushed back to be read as the first line of the following block.
pub struct LineCursor<'a> {
    lines: &'a [&'a str],
    pos: usize,
    pending: Option<&'a str>,
}

impl<'a> LineCursor<'a> {
    pub fn new(lines: &'a [&'a str]) -> Self {
        Self {
            lines,
            pos: 0,
            pending: None,
        }
    }

    pub fn peek(&self) -> Option<&'a str> {
        self.pending.or_else(|| self.lines.get(self.pos).copied())
    }

    pub fn is_empty(&self) -> bool {
        self.peek().is_none()
    }

    fn push_front(&mut self, line: &'a str) {
        self.pending = Some(line);
    }
}

impl<'a> Iterator for LineCursor<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if let Some(p) = self.pending.take() {
            return Some(p);
        }
        let line = self.lines.get(self.pos)?;
        self.pos += 1;
        Some(line)
    }
}

/// Skip zero or more ASCII spaces.
fn skip_spaces(b: &[u8], mut i: usize) -> usize {
    while b.get(i) == Some(&b' ') {
        i += 1;
    }
    i
}

/// Skip one or more ASCII spaces; `None` if there was not at least one.
fn skip_spaces_required(b: &[u8], i: usize) -> Option<usize> {
    let next = skip_spaces(b, i);
    (next > i).then_some(next)
}

/// Parse an integer, returning it with the index just past it.
///
/// Values outside `i32` are rejected rather than wrapped — a beat that large is corrupt data.
fn parse_int(b: &[u8], start: usize, allow_sign: bool) -> Option<(i32, usize)> {
    let mut i = start;
    let negative = if allow_sign && b.get(i) == Some(&b'-') {
        i += 1;
        true
    } else {
        false
    };
    let digits_start = i;
    let mut value: i64 = 0;
    while let Some(&c) = b.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        value = value.checked_mul(10)?.checked_add(i64::from(c - b'0'))?;
        if value > i64::from(i32::MAX) + 1 {
            return None;
        }
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    let signed = if negative { -value } else { value };
    i32::try_from(signed).ok().map(|v| (v, i))
}
