//! How loud a song actually is, to the ear.
//!
//! A library assembled from a thousand different uploads is not level with itself: one song
//! arrives mastered for radio and the next is a quiet transfer of a vinyl rip, and the
//! difference is easily fifteen decibels. At a party that means somebody reaches for the volume
//! between every song, which is the one job the game should be doing for them.
//!
//! **Peak level is the wrong measure and RMS is not much better.** A track that is quiet but
//! has one loud drum hit peaks at full scale, so peak normalisation leaves it quiet. RMS treats
//! a bass rumble as loudly as a vocal, so a bass-heavy mix ends up turned down for energy
//! nobody perceives.
//!
//! So this is **EBU R128** (ITU-R BS.1770), which is what broadcast and every streaming service
//! uses: filter the signal to approximate what the ear is sensitive to, measure mean square
//! power over overlapping blocks, and then throw away the quiet blocks before averaging — so
//! the answer describes the *song*, not the silence around it. Two filters, one gate, and about
//! a hundred lines; the alternative is a library that is loud in patches.
//!
//! The result is in **LUFS**, negative, typically between -6 and -25.

/// Where songs are normalised to, in LUFS.
///
/// **Measured against a real library rather than chosen from a standard.** Sixty songs from an
/// 8,134-song collection run from -27 to -5.6 LUFS — a spread of 21.6 dB, which is the
/// complaint — with a mean of -10.3. ReplayGain's -18 and broadcast's -23 are both far below
/// that, so either would turn almost every song *down* by eight decibels or more and the whole
/// game would go quiet.
///
/// -14 is what the streaming services settled on for the same reason and against the same kind
/// of material: close enough to modern masters that most songs are barely touched, low enough
/// that the quiet ones can be brought up without a wall of limiting.
pub const TARGET_LUFS: f32 = -14.0;

/// The highest a normalised song is allowed to peak, as a fraction of full scale.
///
/// Turning a quiet song up is only free while there is headroom above its loudest sample.
/// Past that the samples clamp, and a clipped chorus is a far worse fault than a quiet song —
/// it is heard as distortion, which sounds like a broken game rather than a quiet recording.
const CEILING: f32 = 0.98;

/// A two-pole filter section, as a difference equation.
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

#[derive(Clone, Copy, Default)]
struct State {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn run(&self, state: &mut State, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * state.x1 + self.b[2] * state.x2
            - self.a[0] * state.y1
            - self.a[1] * state.y2;
        state.x2 = state.x1;
        state.x1 = x;
        state.y2 = state.y1;
        state.y1 = y;
        y
    }
}

/// The two BS.1770 pre-filters, designed for `rate`.
///
/// The standard tabulates them at 48 kHz. Songs are not all at 48 kHz, so they are designed
/// here from the frequencies and Q factors those coefficients came from — using the 48 kHz
/// numbers directly on 44.1 kHz material shifts both corners by a tenth, which is a small
/// error in the same direction for every song and therefore invisible until compared with
/// anything measured properly.
fn prefilters(rate: f64) -> (Biquad, Biquad) {
    // Stage 1: a high-frequency shelf standing in for the head's own response.
    let f0 = 1681.974450955533;
    let gain_db = 3.999843853973347;
    let q = 0.7071752369554196;

    let k = (std::f64::consts::PI * f0 / rate).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let denominator = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / denominator,
            2.0 * (k * k - vh) / denominator,
            (vh - vb * k / q + k * k) / denominator,
        ],
        a: [
            2.0 * (k * k - 1.0) / denominator,
            (1.0 - k / q + k * k) / denominator,
        ],
    };

    // Stage 2: a high-pass, because rumble below 40 Hz is felt rather than heard and should
    // not count towards how loud something is.
    let f0 = 38.13547087602444;
    let q = 0.5003270373238773;
    let k = (std::f64::consts::PI * f0 / rate).tan();
    let denominator = 1.0 + k / q + k * k;
    let highpass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [
            2.0 * (k * k - 1.0) / denominator,
            (1.0 - k / q + k * k) / denominator,
        ],
    };

    (shelf, highpass)
}

