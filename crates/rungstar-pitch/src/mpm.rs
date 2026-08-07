//! McLeod pitch method — the improved detector.
//!
//! Where CAMDF answers "which of these 49 semitones fits best", this answers "what is the
//! fundamental frequency", and says how confident it is. It works from the normalised square
//! difference function:
//!
//! ```text
//! n(tau) = 2 * sum(x[j] * x[j+tau]) / sum(x[j]^2 + x[j+tau]^2)
//! ```
//!
//! which is bounded to `-1..=1` regardless of loudness. Peaks mark candidate periods. Taking
//! the *first* peak within 90% of the tallest is what avoids the octave errors CAMDF makes:
//! a voice an octave up correlates just as well at twice the period, and the naive choice of
//! the tallest peak picks the wrong one about as often as not.
//!
//! Parabolic interpolation around the chosen peak recovers sub-sample precision, so vibrato
//! and slightly-flat singing are visible instead of being quantised away.

use crate::{Detection, ANALYSIS_WINDOW};

/// Samples used per detection.
///
/// Half the capture window: enough for two periods of the lowest note, and the inner loop
/// cost is quadratic in this, so the smaller usable size is worth taking.
const MPM_WINDOW: usize = 2048;

/// A peak must reach this fraction of the tallest to be accepted, favouring the earliest.
const PEAK_THRESHOLD: f32 = 0.9;

/// Below this clarity the window is treated as unvoiced — breath, noise, a backing track.
const MIN_CLARITY: f32 = 0.55;

#[derive(Debug, Clone)]
pub struct Mpm {
    sample_rate: u32,
    min_lag: usize,
    max_lag: usize,
    /// The window, DC-removed and scaled to roughly -1..=1 so f32 accumulation stays exact.
    window: Vec<f32>,
    /// Running sum of squares, so the normalising term costs O(1) per lag instead of O(n).
    squares: Vec<f32>,
    nsdf: Vec<f32>,
}

impl Mpm {
    pub fn new(sample_rate: u32) -> Self {
        let lowest = crate::halftone_to_frequency(0);
        let highest = crate::halftone_to_frequency(crate::HALFTONE_COUNT as i32 - 1);
        let min_lag = (f64::from(sample_rate) / highest).floor().max(2.0) as usize;
        let max_lag = ((f64::from(sample_rate) / lowest).ceil() as usize).min(MPM_WINDOW * 3 / 4);
        Self {
            sample_rate,
            min_lag,
            max_lag,
            window: vec![0.0; MPM_WINDOW],
            squares: vec![0.0; MPM_WINDOW + 1],
            nsdf: vec![0.0; max_lag + 1],
        }
    }

    /// Detect the fundamental in the most recent [`MPM_WINDOW`] samples.
    ///
    /// `write` is the ring buffer's next-write index, which is also its oldest sample.
    pub fn detect(
        &mut self,
        ring: &[i16; ANALYSIS_WINDOW],
        write: usize,
        volume: f32,
    ) -> Option<Detection> {
        self.load_window(ring, write);
        self.compute_nsdf();

        let lag = self.pick_peak()?;
        let (period, clarity) = parabolic_peak(&self.nsdf, lag);
        if clarity < MIN_CLARITY || period <= 0.0 {
            return None;
        }

        let frequency = self.sample_rate as f32 / period;
        let (halftone, _exact) = crate::frequency_to_halftone(frequency)?;
        Some(Detection {
            halftone,
            pitch_class: halftone.rem_euclid(12),
            frequency: Some(frequency),
            clarity: clarity.clamp(0.0, 1.0),
            volume,
        })
    }

    /// Copy the newest samples out of the ring in chronological order, removing any DC bias
    /// and scaling to unit range.
    ///
    /// A DC offset — common on cheap USB capture hardware — inflates the correlation at every
    /// lag and flattens the peaks. Scaling keeps the f32 sums below the point where they
    /// would start losing bits.
    fn load_window(&mut self, ring: &[i16; ANALYSIS_WINDOW], write: usize) {
        let start = (write + ANALYSIS_WINDOW - MPM_WINDOW) % ANALYSIS_WINDOW;
        let mut mean = 0.0f32;
        for (i, slot) in self.window.iter_mut().enumerate() {
            let sample = f32::from(ring[(start + i) % ANALYSIS_WINDOW]);
            *slot = sample;
            mean += sample;
        }
        mean /= MPM_WINDOW as f32;

        let mut running = 0.0f32;
        self.squares[0] = 0.0;
        for i in 0..MPM_WINDOW {
            let value = (self.window[i] - mean) / 32_768.0;
            self.window[i] = value;
            running += value * value;
            self.squares[i + 1] = running;
        }
    }

