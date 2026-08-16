//! The clock everything else follows.
//!
//! Singing is judged by comparing when a note *should* be sung against when a sound arrived,
//! so the whole game hangs off one question: how far into the song are we? Three things
//! disagree about the answer — the audio device's playback position, the system timer, and
//! the video decoder — and the differences are milliseconds, which is exactly the scale that
//! decides whether a note counts.
//!
//! The approach is UltraStar Deluxe's, and it is the right one: run a free-running timer, and
//! drag it toward the audio device's reported position gradually rather than snapping to it.
//! Device positions arrive quantised to buffer boundaries and jitter by several milliseconds;
//! following them directly makes the beat visibly stutter.
//!
//! One rule matters more than the rest: **the clock never runs backwards**. A note already
//! scored must not be scored again, and lyrics must not jump back mid-word.

use std::time::Duration;

/// Weight given to the running average when a new drift measurement arrives.
///
/// High, because individual measurements are noisy and the thing being corrected — clock
/// drift — changes slowly.
const AVERAGE_WEIGHT: f64 = 0.7;

/// Drift beyond which the clock is nudged forward, in seconds.
const FORWARD_THRESHOLD: f64 = 0.010;

/// Drift beyond which the clock is paused to let the audio catch up, in seconds.
const PAUSE_THRESHOLD: f64 = 0.010;

/// Drift so large that gradual correction is pointless, in seconds.
const RESYNC_THRESHOLD: f64 = 5.0;

/// Default compensation for the round trip from singing to a detected pitch, in seconds.
///
/// Covers the microphone, the capture buffer and the analysis window. Adjustable per setup,
/// because USB interfaces vary by more than this value does.
pub const DEFAULT_MIC_DELAY: f64 = 0.140;

/// How the beat streams relate to the song's timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    /// `#BPM` multiplied by four — the rate the beat grid actually advances at.
    pub grid_rate: f64,
    /// `#GAP` in seconds: how long before the first beat the audio starts.
    pub gap: f64,
    /// Compensation applied to the detection stream only.
    pub mic_delay: f64,
}

impl Timing {
    pub fn new(bpm_file_value: f64, gap_ms: f64) -> Self {
        Self {
            grid_rate: bpm_file_value * 4.0,
            gap: gap_ms / 1000.0,
            mic_delay: DEFAULT_MIC_DELAY,
        }
    }

    fn beats(self, seconds: f64) -> f64 {
        seconds * self.grid_rate / 60.0
    }
}

/// Where the song is, expressed the three different ways the game needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beats {
    /// Drives lyrics and note drawing.
    pub visual: f64,
    /// Drives the metronome and click track. Same as `visual`, kept separate so it can be
    /// offset independently without disturbing what is drawn.
    pub click: f64,
    /// Drives scoring.
    ///
    /// Offset by the microphone delay *and* half a beat: a pitch detected now describes sound
    /// that happened a moment ago, and the analysis window straddles the boundary rather than
    /// starting at it.
    pub detection: f64,
}

/// A monotonic song clock that follows an external audio position.
#[derive(Debug, Clone)]
pub struct MasterClock {
    timing: Timing,
    /// Current position in the song, in seconds from the start of the audio.
    position: f64,
    average_drift: f64,
    paused: bool,
    /// Whether the clock is being held still to let the audio catch up.
    holding: bool,
    started: bool,
}

impl MasterClock {
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            position: 0.0,
            average_drift: 0.0,
            paused: true,
            holding: false,
            started: false,
        }
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = timing;
    }

    /// Current position in seconds from the start of the audio.
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn start(&mut self) {
        self.paused = false;
        self.started = true;
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Jump to a position, discarding accumulated drift. Used for seeking and restarts.
    pub fn seek(&mut self, position: f64) {
        self.position = position;
        self.average_drift = 0.0;
        self.holding = false;
    }

    /// Advance by the elapsed wall-clock time.
    pub fn tick(&mut self, elapsed: Duration) {
        if self.paused || self.holding {
            return;
        }
        self.position += elapsed.as_secs_f64();
    }

    /// Reconcile against the audio device's reported position.
    ///
    /// Returns the drift that was acted on, for diagnostics.
    pub fn synchronize(&mut self, audio_position: f64) -> f64 {
        if !self.started {
            self.position = audio_position;
            return 0.0;
        }
        let drift = audio_position - self.position;

        // A gap this large is not drift — it is a seek, a stall, or a decoder restart, and
        // averaging it in would take many seconds to work off.
        if drift.abs() > RESYNC_THRESHOLD {
            self.seek(audio_position);
            return drift;
        }

        self.average_drift = drift * (1.0 - AVERAGE_WEIGHT) + self.average_drift * AVERAGE_WEIGHT;

        if self.average_drift > FORWARD_THRESHOLD {
            // Audio is ahead: step forward to meet it and start averaging afresh.
            self.position += self.average_drift;
            self.average_drift = 0.0;
            self.holding = false;
        } else if self.average_drift < -PAUSE_THRESHOLD {
            // Audio is behind. Hold rather than step back, because stepping back would
            // re-play beats that have already been scored.
            self.holding = true;
        } else {
            // Back inside the dead band, so run normally again. Note the condition is drift
            // leaving the band, not reaching zero: the running average approaches zero
            // asymptotically and would never actually arrive, leaving the clock held for
            // good. UltraStar Deluxe waits for zero and has exactly that hazard.
            self.holding = false;
        }
        self.average_drift
    }

    /// The three beat positions for the current moment.
    pub fn beats(&self) -> Beats {
        let lyric_time = self.position - self.timing.gap;
        Beats {
            visual: self.timing.beats(lyric_time),
            click: self.timing.beats(lyric_time),
            detection: self.detection_beat(self.timing.mic_delay),
        }
    }

    /// The detection beat for a microphone lagging by `mic_delay` seconds.
    ///
    /// Asked for rather than held, because singers do not share a microphone. A USB mic and a
    /// Bluetooth headset are hundreds of milliseconds apart, and one delay applied to both
    /// puts every hit of one singer in the wrong place — which looks to them like the game
    /// not hearing them rather than like a setting.
    pub fn detection_beat(&self, mic_delay: f64) -> f64 {
        let lyric_time = self.position - self.timing.gap;
        -0.5 + self.timing.beats(lyric_time - mic_delay)
    }

    /// Whole beats crossed since `previous`, for driving per-beat scoring.
    ///
    /// Returns an empty range when time has not moved on, so a caller that polls faster than
    /// the beat rate simply gets nothing to do.
    pub fn beats_crossed(previous: f64, current: f64) -> std::ops::RangeInclusive<i32> {
        let from = previous.floor() as i32 + 1;
        let to = current.floor() as i32;
        from..=to
    }
}
