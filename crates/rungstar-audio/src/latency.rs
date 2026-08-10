//! Measuring how far the microphone lags the music.
//!
//! `mic_delay_ms` shifts the whole scoring clock, so getting it wrong shifts every hit — sing
//! perfectly and score badly, with nothing on screen to say why. The default of 140 ms is a
//! reasonable guess for a USB microphone on a desktop, and a guess is all it is: a Bluetooth
//! speaker adds a couple of hundred milliseconds on its own, and a Steam Deck's internal path
//! is far quicker than either.
//!
//! So: play a sound, listen for it coming back, and measure the gap. The parts that need care
//! are which gap, what sound, and how to find it.
//!
//! **Which gap.** The song clock is taken from frames the output device has *consumed*, not
//! from frames pushed at it, so everything queued in SDL is already accounted for. What is not
//! is the device's own buffer, the trip through the air, and the whole capture path. A
//! loopback measures exactly that remainder — provided the capture side is drained immediately
//! before the sound is played, so the two timelines are pinned to the same instant.
//!
//! **What sound.** A click is the obvious choice and the wrong one: all its energy is in one
//! sample, so a room, a cheap speaker or a moment of noise loses it. A **swept sine** spreads
//! the energy over time and frequency, and correlating against it concentrates all of that back
//! into one sharp peak — the same trick radar uses, and for the same reason. 300 Hz to 3 kHz,
//! which is where microphones and small speakers are most sensitive.
//!
//! **How to find it.** Cross-correlation, coarse then fine: a peak search at a quarter rate to
//! locate it, then a few samples either side at full rate. Searching the whole range at full
//! rate is a hundred and seventy million multiply-adds a pass, and a button that freezes the
//! game for a second is a button people press twice.

/// The lowest and highest frequency in the sweep.
///
/// The top is well under a quarter of 44.1 kHz on purpose: the coarse search decimates by four,
/// and anything above that quarter would alias and move the peak it is trying to find.
const SWEEP_LOW: f32 = 300.0;
const SWEEP_HIGH: f32 = 3000.0;

/// How long the sweep lasts.
///
/// Long enough to be found in a room with people in it, short enough not to be a noise anybody
/// minds hearing five times.
pub const SWEEP_SECS: f32 = 0.15;

/// The furthest delay worth looking for.
///
/// Half a second is already unusable for singing — it is two beats at 240 BPM — so a result
/// past it means the sweep was not heard and something else correlated.
pub const MAX_DELAY_SECS: f32 = 0.5;

/// How much the coarse pass decimates.
const DECIMATE: usize = 4;

/// The weakest match worth believing.
///
/// **Measured rather than chosen.** Noise alone peaks around 0.095 — correlate anything against
/// enough noise and something will match a bit — while a sweep buried 12 dB *under* the noise
/// still reaches 0.13, and one in an ordinary room reaches 0.9. This sits above the first and
/// below the second, and `noise_and_a_real_sweep_stay_far_enough_apart` is what stops a change
/// to the sweep quietly closing the gap.
pub const CONFIDENT: f32 = 0.12;

/// The sweep to play, as 16-bit mono at `rate`.
///
/// Amplitude is deliberately modest. It goes out of a speaker into a microphone, which is the
/// arrangement that howls, and a correlation does not need volume — it needs the signal to be
/// *present*.
pub fn sweep(rate: u32) -> Vec<i16> {
    let rate = rate.max(1) as f32;
    let total = (SWEEP_SECS * rate) as usize;
    (0..total)
        .map(|n| {
            let t = n as f32 / rate;
            let progress = t / SWEEP_SECS;
            // Exponential rather than linear, so each octave gets the same amount of time and
            // the low end — where a small speaker struggles — is not over in a blink.
            let freq = SWEEP_LOW * (SWEEP_HIGH / SWEEP_LOW).powf(progress);
            // The phase of an exponential sweep is the integral of its frequency.
            let k = (SWEEP_HIGH / SWEEP_LOW).ln() / SWEEP_SECS;
            let phase = std::f32::consts::TAU * SWEEP_LOW * ((k * t).exp() - 1.0) / k;
            // A raised cosine over the whole sweep, so it starts and ends at silence. A sweep
            // that begins abruptly is a click, and a click is what this is trying not to rely
            // on.
            let window = 0.5 - 0.5 * (std::f32::consts::TAU * progress).cos();
            let _ = freq;
            (phase.sin() * window * 0.35 * 32767.0) as i16
        })
        .collect()
}