    fn compute_nsdf(&mut self) {
        let total_energy = self.squares[MPM_WINDOW];
        self.nsdf[0] = 1.0;
        for lag in 1..=self.max_lag {
            let overlap = MPM_WINDOW - lag;
            let correlation = dot(&self.window[..overlap], &self.window[lag..lag + overlap]);
            // Energy over both halves of the comparison, from the prefix sums.
            let energy = self.squares[overlap] + (total_energy - self.squares[lag]);
            self.nsdf[lag] = if energy > 0.0 {
                2.0 * correlation / energy
            } else {
                0.0
            };
        }
    }

    /// The first peak tall enough to be believed.
    ///
    /// The scan starts at lag one, not at the shortest detectable period: the broad lobe
    /// around lag zero has to be walked past in full, and for a high note that lobe reaches
    /// beyond the shortest period. Starting the walk later swallows the real peak and reports
    /// the octave below.
    fn pick_peak(&self) -> Option<usize> {
        let mut peaks: Vec<usize> = Vec::new();
        let mut lag = 1;

        while lag <= self.max_lag && self.nsdf[lag] > 0.0 {
            lag += 1;
        }
        while lag <= self.max_lag {
            while lag <= self.max_lag && self.nsdf[lag] <= 0.0 {
                lag += 1;
            }
            if lag > self.max_lag {
                break;
            }
            // Take the tallest point of this positive lobe.
            let mut best = lag;
            while lag <= self.max_lag && self.nsdf[lag] > 0.0 {
                if self.nsdf[lag] > self.nsdf[best] {
                    best = lag;
                }
                lag += 1;
            }
            peaks.push(best);
        }

        let tallest = peaks
            .iter()
            .map(|&p| self.nsdf[p])
            .fold(f32::NEG_INFINITY, f32::max);
        if !tallest.is_finite() || tallest <= 0.0 {
            return None;
        }
        let cutoff = tallest * PEAK_THRESHOLD;
        let chosen = peaks.into_iter().find(|&p| self.nsdf[p] >= cutoff)?;
        // A period shorter than the highest detectable note means the input is out of range;
        // the next peak up would be a subharmonic, which is worse than saying nothing.
        (chosen >= self.min_lag).then_some(chosen)
    }
}

/// Fit a parabola through a peak and its neighbours, returning the refined position and height.
///
/// The true period rarely lands exactly on a sample, and without this the reported pitch
/// steps in visible jumps at higher notes, where one sample of lag is a large interval.
fn parabolic_peak(values: &[f32], index: usize) -> (f32, f32) {
    if index == 0 || index + 1 >= values.len() {
        return (index as f32, values[index]);
    }
    let (left, mid, right) = (values[index - 1], values[index], values[index + 1]);
    let denominator = 2.0 * (2.0 * mid - left - right);
    if denominator.abs() < f32::EPSILON {
        return (index as f32, mid);
    }
    let offset = (right - left) / denominator;
    let height = mid - 0.25 * (left - right) * offset;
    (index as f32 + offset, height)
}

/// Dot product with several accumulators.
///
/// Floating-point addition is not associative, so a plain `sum()` has to stay sequential and
/// the compiler cannot vectorise it. Splitting into independent lanes gives it permission,
/// and this loop is where nearly all of the detector's time goes.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    const LANES: usize = 8;
    let mut acc = [0.0f32; LANES];
    let mut chunks = a.chunks_exact(LANES).zip(b.chunks_exact(LANES));
    for (x, y) in &mut chunks {
        for lane in 0..LANES {
            acc[lane] += x[lane] * y[lane];
        }
    }
    let mut total: f32 = acc.iter().sum();
    let offset = a.len() - a.len() % LANES;
    for i in offset..a.len() {
        total += a[i] * b[i];
    }
    total
}
