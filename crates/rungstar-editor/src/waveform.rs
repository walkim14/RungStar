//! The picture of the audio behind the notes.
//!
//! Timing a song by ear alone means playing the same two seconds forty times. A waveform turns
//! that into looking: the consonant that starts a word is visible, and a note can be dragged
//! onto it.
//!
//! What is stored is a **peak envelope**, not samples. A four-minute song at 44.1 kHz is ten
//! million samples per channel and a staff is a thousand pixels wide, so drawing from samples
//! means reading ten thousand of them per pixel every frame. The envelope is computed once at
//! a fixed resolution and every zoom level reads from it.

/// How many buckets a second is divided into.
///
/// A hundred is 10 ms per bucket, which is finer than any note boundary matters to and still
/// only 24,000 buckets for a four-minute song — a hundred kilobytes, computed once.
pub const PER_SECOND: usize = 100;

/// The peak envelope of a song's audio.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Waveform {
    /// Loudest absolute sample in each bucket, `0.0..=1.0`.
    peaks: Vec<f32>,
    seconds: f64,
}

impl Waveform {
    /// Build an envelope from interleaved samples.
    pub fn from_samples(samples: &[i16], channels: usize, sample_rate: u32) -> Self {
        let channels = channels.max(1);
        let rate = sample_rate.max(1) as f64;
        let frames = samples.len() / channels;
        let seconds = frames as f64 / rate;
        let buckets = ((seconds * PER_SECOND as f64).ceil() as usize).max(1);
        let mut peaks = vec![0.0_f32; buckets];
        let per_bucket = (rate / PER_SECOND as f64).max(1.0);

        for (frame, chunk) in samples.chunks(channels).enumerate() {
            let bucket = ((frame as f64 / per_bucket) as usize).min(buckets - 1);
            // The loudest channel, not their average: a vocal panned to one side would
            // otherwise be drawn at half its height.
            let peak = chunk
                .iter()
                .map(|sample| (*sample as f32 / i16::MAX as f32).abs())
                .fold(0.0_f32, f32::max);
            if peak > peaks[bucket] {
                peaks[bucket] = peak;
            }
        }
        Self { peaks, seconds }
    }

    pub fn is_empty(&self) -> bool {
        self.peaks.is_empty()
    }

    pub fn seconds(&self) -> f64 {
        self.seconds
    }

    /// The loudest sample between two moments, `0.0..=1.0`.
    ///
    /// Peak rather than average across the span, because a waveform drawn from averages
    /// flattens into a band as you zoom out and stops showing where anything starts.
    pub fn peak_between(&self, from: f64, to: f64) -> f32 {
        if self.peaks.is_empty() {
            return 0.0;
        }
        let last = self.peaks.len() - 1;
        let start = ((from * PER_SECOND as f64).floor().max(0.0) as usize).min(last);
        let end = ((to * PER_SECOND as f64).ceil().max(0.0) as usize).min(last);
        self.peaks[start..=end.max(start)]
            .iter()
            .fold(0.0_f32, |a, b| a.max(*b))
    }

    /// The envelope over a span, resampled to `columns` values.
    ///
    /// What the screen actually asks for: one number per pixel of staff, however far it is
    /// zoomed in.
    pub fn columns(&self, from: f64, to: f64, columns: usize) -> Vec<f32> {
        if columns == 0 || to <= from {
            return Vec::new();
        }
        let step = (to - from) / columns as f64;
        (0..columns)
            .map(|index| {
                let at = from + step * index as f64;
                self.peak_between(at, at + step)
            })
            .collect()
    }
}