/// What one measurement found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heard {
    /// The delay, in milliseconds.
    pub millis: f32,
    /// How well the best match matched, from 0 to 1.
    ///
    /// Level-independent: it is the correlation divided by the energy of both signals, so a
    /// quiet room and a loud one give the same number for the same measurement. Noise alone
    /// lands around 0.02; a sweep that was genuinely heard is 0.1 upwards.
    pub confidence: f32,
    /// The loudest thing in the recording, as a fraction of full scale.
    ///
    /// Carried because "the microphone heard nothing at all" and "the microphone heard the
    /// room but not the sweep" are different faults with different fixes — a dead device
    /// against speakers pointing the wrong way — and a match score alone cannot tell them
    /// apart. Both look like zero confidence.
    pub level: f32,
}

/// Find `reference` inside `recorded`, both mono at `rate`.
///
/// `recorded` must begin at the instant the reference was played, so index zero is a delay of
/// zero. `None` when there is nothing to find.
pub fn find(reference: &[i16], recorded: &[i16], rate: u32) -> Option<Heard> {
    let max_lag = (MAX_DELAY_SECS * rate as f32) as usize;
    if reference.is_empty() || recorded.len() < reference.len() {
        return None;
    }

    // Coarse: a quarter of the samples, so a search over half a second costs a hundredth of
    // what it would at full rate.
    let short_reference = decimate(reference);
    let short_recorded = decimate(recorded);
    let coarse_lags =
        (max_lag / DECIMATE).min(short_recorded.len().saturating_sub(short_reference.len()));
    if short_reference.is_empty() || coarse_lags == 0 {
        return None;
    }
    let (coarse_at, confidence) = best_lag(&short_reference, &short_recorded, coarse_lags);

    // Fine: the decimation put the answer within two full-rate samples either side, and the
    // window is widened a little beyond that for the peak's own width.
    let centre = coarse_at * DECIMATE;
    let from = centre.saturating_sub(DECIMATE * 2);
    let to = (centre + DECIMATE * 2).min(recorded.len().saturating_sub(reference.len()));
    if to <= from {
        return None;
    }
    let full_reference: Vec<f32> = reference.iter().map(|s| f32::from(*s)).collect();
    let window: Vec<f32> = recorded[from..].iter().map(|s| f32::from(*s)).collect();
    let (offset, _) = best_lag(&full_reference, &window, to - from);
    let lag = from + offset;

    // Confidence comes from the coarse pass, because that is the one that looked everywhere: a
    // fine pass over eight lags is all peak and says nothing about whether the sweep was heard.
    let level = recorded
        .iter()
        .map(|s| f32::from(s.saturating_abs()) / 32768.0)
        .fold(0.0f32, f32::max);
    Some(Heard {
        millis: lag as f32 * 1000.0 / rate as f32,
        confidence,
        level,
    })
}

/// The lag that matches best, and how well it matched, as a fraction of a perfect match.
///
/// **Normalised**, by the energy of the reference and of the window it is being compared with.
/// A raw correlation grows with how loud the recording is, so a quiet room and a loud one give
/// different numbers for the same measurement and no threshold can be set on it that means
/// anything. Divided through, the result is between 0 and 1 whatever the level: about 0.02 for
/// noise, and anything from 0.1 upwards for a sweep that was actually heard.
fn best_lag(reference: &[f32], recorded: &[f32], lags: usize) -> (usize, f32) {
    let reference_energy = reference
        .iter()
        .map(|a| f64::from(a * a))
        .sum::<f64>()
        .sqrt();
    if reference_energy <= 0.0 {
        return (0, 0.0);
    }

    // The window's energy, kept as a running total rather than recomputed per lag — otherwise
    // normalising costs as much again as the correlation it is normalising.
    let mut window_energy: f64 = recorded[..reference.len().min(recorded.len())]
        .iter()
        .map(|b| f64::from(b * b))
        .sum();

    let mut best = (0usize, 0f32);
    for lag in 0..lags {
        let window = &recorded[lag..lag + reference.len()];
        let mut sum = 0f64;
        for (a, b) in reference.iter().zip(window) {
            sum += f64::from(*a) * f64::from(*b);
        }
        // Absolute, because a speaker or a microphone may invert the signal and an inverted
        // sweep is still the sweep.
        let denominator = reference_energy * window_energy.max(1e-9).sqrt();
        let strength = (sum.abs() / denominator) as f32;
        if strength > best.1 {
            best = (lag, strength);
        }

        // Slide the window on by one.
        let leaving = f64::from(recorded[lag]);
        window_energy -= leaving * leaving;
        if let Some(entering) = recorded.get(lag + reference.len()) {
            window_energy += f64::from(*entering) * f64::from(*entering);
        }
        window_energy = window_energy.max(0.0);
    }
    best
}

