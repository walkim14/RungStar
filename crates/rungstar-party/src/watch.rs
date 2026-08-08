//! The loop that applies a challenge to a song in progress.
//!
//! One loop for all fourteen of them, fed at each line end with what everybody has scored.
//! Every rule the reference implements as its own plugin is a comparison in here.
//!
//! Nothing in this file knows about audio, drawing or time-of-day. It is handed beats, scores
//! and ratings and answers three questions: is the song over, who is out, and is the music
//! playing. That is what makes a mode testable without singing anything.

use crate::challenge::{Effects, Finish, Knockout, Length, Music};

/// Why a song stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// It ran out of notes.
    Song,
    /// Somebody reached the target.
    Reached { singer: usize, points: i32 },
    /// Everybody still in has been knocked out, or only one is left.
    Knockout,
    /// A short song reached its halfway line.
    Halfway,
}

/// One singer's standing under a challenge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Standing {
    /// Lines in a row — not in total — that scored nothing.
    pub silent_lines: usize,
    /// The average rating of the lines sung so far, `0.0..=1.0`.
    pub rating: f32,
    /// The line they went out on, if they are out.
    pub out_at: Option<usize>,
}

impl Standing {
    pub fn is_out(&self) -> bool {
        self.out_at.is_some()
    }
}

/// A song being sung under a challenge.
pub struct Watch {
    effects: Effects,
    /// Beats from the first note to the last, for the rising bar and the halfway point.
    total_beats: f64,
    standings: Vec<Standing>,
    last_score: Vec<i32>,
    lines: usize,
    ending: Option<Ending>,
    /// Deaf: whether the music is playing, and when that next changes, in seconds.
    playing: bool,
    next_change: Option<f64>,
    noise: Noise,
}

impl Watch {
    /// `seed` only matters for [`Music::Cutting`], and is taken rather than drawn so a mode
    /// can be replayed exactly in a test.
    pub fn new(effects: Effects, singers: usize, total_beats: f64, seed: u64) -> Self {
        Self {
            effects,
            total_beats: total_beats.max(1.0),
            standings: vec![
                Standing {
                    silent_lines: 0,
                    rating: 0.0,
                    out_at: None,
                };
                singers
            ],
            last_score: vec![0; singers],
            lines: 0,
            ending: None,
            playing: true,
            next_change: None,
            noise: Noise::new(seed),
        }
    }

    pub fn effects(&self) -> &Effects {
        &self.effects
    }

    pub fn standings(&self) -> &[Standing] {
        &self.standings
    }

    /// Why the song stopped, or `None` while it is still going.
    pub fn ending(&self) -> Option<Ending> {
        self.ending
    }

    pub fn is_out(&self, singer: usize) -> bool {
        self.standings.get(singer).is_some_and(Standing::is_out)
    }

    /// The bar a rating has to clear right now, `0.0..=1.0`, for the meter that shows it.
    ///
    /// Drawing this is not decoration: a rule that puts you out without showing how close you
    /// were is a rule nobody can play against.
    pub fn bar_at(&self, beat: f64) -> Option<f32> {
        match self.effects.knockout {
            Some(Knockout::Rising { full_at }) => {
                let full = (self.total_beats * full_at).max(1.0);
                Some((beat / full).clamp(0.0, 1.0) as f32)
            }
            _ => None,
        }
    }

    /// Report the end of a line.
    ///
    /// `scores` is everybody's running total and `ratings` is how well each of them sang the
    /// line just finished, `0.0..=1.0`. Both are per singer, in the same order throughout.
    pub fn line_ended(&mut self, beat: f64, scores: &[i32], ratings: &[f32]) {
        if self.ending.is_some() {
            return;
        }
        self.lines += 1;

        for (singer, standing) in self.standings.iter_mut().enumerate() {
            let score = scores.get(singer).copied().unwrap_or(0);
            let gained = score - self.last_score.get(singer).copied().unwrap_or(0);
            self.last_score[singer] = score;
            if standing.is_out() {
                continue;
            }
            // In a row rather than in total: three bad lines spread over a long song is a
            // human being, three together is somebody who has stopped singing.
            if gained > 0 {
                standing.silent_lines = 0;
            } else {
                standing.silent_lines += 1;
            }
            let rating = ratings.get(singer).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            // A running average, so one disastrous line does not end a round on its own. The
            // bar is what rises; the thing measured against it should not be that jumpy.
            let seen = self.lines as f32;
            standing.rating += (rating - standing.rating) / seen;
        }

        self.apply_knockout(beat);
        self.apply_finish(beat, scores);
    }

