//! Turning sung pitches into points.
//!
//! The model is UltraStar Deluxe's, because a karaoke score only means anything against the
//! scores people already have. Ten thousand points are available: nine thousand spread across
//! the notes in proportion to how long they are and what they are worth, and one thousand
//! spread evenly over the lines as a bonus for singing each one well.
//!
//! Two details are easy to get wrong and both are load-bearing:
//!
//! * **Matching ignores octaves.** The sung pitch is folded to within six semitones of the
//!   target before comparison, so a bass singing an octave below the recording scores the
//!   same as a soprano singing along with it.
//! * **Every beat of a note scores separately.** A note is not hit or missed as a unit; it
//!   pays out once per beat it is held, which is what makes sustained notes worth holding.
//!
//! ```
//! use rungstar_score::{Difficulty, ScoreTrack, Scorer};
//! use rungstar_song::SongTxt;
//!
//! let song = SongTxt::parse_str(
//!     "#TITLE:t\n#ARTIST:a\n#BPM:120\n#GAP:0\n: 0 4 0 la\n: 8 4 2 la\nE\n",
//! )
//! .unwrap();
//! let track = ScoreTrack::from_lines(&song.tracks.track_1);
//! let mut scorer = Scorer::new(track, Difficulty::Medium);
//!
//! // Sing the first note perfectly: four beats, on pitch.
//! for beat in 0..4 {
//!     scorer.sing_beat(beat, Some(0));
//! }
//! assert!(scorer.totals().notes > 0);
//! ```

#![forbid(unsafe_code)]

mod scorer;
mod track;

pub use scorer::Scorer;
pub use track::{ScoreTrack, ScoredLine, ScoredNote};

use rungstar_song::NoteKind;

/// Total points available for a song.
pub const MAX_SONG_SCORE: i64 = 10_000;
/// Of that total, how much is reserved for the per-line bonus.
pub const MAX_SONG_LINE_BONUS: i64 = 1_000;
/// The remainder, spread across the notes.
pub const MAX_NOTE_SCORE: i64 = MAX_SONG_SCORE - MAX_SONG_LINE_BONUS;
/// Highest per-line rating, as shown in the pop-up after each line.
pub const MAX_LINE_RATING: i32 = 8;

/// How close the sung pitch has to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Difficulty {
    /// Two semitones either way.
    Easy,
    /// One semitone either way.
    #[default]
    Medium,
    /// Exact pitch class only.
    Hard,
}

impl Difficulty {
    /// Permitted deviation in semitones.
    pub fn tolerance(self) -> i32 {
        match self {
            Self::Easy => 2,
            Self::Medium => 1,
            Self::Hard => 0,
        }
    }
}

/// What happened on one beat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatResult {
    /// Index of the note that was expected, if any.
    pub target: Option<usize>,
    /// The pitch the target wanted, folded into the sung octave — what the UI should draw.
    pub target_tone: Option<i32>,
    /// Whether the beat scored.
    pub hit: bool,
    /// Points awarded, before rounding.
    pub points: f64,
}

/// What one completed line was worth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineResult {
    /// Fraction of the line's available points that were earned, `0.0..=1.0`.
    pub perfection: f64,
    /// The same, as the `0..=8` figure shown in the pop-up.
    pub rating: i32,
    /// Bonus points awarded.
    pub bonus: f64,
}

/// A run of beats sung at one pitch, kept for drawing the player's line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SungNote {
    pub start: i32,
    pub duration: i32,
    /// Pitch class the player actually sang.
    pub tone: i32,
    /// Whether these beats scored.
    pub hit: bool,
}

/// Score components, reconciled so the parts sum to the whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub notes: i64,
    pub golden: i64,
    pub line_bonus: i64,
    pub total: i64,
}

/// The label shown on the results screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    ToneDeaf,
    Amateur,
    Wannabe,
    Hopeful,
    RisingStar,
    LeadSinger,
    Superstar,
    Ultrastar,
}

impl Rating {
    /// Band a final score falls into.
    pub fn from_score(score: i64) -> Self {
        match score {
            ..=2009 => Self::ToneDeaf,
            2010..=4009 => Self::Amateur,
            4010..=5009 => Self::Wannabe,
            5010..=6009 => Self::Hopeful,
            6010..=7509 => Self::RisingStar,
            7510..=8509 => Self::LeadSinger,
            8510..=9009 => Self::Superstar,
            _ => Self::Ultrastar,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToneDeaf => "Tone Deaf",
            Self::Amateur => "Amateur",
            Self::Wannabe => "Wannabe",
            Self::Hopeful => "Hopeful",
            Self::RisingStar => "Rising Star",
            Self::LeadSinger => "Lead Singer",
            Self::Superstar => "Superstar",
            Self::Ultrastar => "Ultrastar",
        }
    }
}

/// Fold `sung` into the octave nearest `target`.
///
/// Both are pitch classes as far as the comparison is concerned; this brings them into the
/// same octave so the difference is the interval a listener would hear, never an octave jump.
pub fn fold_to_octave(sung: i32, target: i32) -> i32 {
    let mut sung = sung;
    while sung - target > 6 {
        sung -= 12;
    }
    while sung - target < -6 {
        sung += 12;
    }
    sung
}

/// Score weight of a note kind, matching UltraStar Deluxe's `ScoreFactor`.
fn score_factor(kind: NoteKind) -> f64 {
    f64::from(kind.score_factor())
}
