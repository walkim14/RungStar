//! Circular average magnitude difference function — UltraStar Deluxe's detector.
//!
//! For each candidate semitone the window is compared against a copy of itself delayed by one
//! period of that semitone:
//!
//! ```text
//! D(tau) = (1/N) * sum over n of | x[(n + tau) mod N] - x[n] |
//! ```
//!
//! A signal that repeats with period `tau` cancels, so the smallest `D` names the pitch. It
//! is cheap — 49 lags over a 4096-sample window — and quantised to semitones by construction.
//!
//! Reproduced exactly, including the tie-break toward the lower semitone, so scores set
//! against UltraStar Deluxe remain comparable.

use crate::{Detection, ANALYSIS_WINDOW, HALFTONE_COUNT};

/// Precomputed lag table for one sample rate.
#[derive(Debug, Clone)]
pub struct Camdf {
    /// Delay in samples for each semitone, i.e. one period of it.
    delays: [usize; HALFTONE_COUNT],
}

impl Camdf {
    pub fn new(sample_rate: u32) -> Self {
        let mut delays = [0usize; HALFTONE_COUNT];
        for (index, delay) in delays.iter_mut().enumerate() {
            let frequency = crate::halftone_to_frequency(index as i32);
            *delay = (f64::from(sample_rate) / frequency).round() as usize;
        }
        Self { delays }
    }

    /// Semitone delays, lowest note first.
    pub fn delays(&self) -> &[usize; HALFTONE_COUNT] {
        &self.delays
    }

    /// Find the best-matching semitone for the window.
    ///
    /// The window is treated as circular, which makes the result independent of where the
    /// ring buffer happens to start — so no linearisation copy is needed.
    pub fn detect(&self, ring: &[i16; ANALYSIS_WINDOW], volume: f32) -> Option<Detection> {
        let mut best_index = 0usize;
        let mut best_value = f32::INFINITY;
        let mut worst_value = 0.0f32;

        for (index, &delay) in self.delays.iter().enumerate() {
            let value = circular_amd(ring, delay);
            // Strictly-less keeps the earliest (lowest) semitone on a tie.
            if value < best_value {
                best_value = value;
                best_index = index;
            }
            if value > worst_value {
                worst_value = value;
            }
        }

        // CAMDF has no native confidence figure. How far the best lag sits below the worst is
        // a serviceable stand-in: a periodic signal produces a deep minimum, noise does not.
        let clarity = if worst_value > 0.0 {
            (1.0 - best_value / worst_value).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let halftone = best_index as i32;
        Some(Detection {
            halftone,
            pitch_class: halftone.rem_euclid(12),
            frequency: None,
            clarity,
            volume,
        })
    }
}

/// Average absolute difference between the window and itself delayed by `delay` samples.
///
/// The wrap-around is handled by splitting into two contiguous runs rather than masking each
/// index. Masking forces a scalar loop; two straight slices let the compiler vectorise, which
/// is worth roughly a fivefold difference here.
fn circular_amd(ring: &[i16; ANALYSIS_WINDOW], delay: usize) -> f32 {
    let split = ANALYSIS_WINDOW - delay;
    // n in 0..split compares against n + delay, which does not wrap.
    let mut total = abs_diff_sum(&ring[delay..], &ring[..split]);
    // n in split..N wraps around to the start of the buffer.
    total += abs_diff_sum(&ring[..delay], &ring[split..]);
    total as f32 / ANALYSIS_WINDOW as f32
}

/// Sum of absolute differences, with several accumulators so the loop vectorises.
fn abs_diff_sum(a: &[i16], b: &[i16]) -> u32 {
    const LANES: usize = 16;
    let mut acc = [0u32; LANES];
    let mut chunks = a.chunks_exact(LANES).zip(b.chunks_exact(LANES));
    for (x, y) in &mut chunks {
        for lane in 0..LANES {
            acc[lane] += (i32::from(x[lane]) - i32::from(y[lane])).unsigned_abs();
        }
    }
    let mut total: u32 = acc.iter().sum();
    let offset = a.len() - a.len() % LANES;
    for i in offset..a.len() {
        total += (i32::from(a[i]) - i32::from(b[i])).unsigned_abs();
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f64, sample_rate: u32) -> Box<[i16; ANALYSIS_WINDOW]> {
        let mut buffer = Box::new([0i16; ANALYSIS_WINDOW]);
        for (n, sample) in buffer.iter_mut().enumerate() {
            let t = n as f64 / f64::from(sample_rate);
            *sample = ((t * frequency * std::f64::consts::TAU).sin() * 16_000.0) as i16;
        }
        buffer
    }

    #[test]
    fn lag_table_covers_c2_to_c6() {
        let camdf = Camdf::new(44_100);
        // C2 is about 65.4 Hz, so a period of roughly 674 samples.
        assert_eq!(camdf.delays()[0], 674);
        // A4 at 440 Hz is 100.2 samples.
        assert_eq!(camdf.delays()[crate::REFERENCE_INDEX as usize], 100);
    }

    #[test]
    fn detects_a4() {
        let camdf = Camdf::new(44_100);
        let buffer = sine(440.0, 44_100);
        let detection = camdf.detect(&buffer, 0.5).unwrap();
        assert_eq!(detection.halftone, crate::REFERENCE_INDEX);
        assert_eq!(detection.pitch_class, 9, "A is pitch class 9");
    }

    #[test]
    fn detection_is_independent_of_ring_origin() {
        let camdf = Camdf::new(44_100);
        let buffer = sine(440.0, 44_100);
        let straight = camdf.detect(&buffer, 0.5).unwrap();

        // Rotate the window; a circular measure must not care where it starts.
        let mut rotated = Box::new([0i16; ANALYSIS_WINDOW]);
        for (n, sample) in rotated.iter_mut().enumerate() {
            *sample = buffer[(n + 1234) % ANALYSIS_WINDOW];
        }
        let shifted = camdf.detect(&rotated, 0.5).unwrap();
        assert_eq!(straight.halftone, shifted.halftone);
    }
}