    fn apply_knockout(&mut self, beat: f64) {
        let line = self.lines.saturating_sub(1);
        match self.effects.knockout {
            None => {}
            Some(Knockout::Silent { lines }) => {
                // Only against the others. Everybody missing the same tricky verse is a tricky
                // verse, and a rule that ends the round there ends most rounds in the middle.
                let fewest = self
                    .standings
                    .iter()
                    .filter(|s| !s.is_out())
                    .map(|s| s.silent_lines)
                    .min()
                    .unwrap_or(0);
                for standing in &mut self.standings {
                    if !standing.is_out()
                        && standing.silent_lines >= lines
                        && standing.silent_lines > fewest
                    {
                        standing.out_at = Some(line);
                    }
                }
            }
            Some(Knockout::Rising { .. }) => {
                let Some(bar) = self.bar_at(beat) else {
                    return;
                };
                for standing in &mut self.standings {
                    if !standing.is_out() && standing.rating < bar {
                        standing.out_at = Some(line);
                    }
                }
            }
        }

        // With one singer there is nobody to be left standing against, so the song plays out;
        // with more, it stops as soon as it is decided.
        let left = self.standings.iter().filter(|s| !s.is_out()).count();
        if self.effects.knockout.is_some() && self.standings.len() > 1 && left <= 1 {
            self.ending = Some(Ending::Knockout);
        }
    }

    fn apply_finish(&mut self, beat: f64, scores: &[i32]) {
        if self.ending.is_some() {
            return;
        }
        if let Finish::AtPoints(target) = self.effects.finish {
            // At a line end rather than the instant the points land. Stopping mid-word to
            // announce a winner is worse than the half-line it costs to be tidy.
            if let Some((singer, points)) = scores
                .iter()
                .enumerate()
                .filter(|(_, points)| **points >= target)
                .max_by_key(|(_, points)| **points)
                .map(|(singer, points)| (singer, *points))
            {
                self.ending = Some(Ending::Reached { singer, points });
                return;
            }
        }
        if self.effects.length == Length::Half && beat >= self.total_beats / 2.0 {
            self.ending = Some(Ending::Halfway);
        }
    }

    /// Tell the caller the song ran out on its own.
    pub fn song_ended(&mut self) {
        if self.ending.is_none() {
            self.ending = Some(Ending::Song);
        }
    }

    /// Whether the backing track should be audible at `seconds` into the song.
    ///
    /// Called every frame. For everything but Deaf it is a constant.
    pub fn music_at(&mut self, seconds: f64) -> bool {
        let Music::Cutting {
            least_silence,
            most_silence,
            least_sound,
            most_sound,
        } = self.effects.music
        else {
            return true;
        };
        let due = *self.next_change.get_or_insert_with(|| {
            seconds + least_sound + self.noise.range(most_sound - least_sound)
        });
        if seconds < due {
            return self.playing;
        }
        self.playing = !self.playing;
        let (least, most) = if self.playing {
            (least_sound, most_sound)
        } else {
            (least_silence, most_silence)
        };
        self.next_change = Some(seconds + least + self.noise.range(most - least));
        self.playing
    }
}

/// A small deterministic generator, so a mode is reproducible in a test.
///
/// xorshift64* rather than a dependency: this picks how long a silence lasts, and the bar for
/// that is "not obviously periodic", which it clears.
struct Noise(u64);

impl Noise {
    fn new(seed: u64) -> Self {
        // One splitmix64 step first. Forcing the low bit on its own is not enough: 42 and 43
        // both become 43 and two songs seeded a beat apart cut the music at the same moments.
        // It also keeps zero away from xorshift, for which zero is a fixed point.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((z ^ (z >> 31)) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0.0..span`.
    fn range(&mut self, span: f64) -> f64 {
        if span <= 0.0 {
            return 0.0;
        }
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 * span
    }
}
