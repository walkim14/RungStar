//! Working out which note somebody is singing.
//!
//! A capture stream arrives as interleaved PCM; one [`Analyzer`] per player keeps a rolling
//! window of their channel and, on demand, reports the semitone they are closest to.
//!
//! Two algorithms are available:
//!
//! * [`Algorithm::Camdf`] reproduces UltraStar Deluxe exactly — a circular average magnitude
//!   difference function evaluated at the 49 semitones from C2 to C6. It is cheap, and its
//!   quirks are what two decades of high scores were set against.
//! * [`Algorithm::Mpm`] is the McLeod pitch method with parabolic interpolation. It reports a
//!   continuous frequency and a clarity figure, so it resolves vibrato and near-miss pitches
//!   that CAMDF quantises away, and it does not suffer CAMDF's octave errors.
//!
//! Detection is deliberately separate from scoring: this crate answers "what pitch is this",
//! and [`rungstar-score`] decides what that is worth.

#![forbid(unsafe_code)]

mod camdf;
mod mpm;

pub use camdf::Camdf;
pub use mpm::Mpm;

/// Length of the rolling analysis window, in samples.
///
/// A power of two so the circular index arithmetic is a mask, and long enough to hold three
/// periods of the lowest detectable note at 44.1 kHz.
pub const ANALYSIS_WINDOW: usize = 4096;

/// Number of semitones the detector can report, C2 to C6 inclusive.
pub const HALFTONE_COUNT: usize = 49;

/// Index of A4 within the semitone range, i.e. the reference pitch.
pub const REFERENCE_INDEX: i32 = 33;

/// Concert pitch of A4, in hertz.
pub const REFERENCE_FREQ: f64 = 440.0;

/// Volume gate presets, as a fraction of full scale.
///
/// Anything quieter is treated as silence. The eight steps match UltraStar Deluxe's options
/// screen so a familiar setting behaves the same way.
pub const THRESHOLD_PRESETS: [f32; 8] = [0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.40, 0.60];

/// Which detector to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Algorithm {
    /// UltraStar Deluxe's circular AMDF. Quantised to semitones, no confidence figure.
    Camdf,
    /// McLeod pitch method: continuous frequency, clarity figure, fewer octave errors.
    #[default]
    Mpm,
}

/// Input gain applied before analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicBoost {
    #[default]
    Off,
    Plus6dB,
    Plus12dB,
    Plus18dB,
}

impl MicBoost {
    /// Linear gain. The steps are powers of two so the multiply is exact.
    pub fn gain(self) -> i32 {
        match self {
            Self::Off => 1,
            Self::Plus6dB => 2,
            Self::Plus12dB => 4,
            Self::Plus18dB => 8,
        }
    }
}

/// How the analyzer is set up. Everything here is per-player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalyzerConfig {
    pub sample_rate: u32,
    pub algorithm: Algorithm,
    /// Fraction of full scale below which input counts as silence.
    pub threshold: f32,
    pub boost: MicBoost,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            algorithm: Algorithm::default(),
            threshold: THRESHOLD_PRESETS[1],
            boost: MicBoost::default(),
        }
    }
}

/// What the detector heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    /// Semitone index within C2..C6, i.e. `0..HALFTONE_COUNT`.
    pub halftone: i32,
    /// Pitch class, `0..12`. This is what scoring compares, since matching ignores octaves.
    pub pitch_class: i32,
    /// Estimated fundamental, when the algorithm can produce one.
    pub frequency: Option<f32>,
    /// How periodic the window was, `0.0..=1.0`. CAMDF reports a derived approximation.
    pub clarity: f32,
    /// Peak level of the window, `0.0..=1.0`.
    pub volume: f32,
}

/// Semitone index nearest a frequency, and the exact (fractional) index.
///
/// Returns `None` for frequencies outside the detectable range.
pub fn frequency_to_halftone(frequency: f32) -> Option<(i32, f32)> {
    if frequency <= 0.0 || !frequency.is_finite() {
        return None;
    }
    let exact = (f64::from(frequency) / REFERENCE_FREQ).log2() * 12.0 + f64::from(REFERENCE_INDEX);
    let nearest = exact.round();
    if nearest < 0.0 || nearest >= HALFTONE_COUNT as f64 {
        return None;
    }
    Some((nearest as i32, exact as f32))
}