/// Sum every `DECIMATE` samples, as f32.
///
/// A plain sum rather than a filtered resample: the sweep stops well below the new Nyquist, so
/// there is nothing above it to fold back, and summing is a low-pass in its own right.
fn decimate(samples: &[i16]) -> Vec<f32> {
    samples
        .chunks(DECIMATE)
        .map(|chunk| chunk.iter().map(|s| f32::from(*s)).sum::<f32>() / DECIMATE as f32)
        .collect()
}

/// Turn several passes into one answer, or say why there is not one.
///
/// The median rather than the mean, and agreement rather than confidence alone: a cough, a door
/// or somebody saying "is it doing it yet" lands on exactly one pass, and an average quietly
/// absorbs it where a median throws it away.
pub fn settle(passes: &[Heard]) -> Result<f32, &'static str> {
    let mut good: Vec<f32> = passes
        .iter()
        .filter(|heard| heard.confidence >= CONFIDENT)
        .map(|heard| heard.millis)
        .collect();
    if good.len() < 2 {
        // Which of the two faults it is. A microphone delivering silence is a different
        // problem from one that works and cannot hear the speakers, and telling somebody to
        // turn the volume up when the device is dead wastes their evening.
        let loudest = passes.iter().map(|p| p.level).fold(0.0f32, f32::max);
        return Err(if loudest < 0.005 {
            "the microphone recorded silence"
        } else {
            "the microphone did not hear the sound"
        });
    }
    good.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = good[good.len() / 2];

    // A **majority** within five milliseconds of each other, which is the part that actually
    // rules noise out. Confidence is a single-pass judgement and the gap between a faint sweep
    // and a loud room is not wide; agreement is not close at all, because noise matches best at
    // a different lag every time while a real delay lands in the same place every time. Five
    // milliseconds, because below that is finer than anybody can sing.
    let agreeing = good.iter().filter(|m| (**m - median).abs() <= 5.0).count();
    if agreeing * 2 <= good.len() {
        return Err("the measurements did not agree");
    }
    Ok(median)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    /// A recording of the sweep, `delay_ms` late, with `noise` of full scale mixed in.
    fn recorded(delay_ms: f32, noise: f32, seconds: f32) -> Vec<i16> {
        let reference = sweep(RATE);
        let total = (seconds * RATE as f32) as usize;
        let at = (delay_ms * RATE as f32 / 1000.0) as usize;
        let mut state = 0x2545_f491u32;
        (0..total)
            .map(|n| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let white = ((state & 0xFFFF) as f32 / 32768.0) - 1.0;
                let signal = reference.get(n.wrapping_sub(at)).copied().unwrap_or(0);
                let mixed = f32::from(signal) * 0.5 + white * noise * 32767.0;
                mixed.clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn a_known_delay_is_measured_to_within_a_millisecond() {
        for wanted in [0.0f32, 12.0, 45.5, 140.0, 320.0] {
            let heard = find(&sweep(RATE), &recorded(wanted, 0.0, 1.0), RATE).expect("a delay");
            assert!(
                (heard.millis - wanted).abs() <= 1.0,
                "wanted {wanted} ms, measured {:.1} ms",
                heard.millis
            );
        }
    }

    #[test]
    fn it_still_works_in_a_room_with_people_in_it() {
        // Noise at a tenth of full scale is a loud room. The sweep is quieter than that in
        // places, which is the whole reason for correlating rather than looking for a peak.
        let heard = find(&sweep(RATE), &recorded(95.0, 0.1, 1.0), RATE).expect("a delay");
        assert!(
            (heard.millis - 95.0).abs() <= 2.0,
            "measured {:.1} ms",
            heard.millis
        );
        assert!(
            heard.confidence > CONFIDENT,
            "confidence {:.2}",
            heard.confidence
        );
    }

    #[test]
    fn an_inverted_signal_is_still_the_signal() {
        // A speaker wired out of phase, or a microphone that inverts. The delay is the same
        // and only the sign of the correlation changes.
        let mut flipped = recorded(60.0, 0.0, 1.0);
        for sample in &mut flipped {
            *sample = -*sample;
        }
        let heard = find(&sweep(RATE), &flipped, RATE).expect("a delay");
        assert!((heard.millis - 60.0).abs() <= 1.0, "{:.1} ms", heard.millis);
    }

    #[test]
    fn silence_is_reported_as_nothing_heard() {
        // Headphones, a muted microphone, a device that opens and delivers nothing. The
        // measurement must refuse rather than return whatever noise correlated best.
        let quiet: Vec<i16> = vec![0; RATE as usize];
        let heard = find(&sweep(RATE), &quiet, RATE);
        assert!(heard.is_none_or(|h| h.confidence < CONFIDENT), "{heard:?}");
    }

    #[test]
    fn noise_alone_does_not_look_like_a_measurement() {
        for (level, seed) in [
            (0.05f32, 0xdead_beefu32),
            (0.4, 0x0bad_f00d),
            (0.9, 0x1357_9bdf),
        ] {
            let heard = find(&sweep(RATE), &noise_only(level, 1.0, seed), RATE);
            assert!(
                heard.is_none_or(|h| h.confidence < CONFIDENT),
                "noise at {level} measured as {heard:?}"
            );
        }
    }

    #[test]
    fn noise_and_a_real_sweep_stay_far_enough_apart() {
        // Where `CONFIDENT` came from, kept as a test so that changing the sweep — its length,
        // its frequency range, its window — cannot quietly close the gap it sits in.
        let reference = sweep(RATE);
        let loudest_noise = [0x1111_2222u32, 0x3333_4444, 0x5555_6666]
            .into_iter()
            .filter_map(|seed| find(&reference, &noise_only(0.3, 1.0, seed), RATE))
            .map(|heard| heard.confidence)
            .fold(0.0f32, f32::max);

        // A sweep twelve decibels *under* the noise around it, which is far worse than any
        // real room.
        let mut faint = noise_only(0.2, 1.0, 0x5555_1234);
        let at = (95.0 * RATE as f32 / 1000.0) as usize;
        for (i, sample) in reference.iter().enumerate() {
            let slot = &mut faint[at + i];
            *slot = slot.saturating_add((f32::from(*sample) * 0.05) as i16);
        }
        let quietest_signal = find(&reference, &faint, RATE).expect("a delay").confidence;

        assert!(
            loudest_noise < CONFIDENT,
            "noise reaches {loudest_noise:.3}, at or above the {CONFIDENT} threshold"
        );
        assert!(
            quietest_signal > CONFIDENT,
            "a faint sweep only reaches {quietest_signal:.3}"
        );
    }

    /// Noise and nothing else.
    fn noise_only(level: f32, seconds: f32, seed: u32) -> Vec<i16> {
        let total = (seconds * RATE as f32) as usize;
        let mut state = seed;
        (0..total)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let white = ((state & 0xFFFF) as f32 / 32768.0) - 1.0;
                (white * level * 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn one_bad_pass_does_not_move_the_answer() {
        // Somebody coughs, or a door goes. A mean would absorb it; the median throws it away.
        let passes = [
            Heard {
                millis: 96.0,
                confidence: 0.9,
                level: 0.3,
            },
            Heard {
                millis: 94.0,
                confidence: 0.8,
                level: 0.3,
            },
            Heard {
                millis: 310.0,
                confidence: 0.7,
                level: 0.3,
            },
            Heard {
                millis: 95.0,
                confidence: 0.95,
                level: 0.3,
            },
            Heard {
                millis: 95.5,
                confidence: 0.85,
                level: 0.3,
            },
        ];
        let settled = settle(&passes).expect("an answer");
        assert!((settled - 95.0).abs() <= 1.5, "settled on {settled}");
    }

    #[test]
    fn passes_nobody_heard_are_refused_rather_than_averaged() {
        let passes = [
            Heard {
                millis: 12.0,
                confidence: 0.02,
                level: 0.3,
            },
            Heard {
                millis: 340.0,
                confidence: 0.03,
                level: 0.3,
            },
        ];
        assert!(settle(&passes).is_err());
        assert!(settle(&[]).is_err());
    }

    #[test]
    fn measurements_that_disagree_are_refused() {
        // Two confident answers a hundred milliseconds apart are not a measurement, they are
        // two different things being measured. Better to say so than to average them.
        let passes = [
            Heard {
                millis: 40.0,
                confidence: 0.9,
                level: 0.3,
            },
            Heard {
                millis: 260.0,
                confidence: 0.9,
                level: 0.3,
            },
        ];
        assert!(settle(&passes).is_err());
    }

    #[test]
    fn the_sweep_starts_and_ends_at_silence() {
        // It plays out of a speaker sitting next to an open microphone. A sweep that begins
        // abruptly is a click, and a click is exactly what correlating is meant to avoid
        // depending on.
        let signal = sweep(RATE);
        assert!(signal.len() > 1000);
        assert!(signal[0].abs() < 200, "starts at {}", signal[0]);
        assert!(
            signal[signal.len() - 1].abs() < 200,
            "ends at {}",
            signal[signal.len() - 1]
        );
        let peak = signal.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(peak > 5_000 && peak < 16_000, "peak {peak}");
    }
}