/// What a measurement found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    /// Integrated loudness in LUFS. Negative, typically -6 to -25.
    pub lufs: f32,
    /// The loudest sample, as a fraction of full scale.
    ///
    /// Kept because loudness alone does not say how much room there is above it. A sparse
    /// recording can be quiet and still touch full scale on one drum hit, and turning that up
    /// by the nine decibels its loudness asks for clips every hit in the song.
    pub peak: f32,
}

/// Measure an interleaved 16-bit signal.
pub fn analyse(samples: &[i16], channels: usize, rate: u32) -> Option<Measured> {
    let peak = samples
        .iter()
        .map(|s| f32::from(s.saturating_abs()) / 32768.0)
        .fold(0.0f32, f32::max);
    integrated(samples, channels, rate).map(|lufs| Measured { lufs, peak })
}

/// Integrated loudness of an interleaved 16-bit signal, in LUFS.
///
/// `None` when there is not enough audio to measure — under a second, or a file that is
/// silence all the way through. A caller must treat that as "leave it alone" rather than as
/// zero, since a gain computed from silence is a very loud number.
pub fn integrated(samples: &[i16], channels: usize, rate: u32) -> Option<f32> {
    let channels = channels.max(1);
    let rate = rate.max(1) as f64;
    let frames = samples.len() / channels;
    // Blocks are 400 ms, overlapping by 75%, which is what the standard specifies. The overlap
    // is what stops a loud moment falling between two blocks and being averaged away.
    let block = (rate * 0.4) as usize;
    let step = block / 4;
    if block == 0 || step == 0 || frames < block {
        return None;
    }

    let (shelf, highpass) = prefilters(rate);
    // Filtering is per channel and the state has to carry across the whole signal, so the
    // filtered signal is built once rather than per block.
    let mut filtered = vec![0f32; frames * channels];
    for channel in 0..channels {
        let mut a = State::default();
        let mut b = State::default();
        for frame in 0..frames {
            let x = f64::from(samples[frame * channels + channel]) / 32768.0;
            let y = highpass.run(&mut b, shelf.run(&mut a, x));
            filtered[frame * channels + channel] = y as f32;
        }
    }

    // Mean square per block, summed across channels. The standard weights surround channels
    // above unity; with mono and stereo every weight is 1, and nothing here is surround.
    let mut blocks: Vec<f64> = Vec::with_capacity(frames / step + 1);
    let mut start = 0;
    while start + block <= frames {
        let mut sum = 0f64;
        for frame in start..start + block {
            for channel in 0..channels {
                let v = f64::from(filtered[frame * channels + channel]);
                sum += v * v;
            }
        }
        blocks.push(sum / block as f64);
        start += step;
    }
    if blocks.is_empty() {
        return None;
    }

    let loudness_of = |mean_square: f64| -0.691 + 10.0 * mean_square.max(1e-12).log10();

    // The gating, which is the part that makes this a measure of the song rather than of the
    // recording. An absolute gate drops digital silence and room tone; a relative gate, set
    // ten decibels below the ungated average, then drops the quiet passages — so a track with
    // a long fade-out is not reported as quieter than the same track without one.
    const ABSOLUTE: f64 = -70.0;
    let above_absolute: Vec<f64> = blocks
        .iter()
        .copied()
        .filter(|ms| loudness_of(*ms) > ABSOLUTE)
        .collect();
    if above_absolute.is_empty() {
        return None;
    }
    let mean = above_absolute.iter().sum::<f64>() / above_absolute.len() as f64;
    let relative = loudness_of(mean) - 10.0;

    let kept: Vec<f64> = above_absolute
        .into_iter()
        .filter(|ms| loudness_of(*ms) > relative)
        .collect();
    if kept.is_empty() {
        return None;
    }
    let mean = kept.iter().sum::<f64>() / kept.len() as f64;
    Some(loudness_of(mean) as f32)
}