/// Frequency of a semitone index.
pub fn halftone_to_frequency(halftone: i32) -> f64 {
    REFERENCE_FREQ * 2.0_f64.powf(f64::from(halftone - REFERENCE_INDEX) / 12.0)
}

/// A rolling analysis window for one player's microphone channel.
///
/// Samples are pushed in as they are captured; [`Analyzer::detect`] runs whenever a result is
/// wanted, which is decoupled from both the capture block size and the frame rate.
pub struct Analyzer {
    config: AnalyzerConfig,
    /// Circular buffer of the most recent [`ANALYSIS_WINDOW`] samples.
    ring: Box<[i16; ANALYSIS_WINDOW]>,
    /// Where the next sample goes; also the index of the oldest sample.
    write: usize,
    /// How many samples have been seen, capped at the window length.
    filled: usize,
    camdf: Camdf,
    mpm: Mpm,
}

impl Analyzer {
    pub fn new(config: AnalyzerConfig) -> Self {
        Self {
            camdf: Camdf::new(config.sample_rate),
            mpm: Mpm::new(config.sample_rate),
            config,
            ring: Box::new([0; ANALYSIS_WINDOW]),
            write: 0,
            filled: 0,
        }
    }

    pub fn config(&self) -> &AnalyzerConfig {
        &self.config
    }

    /// Change settings. The sample rate change rebuilds the lag tables.
    pub fn set_config(&mut self, config: AnalyzerConfig) {
        if config.sample_rate != self.config.sample_rate {
            self.camdf = Camdf::new(config.sample_rate);
            self.mpm = Mpm::new(config.sample_rate);
        }
        self.config = config;
    }

    /// Append captured samples, applying mic boost with saturation.
    ///
    /// Boost is applied on the way in rather than during analysis so it costs nothing per
    /// detection, and so the level shown on a VU meter matches what the detector sees.
    pub fn push(&mut self, samples: &[i16]) {
        let gain = self.config.boost.gain();
        for &sample in samples {
            let boosted = if gain == 1 {
                sample
            } else {
                (i32::from(sample) * gain).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
            };
            self.ring[self.write] = boosted;
            self.write = (self.write + 1) % ANALYSIS_WINDOW;
        }
        self.filled = (self.filled + samples.len()).min(ANALYSIS_WINDOW);
    }

    /// Whether enough audio has arrived for a meaningful result.
    pub fn is_ready(&self) -> bool {
        self.filled >= ANALYSIS_WINDOW
    }

    /// Peak level of the current window, `0.0..=1.0`.
    pub fn volume(&self) -> f32 {
        let peak = self
            .ring
            .iter()
            .map(|s| i32::from(*s).unsigned_abs())
            .max()
            .unwrap_or(0);
        // i16::MIN has no positive counterpart, so normalise against its magnitude.
        peak as f32 / 32_768.0
    }

    /// Detect the pitch of the current window.
    ///
    /// Returns `None` when the window is too quiet or too unlike a pitched sound, which is
    /// the common case: most of the time nobody is singing into a given microphone.
    pub fn detect(&mut self) -> Option<Detection> {
        if !self.is_ready() {
            return None;
        }
        let volume = self.volume();
        if volume < self.config.threshold {
            return None;
        }
        match self.config.algorithm {
            Algorithm::Camdf => self.camdf.detect(&self.ring, volume),
            Algorithm::Mpm => self.mpm.detect(&self.ring, self.write, volume),
        }
    }

    /// Clear the window, e.g. when restarting a song.
    pub fn reset(&mut self) {
        self.ring.fill(0);
        self.write = 0;
        self.filled = 0;
    }
}

impl std::fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Analyzer")
            .field("config", &self.config)
            .field("filled", &self.filled)
            .finish_non_exhaustive()
    }
}
