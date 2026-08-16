//! The song clock: does it stay with the audio without ever stepping back?

use std::time::Duration;

use rungstar_audio::{MasterClock, Timing};

fn clock(bpm: f64, gap_ms: f64) -> MasterClock {
    let mut clock = MasterClock::new(Timing::new(bpm, gap_ms));
    clock.start();
    clock
}

fn advance(clock: &mut MasterClock, seconds: f64) {
    clock.tick(Duration::from_secs_f64(seconds));
}

#[test]
fn beats_follow_the_same_arithmetic_as_the_song_format() {
    // The grid runs at four times the file's BPM, so at 60 BPM a beat is a quarter second.
    let mut clock = clock(60.0, 500.0);
    clock.seek(1.5);
    let beats = clock.beats();
    // 1.5s in, minus the 0.5s gap, is one second of song, which is four beats.
    assert!((beats.visual - 4.0).abs() < 1e-9, "got {}", beats.visual);
}

#[test]
fn the_detection_stream_lags_the_visual_one() {
    let clock = clock(60.0, 0.0);
    let beats = clock.beats();
    // Half a beat, plus the microphone delay expressed in beats.
    let expected = -0.5 - 0.140 * 240.0 / 60.0;
    assert!(
        (beats.detection - expected).abs() < 1e-9,
        "got {}",
        beats.detection
    );
    assert!(beats.detection < beats.visual);
}

#[test]
fn a_small_lead_is_absorbed_gradually() {
    let mut clock = clock(120.0, 0.0);
    clock.seek(10.0);
    // Audio is 5ms ahead — under the threshold, so nothing jumps.
    let drift = clock.synchronize(10.005);
    assert!(drift.abs() < 0.010);
    assert!(
        (clock.position() - 10.0).abs() < 1e-9,
        "position should not have moved"
    );
}

#[test]
fn a_persistent_lead_is_stepped_forward() {
    let mut clock = clock(120.0, 0.0);
    clock.seek(10.0);
    // Repeated reports of the audio being well ahead build up until the clock catches up.
    for _ in 0..20 {
        clock.synchronize(10.1);
    }
    assert!(
        clock.position() > 10.0,
        "clock should have moved toward the audio"
    );
    assert!(clock.position() <= 10.1 + 1e-9, "and not overshot");
}

#[test]
fn the_clock_never_runs_backwards() {
    let mut clock = clock(120.0, 0.0);
    clock.seek(10.0);
    let before = clock.position();

    // Audio consistently reports being behind. The clock must hold, not rewind: rewinding
    // would re-score beats that have already been counted.
    for _ in 0..50 {
        clock.synchronize(9.5);
        advance(&mut clock, 0.016);
        assert!(clock.position() >= before, "position went backwards");
    }
}

#[test]
fn holding_ends_once_the_audio_catches_up() {
    let mut clock = clock(120.0, 0.0);
    clock.seek(10.0);
    for _ in 0..10 {
        clock.synchronize(9.9);
    }
    let held = clock.position();
    advance(&mut clock, 0.05);
    assert!(
        (clock.position() - held).abs() < 1e-9,
        "should be holding still"
    );

    // Audio arrives level again, so normal running resumes.
    for _ in 0..10 {
        clock.synchronize(clock.position());
    }
    advance(&mut clock, 0.05);
    assert!(clock.position() > held, "should be running again");
}

#[test]
fn a_huge_jump_is_treated_as_a_seek_not_as_drift() {
    let mut clock = clock(120.0, 0.0);
    clock.seek(10.0);
    // Someone skipped ahead, or the decoder restarted. Averaging this in would take many
    // seconds to work off, so it is applied at once.
    clock.synchronize(90.0);
    assert!((clock.position() - 90.0).abs() < 1e-9);
}

#[test]
fn a_paused_clock_does_not_advance() {
    let mut clock = clock(120.0, 0.0);
    clock.seek(5.0);
    clock.pause();
    advance(&mut clock, 1.0);
    assert!((clock.position() - 5.0).abs() < 1e-9);

    clock.start();
    advance(&mut clock, 1.0);
    assert!((clock.position() - 6.0).abs() < 1e-9);
}

#[test]
fn the_first_synchronize_adopts_the_audio_position() {
    // Before the clock has been started it has nothing better to go on.
    let mut clock = MasterClock::new(Timing::new(120.0, 0.0));
    clock.synchronize(3.25);
    assert!((clock.position() - 3.25).abs() < 1e-9);
}

#[test]
fn crossed_beats_cover_each_whole_beat_exactly_once() {
    // Polling faster than the beat rate must not repeat or skip a beat.
    let mut seen = Vec::new();
    let mut previous = -1.0f64;
    for step in 0..40 {
        let current = f64::from(step) * 0.25;
        for beat in MasterClock::beats_crossed(previous, current) {
            seen.push(beat);
        }
        previous = current;
    }
    let expected: Vec<i32> = (0..=9).collect();
    assert_eq!(seen, expected);
}

#[test]
fn crossed_beats_are_empty_when_time_stands_still() {
    assert!(MasterClock::beats_crossed(4.2, 4.2).is_empty());
    assert!(MasterClock::beats_crossed(4.2, 4.9).is_empty());
    assert_eq!(
        MasterClock::beats_crossed(4.2, 5.1).collect::<Vec<_>>(),
        vec![5]
    );
}

#[test]
fn a_detection_beat_can_be_asked_for_at_any_microphone_delay() {
    // One delay for the whole game cannot serve a USB microphone and a Bluetooth headset at
    // the same time: they are hundreds of milliseconds apart, and that difference is every
    // hit either singer makes. So the clock answers for a delay rather than holding one.
    let mut clock = clock(60.0, 0.0);
    clock.seek(1.0);

    // The grid runs at four beats a second here, so 140 ms is 0.56 of a beat.
    let immediate = clock.detection_beat(0.0);
    let lagging = clock.detection_beat(0.14);
    assert!(
        (immediate - lagging - 0.56).abs() < 1e-9,
        "{immediate} against {lagging}"
    );

    // And the stream the clock has always reported is that same function asked at the shared
    // delay, so there are not two answers to one question.
    let shared = clock.timing().mic_delay;
    assert!((clock.detection_beat(shared) - clock.beats().detection).abs() < 1e-9);
}
