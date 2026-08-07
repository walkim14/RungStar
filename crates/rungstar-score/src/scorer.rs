//! The running score for one player on one track.

use crate::{
    fold_to_octave, BeatResult, Difficulty, LineResult, Rating, ScoreTrack, SungNote, Totals,
    MAX_LINE_RATING, MAX_NOTE_SCORE, MAX_SONG_LINE_BONUS,
};

/// Tracks one player's score as the song plays.
///
/// Drive it beat by beat with whatever pitch was detected, and tell it when a line ends. The
/// three score components are kept apart because the results screen shows them separately,
/// and as floating point because a single beat is usually worth a fraction of a point.
#[derive(Debug, Clone)]
pub struct Scorer {
    track: ScoreTrack,
    difficulty: Difficulty,
    notes: f64,
    golden: f64,
    line_bonus: f64,
    /// Note points at the start of the current line, so a line's earnings can be isolated.
    at_line_start: f64,
    sung: Vec<SungNote>,
}

impl Scorer {
    pub fn new(track: ScoreTrack, difficulty: Difficulty) -> Self {
        Self {
            track,
            difficulty,
            notes: 0.0,
            golden: 0.0,
            line_bonus: 0.0,
            at_line_start: 0.0,
            sung: Vec::new(),
        }
    }

    pub fn track(&self) -> &ScoreTrack {
        &self.track
    }

    pub fn difficulty(&self) -> Difficulty {
        self.difficulty
    }

    /// What the player has actually sung, for drawing their line against the target.
    pub fn sung_notes(&self) -> &[SungNote] {
        &self.sung
    }

    /// Score one beat against whatever pitch class was detected.
    ///
    /// `sung` is a pitch class, `0..12`, or `None` when nothing usable was heard.
    pub fn sing_beat(&mut self, beat: i32, sung: Option<i32>) -> BeatResult {
        let miss = |target, tone| BeatResult {
            target,
            target_tone: tone,
            hit: false,
            points: 0.0,
        };

        let Some(index) = self.track.note_at(beat) else {
            return miss(None, None);
        };
        let note = self.track.notes()[index];
        if !note.is_scorable() {
            return miss(Some(index), Some(note.tone));
        }
        let Some(sung) = sung else {
            return miss(Some(index), Some(note.tone));
        };

        let folded = fold_to_octave(sung, note.tone);
        // Rap notes have no pitch to match: they score on being sung at all, which is the
        // only way to make a spoken passage playable.
        let hit = !note.kind.requires_pitch_match()
            || (note.tone - folded).abs() <= self.difficulty.tolerance();

        let points = if hit {
            self.track.points_per_beat(&note)
        } else {
            0.0
        };
        if hit {
            if note.kind.is_golden() {
                self.golden += points;
            } else {
                self.notes += points;
            }
        }

        // On a hit the drawn pitch snaps to the target, which is what makes a held note read
        // as a solid bar rather than a jittery one.
        self.record_sung(beat, if hit { note.tone } else { folded }, hit);
        BeatResult {
            target: Some(index),
            target_tone: Some(note.tone),
            hit,
            points,
        }
    }

    /// Close a line and award its share of the bonus.
    ///
    /// Returns `None` for a line worth no points — one made entirely of freestyle notes. Such
    /// lines are excluded from the bonus divisor, so paying out on them would push the total
    /// past ten thousand; UltraStar Deluxe awards them a full bonus and does exactly that.
    pub fn end_line(&mut self, line_index: usize) -> Option<LineResult> {
        let line = *self.track.lines().get(line_index)?;
        let earned = (self.notes + self.golden) - self.at_line_start;
        self.at_line_start = self.notes + self.golden;

        if line.score_value == 0 || self.track.scorable_lines() == 0 {
            return None;
        }

        let total = self.track.total_score_value();
        let max_line_score = if total > 0 {
            MAX_NOTE_SCORE as f64 * line.score_value as f64 / total as f64
        } else {
            0.0
        };
        // The small allowance means a line sung nearly perfectly still counts as perfect,
        // rather than being denied by one missed beat at the edge of a syllable.
        let perfection = if max_line_score <= 2.0 {
            1.0
        } else {
            (earned / (max_line_score - 2.0)).clamp(0.0, 1.0)
        };

        let bonus = MAX_SONG_LINE_BONUS as f64 / self.track.scorable_lines() as f64 * perfection;
        self.line_bonus += bonus;

        Some(LineResult {
            perfection,
            rating: (perfection * f64::from(MAX_LINE_RATING)).round() as i32,
            bonus,
        })
    }

    /// Score components as whole numbers, reconciled so they sum to the total.
    ///
    /// Rounding each part on its own would let the three of them disagree with the headline
    /// figure by a point or two, which players notice. The largest remainders take the
    /// rounding up so the parts always add up.
    pub fn totals(&self) -> Totals {
        let parts = [self.notes, self.golden, self.line_bonus];
        let exact: f64 = parts.iter().sum();
        let total = exact.round() as i64;

        let mut ints = parts.map(|p| p.floor() as i64);
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by(|&a, &b| {
            let (fa, fb) = (parts[a] - parts[a].floor(), parts[b] - parts[b].floor());
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut deficit = total - ints.iter().sum::<i64>();
        for &i in &order {
            if deficit <= 0 {
                break;
            }
            ints[i] += 1;
            deficit -= 1;
        }

        Totals {
            notes: ints[0],
            golden: ints[1],
            line_bonus: ints[2],
            total,
        }
    }

    /// The band the current total falls into.
    pub fn rating(&self) -> Rating {
        Rating::from_score(self.totals().total)
    }

    /// Discard the score but keep the track, e.g. when restarting a song.
    pub fn reset(&mut self) {
        self.notes = 0.0;
        self.golden = 0.0;
        self.line_bonus = 0.0;
        self.at_line_start = 0.0;
        self.sung.clear();
    }

    /// Extend the current sung run, or start a new one.
    fn record_sung(&mut self, beat: i32, tone: i32, hit: bool) {
        if let Some(last) = self.sung.last_mut() {
            if last.tone == tone && last.hit == hit && last.start + last.duration == beat {
                last.duration += 1;
                return;
            }
        }
        self.sung.push(SungNote {
            start: beat,
            duration: 1,
            tone,
            hit,
        });
    }
}
