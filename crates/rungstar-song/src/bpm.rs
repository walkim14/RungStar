//! Tempo and the beat/time conversions the whole engine hangs off.

use std::fmt;

/// Beats per minute, stored exactly as written in the file.
///
/// The UltraStar format has a long-standing quirk: the value in `#BPM` is **a quarter** of the
/// rate the note grid actually advances at. UltraStar Deluxe stores `bpm * 4` internally and
/// every timing calculation uses that. We keep the file value (so writing round-trips) and
/// expose [`Bpm::grid_rate`] for the multiplied one.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Bpm(f64);

/// Below this, a song is assumed to have been authored at half tempo.
///
/// Matches usdb_syncer's `BPM_THRESHOLD`.
pub const BPM_THRESHOLD: f64 = 200.0;

/// UltraStar Deluxe rejects songs slower than this outright.
pub const MIN_BPM: f64 = 1.0;

impl Bpm {
    pub fn new(value: f64) -> Self {
        Self(value)
    }

    /// The raw `#BPM` value.
    pub fn value(self) -> f64 {
        self.0
    }

    /// The rate the beat grid actually advances at, i.e. `#BPM * 4`.
    pub fn grid_rate(self) -> f64 {
        self.0 * 4.0
    }

    pub fn is_usable(self) -> bool {
        self.0 >= MIN_BPM && self.0.is_finite()
    }

    /// Parse a `#BPM` value, tolerating a decimal comma.
    pub fn parse(text: &str) -> Option<Self> {
        text.trim().replace(',', ".").parse::<f64>().ok().map(Self)
    }

    /// Duration of `beats` beats, in seconds.
    pub fn beats_to_secs(self, beats: f64) -> f64 {
        beats / self.grid_rate() * 60.0
    }

    pub fn beats_to_ms(self, beats: f64) -> f64 {
        self.beats_to_secs(beats) * 1000.0
    }

    /// How many beats fit in `secs` seconds, as a real number.
    pub fn secs_to_beats(self, secs: f64) -> f64 {
        secs * self.grid_rate() / 60.0
    }

    /// How many whole beats fit in `secs` seconds, truncated toward zero.
    ///
    /// The truncation is deliberate: it mirrors usdb_syncer's `int(...)` so the YASS
    /// line-break fix produces byte-identical output.
    pub fn secs_to_beats_trunc(self, secs: f64) -> i32 {
        self.secs_to_beats(secs) as i32
    }

    /// Absolute playback time of a beat, accounting for the song's `#GAP`.
    pub fn beat_to_time(self, beat: f64, gap_ms: f64) -> f64 {
        gap_ms / 1000.0 + self.beats_to_secs(beat)
    }

    /// Inverse of [`Bpm::beat_to_time`].
    pub fn time_to_beat(self, secs: f64, gap_ms: f64) -> f64 {
        self.secs_to_beats(secs - gap_ms / 1000.0)
    }

    /// Is this tempo low enough to be a half-tempo transcription?
    pub fn is_too_low(self) -> bool {
        self.0 <= BPM_THRESHOLD
    }

    /// Double the tempo until it clears [`BPM_THRESHOLD`], returning the factor applied.
    ///
    /// Note timings must be multiplied by the same factor to stay in sync.
    pub fn make_large_enough(&mut self) -> i32 {
        // Guard against a zero or non-finite tempo, which would spin forever below.
        if self.0 <= 0.0 || !self.0.is_finite() {
            return 1;
        }
        let mut factor = 1;
        while self.0 * f64::from(factor) <= BPM_THRESHOLD {
            factor *= 2;
        }
        self.0 *= f64::from(factor);
        factor
    }
}

impl fmt::Display for Bpm {
    /// Render the way UltraStar tools do: two decimal places at most, no trailing zeros.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rounded = (self.0 * 100.0).round() / 100.0;
        let mut s = format!("{rounded:.2}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        f.write_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_strips_trailing_zeros() {
        assert_eq!(Bpm::new(250.0).to_string(), "250");
        assert_eq!(Bpm::new(187.5).to_string(), "187.5");
        assert_eq!(Bpm::new(123.456).to_string(), "123.46");
        assert_eq!(Bpm::new(100.10).to_string(), "100.1");
        assert_eq!(Bpm::new(0.0).to_string(), "0");
    }

    #[test]
    fn parse_accepts_decimal_comma() {
        assert_eq!(Bpm::parse("383,33").unwrap().value(), 383.33);
        assert_eq!(Bpm::parse(" 250 ").unwrap().value(), 250.0);
        assert!(Bpm::parse("nonsense").is_none());
    }

    #[test]
    fn beat_grid_is_quadruple_the_file_value() {
        let bpm = Bpm::new(60.0);
        assert_eq!(bpm.grid_rate(), 240.0);
        // 240 grid-beats per minute => 4 per second => one beat is 0.25s.
        assert!((bpm.beats_to_secs(4.0) - 1.0).abs() < 1e-12);
        assert!((bpm.secs_to_beats(1.0) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn beat_to_time_applies_gap() {
        let bpm = Bpm::new(60.0);
        assert!((bpm.beat_to_time(4.0, 500.0) - 1.5).abs() < 1e-12);
        assert!((bpm.time_to_beat(1.5, 500.0) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn make_large_enough_doubles_past_threshold() {
        // 50 -> 100 -> 200 is still <= 200, so it has to double once more.
        let mut bpm = Bpm::new(50.0);
        assert_eq!(bpm.make_large_enough(), 8);
        assert_eq!(bpm.value(), 400.0);

        // Already fast enough: untouched.
        let mut fast = Bpm::new(250.0);
        assert_eq!(fast.make_large_enough(), 1);
        assert_eq!(fast.value(), 250.0);

        // Degenerate tempo must not hang.
        let mut zero = Bpm::new(0.0);
        assert_eq!(zero.make_large_enough(), 1);
    }
}