/// The gain that brings `lufs` to [`TARGET_LUFS`], as a linear multiplier.
///
/// Clamped hard in both directions. A measurement can be wrong — a file that is mostly silence
/// with one loud burst, a song where the recorded music is a fraction of the running time — and
/// an unclamped correction turns that into either a whisper or a bang. Twelve decibels covers
/// every real difference between two masters; anything past it is a measurement to distrust.
pub fn gain_for(lufs: f32) -> f32 {
    const LIMIT_DB: f32 = 12.0;
    let db = (TARGET_LUFS - lufs).clamp(-LIMIT_DB, LIMIT_DB);
    10f32.powf(db / 20.0)
}

/// The gain to play a measured song at, which is [`gain_for`] with the clipping taken out.
///
/// A boost is only free while there is headroom above the loudest sample. Turning a quiet but
/// peaky recording up by the nine decibels its loudness asks for makes every peak clamp, and a
/// distorted chorus reads as a broken game where a quiet song reads as a quiet recording.
/// Turning something *down* can never clip, so a cut is never limited.
pub fn playback_gain(measured: Measured) -> f32 {
    let wanted = gain_for(measured.lufs);
    if wanted <= 1.0 || measured.peak <= 0.0 {
        return wanted;
    }
    wanted.min(CEILING / measured.peak).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `amplitude` of full scale, `seconds` long.
    fn sine(freq: f32, amplitude: f32, seconds: f32, rate: u32) -> Vec<i16> {
        let frames = (seconds * rate as f32) as usize;
        (0..frames)
            .map(|n| {
                let t = n as f32 / rate as f32;
                (amplitude * (std::f32::consts::TAU * freq * t).sin() * 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn a_1khz_sine_at_minus_20_dbfs_measures_about_minus_20_lufs() {
        // The calibration the standard is defined against, and worth stating precisely: the
        // level is **RMS**, not peak. A sine of amplitude `a` has RMS `a/sqrt(2)`, so the
        // amplitude that gives -20 dBFS RMS is 0.1 * sqrt(2), and reading -23 for an amplitude
        // of 0.1 is the measure being right rather than three decibels out.
        let amplitude = 0.1 * std::f32::consts::SQRT_2;
        let lufs = integrated(&sine(1000.0, amplitude, 3.0, 48_000), 1, 48_000).expect("a number");
        assert!(
            (lufs - -20.0).abs() < 0.5,
            "a -20 dBFS RMS tone measured {lufs:.2} LUFS"
        );
    }

    #[test]
    fn twice_the_amplitude_is_six_decibels_louder() {
        let quiet = integrated(&sine(1000.0, 0.1, 3.0, 44_100), 1, 44_100).unwrap();
        let loud = integrated(&sine(1000.0, 0.2, 3.0, 44_100), 1, 44_100).unwrap();
        assert!(
            (loud - quiet - 6.02).abs() < 0.2,
            "{quiet:.2} then {loud:.2} LUFS"
        );
    }

    #[test]
    fn bass_counts_for_less_than_a_midrange_tone_of_the_same_size() {
        // The whole reason for not using plain RMS: low frequencies at the same amplitude as a
        // midrange tone are far less audible, and normalising by raw energy turns a bass-heavy
        // master right down. The weighting falls away below its 38 Hz corner, so the further
        // down the tone is the bigger the difference.
        let at = |freq| integrated(&sine(freq, 0.2, 3.0, 48_000), 1, 48_000).unwrap();
        let mid = at(1000.0);
        assert!(at(40.0) < mid - 4.0, "{:.1} against {mid:.1}", at(40.0));
        assert!(at(20.0) < at(40.0) - 5.0, "20 Hz is not below 40 Hz");
    }

    #[test]
    fn a_long_silence_does_not_make_a_song_quiet() {
        // The gate. A song with a minute of nothing after it is exactly as loud as the same
        // song without; measured ungated it would be reported several decibels quieter and
        // then turned up over everything else.
        let rate = 44_100;
        let music = sine(1000.0, 0.15, 4.0, rate);
        let mut padded = music.clone();
        padded.extend(std::iter::repeat_n(0i16, rate as usize * 8));

        let plain = integrated(&music, 1, rate).unwrap();
        let with_silence = integrated(&padded, 1, rate).unwrap();
        assert!(
            (plain - with_silence).abs() < 0.5,
            "{plain:.2} against {with_silence:.2}"
        );
    }

    #[test]
    fn silence_is_not_measurable_rather_than_being_measured_as_nothing() {
        // A gain computed from a loudness of zero is enormous, so this has to be an absence
        // rather than a number.
        assert!(integrated(&vec![0i16; 44_100 * 2], 1, 44_100).is_none());
        assert!(integrated(&[], 1, 44_100).is_none());
        assert!(integrated(&[1, 2, 3], 1, 44_100).is_none());
    }

    #[test]
    fn a_boost_stops_where_the_headroom_does() {
        // A sparse recording can measure quiet and still touch full scale on one hit. Its
        // loudness asks for a boost that would clamp every one of those hits.
        let peaky = Measured {
            lufs: -26.0,
            peak: 0.99,
        };
        assert!(
            (playback_gain(peaky) - 1.0).abs() < 0.01,
            "no room above it, so no boost: got {}",
            playback_gain(peaky)
        );

        // The same loudness with room above it gets the whole correction.
        let roomy = Measured {
            lufs: -26.0,
            peak: 0.3,
        };
        assert!(playback_gain(roomy) > 3.0, "{}", playback_gain(roomy));

        // And whatever the peak, the result never clips.
        for peak in [0.05, 0.3, 0.7, 0.99, 1.0] {
            for lufs in [-30.0, -22.0, -14.0, -6.0] {
                let gain = playback_gain(Measured { lufs, peak });
                assert!(
                    gain * peak <= 1.0001,
                    "{lufs} LUFS at peak {peak} gives {gain}, which clips"
                );
            }
        }
    }

    #[test]
    fn turning_a_song_down_is_never_limited() {
        // A cut cannot clip, so the ceiling has no say in it.
        let loud = Measured {
            lufs: -5.0,
            peak: 1.0,
        };
        assert!(playback_gain(loud) < 1.0);
        assert!((playback_gain(loud) - gain_for(-5.0)).abs() < 0.001);
    }

    #[test]
    fn a_measurement_carries_the_peak_with_it() {
        let quiet = sine(1000.0, 0.25, 2.0, 44_100);
        let measured = analyse(&quiet, 1, 44_100).expect("a number");
        assert!((measured.peak - 0.25).abs() < 0.01, "{}", measured.peak);
        assert!(measured.lufs < -14.0);
    }

    #[test]
    fn a_quiet_song_is_turned_up_and_a_loud_one_down() {
        assert!(gain_for(-24.0) > 1.0);
        assert!(gain_for(-8.0) < 1.0);
        assert!((gain_for(TARGET_LUFS) - 1.0).abs() < 0.001);
    }

    #[test]
    fn a_wild_measurement_cannot_produce_a_wild_gain() {
        // Twelve decibels either way. A file that is mostly silence measures very quiet, and
        // an unclamped correction would answer that with a bang.
        assert!(gain_for(-60.0) <= 10f32.powf(12.0 / 20.0) + 0.001);
        assert!(gain_for(0.0) >= 10f32.powf(-12.0 / 20.0) - 0.001);
    }

    #[test]
    fn stereo_and_mono_of_the_same_material_agree() {
        // Loudness sums across channels, so the same signal in both ears is 3 dB louder — that
        // is correct and is what the standard says. What must not happen is the interleaving
        // being read wrong, which shows up as a much bigger difference than that.
        let rate = 48_000;
        let mono = sine(1000.0, 0.15, 3.0, rate);
        let stereo: Vec<i16> = mono.iter().flat_map(|s| [*s, *s]).collect();
        let a = integrated(&mono, 1, rate).unwrap();
        let b = integrated(&stereo, 2, rate).unwrap();
        assert!((b - a - 3.01).abs() < 0.2, "{a:.2} then {b:.2}");
    }
}
