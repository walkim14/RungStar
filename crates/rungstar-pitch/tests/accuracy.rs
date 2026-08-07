//! Detector accuracy against synthesised tones of known pitch.
//!
//! Real validation needs a microphone and a singer, but a great deal can be pinned without
//! either: a detector that cannot find a sawtooth at 220 Hz will not find a voice.

use rungstar_pitch::{
    halftone_to_frequency, Algorithm, Analyzer, AnalyzerConfig, MicBoost, ANALYSIS_WINDOW,
    HALFTONE_COUNT,
};

const RATE: u32 = 44_100;

/// A voice-like tone: fundamental plus a few harmonics at decreasing amplitude.
fn voice(frequency: f64, amplitude: f64) -> Vec<i16> {
    (0..ANALYSIS_WINDOW)
        .map(|n| {
            let t = n as f64 / f64::from(RATE);
            let value = (t * frequency * std::f64::consts::TAU).sin() * 0.6
                + (t * frequency * 2.0 * std::f64::consts::TAU).sin() * 0.25
                + (t * frequency * 3.0 * std::f64::consts::TAU).sin() * 0.15;
            (value * amplitude * 32_000.0) as i16
        })
        .collect()
}

fn analyzer(algorithm: Algorithm) -> Analyzer {
    Analyzer::new(AnalyzerConfig {
        algorithm,
        ..AnalyzerConfig::default()
    })
}

fn detect(algorithm: Algorithm, samples: &[i16]) -> Option<rungstar_pitch::Detection> {
    let mut analyzer = analyzer(algorithm);
    analyzer.push(samples);
    analyzer.detect()
}

/// How many of the 49 semitones each detector identifies exactly.
fn sweep_accuracy(algorithm: Algorithm) -> (usize, usize) {
    let mut exact = 0;
    let mut right_pitch_class = 0;
    for halftone in 0..HALFTONE_COUNT as i32 {
        let frequency = halftone_to_frequency(halftone);
        let Some(detection) = detect(algorithm, &voice(frequency, 0.5)) else {
            continue;
        };
        if detection.halftone == halftone {
            exact += 1;
        }
        if detection.pitch_class == halftone.rem_euclid(12) {
            right_pitch_class += 1;
        }
    }
    (exact, right_pitch_class)
}

#[test]
fn mpm_identifies_every_semitone_exactly() {
    let (exact, _) = sweep_accuracy(Algorithm::Mpm);
    assert_eq!(
        exact, HALFTONE_COUNT,
        "MPM should place every semitone of C2..C6"
    );
}

#[test]
fn mpm_frequency_is_within_a_few_cents() {
    for halftone in 0..HALFTONE_COUNT as i32 {
        let expected = halftone_to_frequency(halftone);
        let detection = detect(Algorithm::Mpm, &voice(expected, 0.5)).expect("a detection");
        let reported = f64::from(detection.frequency.expect("MPM reports a frequency"));
        let cents = 1200.0 * (reported / expected).log2();
        assert!(
            cents.abs() < 10.0,
            "semitone {halftone}: expected {expected:.2} Hz, got {reported:.2} Hz ({cents:.1} cents off)"
        );
    }
}

#[test]
fn camdf_gets_the_pitch_class_right_across_the_range() {
    // CAMDF quantises to a lag in whole samples, so at the top of the range neighbouring
    // semitones share a lag and it cannot always place the octave. Pitch class is what
    // scoring compares, and that is what has to hold.
    let (_, pitch_class) = sweep_accuracy(Algorithm::Camdf);
    assert!(
        pitch_class >= HALFTONE_COUNT - 4,
        "CAMDF placed only {pitch_class}/{HALFTONE_COUNT} pitch classes"
    );
}

#[test]
fn mpm_resists_the_octave_error() {
    // A tone whose second harmonic is louder than its fundamental. Choosing the tallest
    // correlation peak would report the octave above; choosing the earliest tall-enough one
    // gets it right.
    let samples: Vec<i16> = (0..ANALYSIS_WINDOW)
        .map(|n| {
            let t = n as f64 / f64::from(RATE);
            let f = 220.0;
            let value = (t * f * std::f64::consts::TAU).sin() * 0.35
                + (t * f * 2.0 * std::f64::consts::TAU).sin() * 0.65;
            (value * 20_000.0) as i16
        })
        .collect();
    let detection = detect(Algorithm::Mpm, &samples).expect("a detection");
    let frequency = detection.frequency.unwrap();
    assert!(
        (frequency - 220.0).abs() < 5.0,
        "expected the 220 Hz fundamental, got {frequency:.1} Hz"
    );
}

#[test]
fn quiet_input_is_gated_out() {
    // Below the threshold nothing is reported, however periodic it is.
    let quiet = voice(440.0, 0.01);
    assert!(detect(Algorithm::Mpm, &quiet).is_none());
    assert!(detect(Algorithm::Camdf, &quiet).is_none());
}

#[test]
fn noise_is_rejected_as_unvoiced() {
    // A crude deterministic noise source; no periodicity for MPM to lock onto.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let noise: Vec<i16> = (0..ANALYSIS_WINDOW)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 48) as i16
        })
        .collect();
    assert!(
        detect(Algorithm::Mpm, &noise).is_none(),
        "noise should read as unvoiced"
    );
}

#[test]
fn mic_boost_lifts_a_quiet_signal_over_the_gate() {
    let quiet = voice(440.0, 0.03);
    let mut plain = analyzer(Algorithm::Mpm);
    plain.push(&quiet);
    assert!(
        plain.detect().is_none(),
        "should be below the gate unboosted"
    );

    let mut boosted = Analyzer::new(AnalyzerConfig {
        algorithm: Algorithm::Mpm,
        boost: MicBoost::Plus18dB,
        ..AnalyzerConfig::default()
    });
    boosted.push(&quiet);
    let detection = boosted
        .detect()
        .expect("boost should carry it over the gate");
    assert_eq!(detection.halftone, 33, "still A4 after boosting");
}

#[test]
fn a_partly_filled_window_reports_nothing() {
    let mut analyzer = analyzer(Algorithm::Mpm);
    analyzer.push(&voice(440.0, 0.5)[..1000]);
    assert!(!analyzer.is_ready());
    assert!(analyzer.detect().is_none());
}
