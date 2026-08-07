//! The precomputed form of a track that scoring runs against.

use rungstar_song::{Line, NoteKind};

/// One note, flattened out of its line and stripped to what scoring needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoredNote {
    pub start: i32,
    pub duration: i32,
    /// The pitch as written. Compared against the sung pitch class after octave folding.
    pub tone: i32,
    pub kind: NoteKind,
    /// Which line this note belongs to.
    pub line: usize,
}

impl ScoredNote {
    pub fn end(self) -> i32 {
        self.start + self.duration
    }

    /// Whether this note can be sung for points at all.
    ///
    /// Freestyle notes are shown but never scored, and a note with no length has no beats to
    /// award anything on.
    pub fn is_scorable(self) -> bool {
        self.kind != NoteKind::Freestyle && self.duration > 0
    }
}

/// What one line contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoredLine {
    pub score_value: i64,
    pub start: i32,
    pub end: i32,
}

/// A track prepared for scoring.
///
/// Notes are flattened into one list sorted by start beat so the note active on a given beat
/// is a binary search rather than a scan — this runs on every beat of every player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreTrack {
    notes: Vec<ScoredNote>,
    lines: Vec<ScoredLine>,
    total_score_value: i64,
    scorable_lines: usize,
}

impl ScoreTrack {
    pub fn from_lines(lines: &[Line]) -> Self {
        let mut notes = Vec::new();
        let mut scored_lines = Vec::with_capacity(lines.len());

        for (index, line) in lines.iter().enumerate() {
            for note in &line.notes {
                notes.push(ScoredNote {
                    start: note.start,
                    duration: note.duration,
                    tone: note.pitch,
                    kind: note.kind,
                    line: index,
                });
            }
            scored_lines.push(ScoredLine {
                score_value: line.score_value(),
                start: line.notes.first().map_or(0, |n| n.start),
                end: line.notes.last().map_or(0, |n| n.start + n.duration),
            });
        }

        notes.sort_by_key(|n| n.start);
        let total_score_value = scored_lines.iter().map(|l| l.score_value).sum();
        // Lines worth nothing — usually entirely freestyle — are left out of the bonus
        // divisor, so they neither earn nor dilute it.
        let scorable_lines = scored_lines.iter().filter(|l| l.score_value > 0).count();

        Self {
            notes,
            lines: scored_lines,
            total_score_value,
            scorable_lines,
        }
    }

    pub fn notes(&self) -> &[ScoredNote] {
        &self.notes
    }

    pub fn lines(&self) -> &[ScoredLine] {
        &self.lines
    }

    /// Sum of `duration × score_factor` over the whole track.
    ///
    /// This is the denominator that turns a note's length into its share of the points.
    pub fn total_score_value(&self) -> i64 {
        self.total_score_value
    }

    pub fn scorable_lines(&self) -> usize {
        self.scorable_lines
    }

    /// Index of the note covering `beat`, if any.
    pub fn note_at(&self, beat: i32) -> Option<usize> {
        let after = self.notes.partition_point(|n| n.start <= beat);
        let index = after.checked_sub(1)?;
        (beat < self.notes[index].end()).then_some(index)
    }

    /// Points one beat of this note is worth.
    pub fn points_per_beat(&self, note: &ScoredNote) -> f64 {
        if self.total_score_value == 0 {
            return 0.0;
        }
        (crate::MAX_NOTE_SCORE as f64 / self.total_score_value as f64)
            * crate::score_factor(note.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rungstar_song::SongTxt;

    fn track(body: &str) -> ScoreTrack {
        let text = format!("#TITLE:t\n#ARTIST:a\n#BPM:120\n#GAP:0\n{body}E\n");
        let song = SongTxt::parse_str(&text).unwrap();
        ScoreTrack::from_lines(&song.tracks.track_1)
    }

    #[test]
    fn score_value_weights_length_and_kind() {
        // Four beats of a normal note plus two of a golden one: 4*1 + 2*2 = 8.
        let track = track(": 0 4 0 a\n* 8 2 0 b\n");
        assert_eq!(track.total_score_value(), 8);
    }

    #[test]
    fn freestyle_notes_are_worth_nothing() {
        let track = track("F 0 4 0 a\n: 8 4 0 b\n");
        assert_eq!(track.total_score_value(), 4);
        assert!(!track.notes()[0].is_scorable());
    }

    #[test]
    fn note_lookup_covers_the_held_beats_only() {
        let track = track(": 4 3 0 a\n");
        assert_eq!(track.note_at(3), None, "before the note");
        assert_eq!(track.note_at(4), Some(0));
        assert_eq!(track.note_at(6), Some(0), "last held beat");
        assert_eq!(track.note_at(7), None, "one past the end");
    }

    #[test]
    fn a_line_of_only_freestyle_does_not_count_toward_the_bonus() {
        let track = track("F 0 4 0 a\n- 6\n: 8 4 0 b\n");
        assert_eq!(track.lines().len(), 2);
        assert_eq!(track.scorable_lines(), 1);
    }
}
