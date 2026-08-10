//! Running the microphone delay measurement against real hardware.
//!
//! [`rungstar_audio::latency`] is the arithmetic and is tested without a sound card. This is the
//! part that needs one: play the sweep, record what comes back, and hand the two to it.
//!
//! **The two timelines have to be pinned to the same instant**, or the answer is out by
//! whatever the pinning was wrong by. The trick is the order of three lines: drain the capture,
//! push the sweep, keep draining. Everything the microphone had already heard by the moment of
//! the push is behind that mark, so the sweep's position after it is the delay and nothing else.
//!
//! What it measures is the device's own output buffer, the trip through the air, and the whole
//! capture path. Not SDL's output queue — the song clock is taken from frames the device has
//! *consumed*, so that part is already accounted for and counting it again would double it.
//!
//! **It is stepped rather than run.** Five passes on each of several microphones is fifteen
//! seconds, and a call that blocks for fifteen seconds is a frozen game: no meter, no pass
//! count, no way to tell it from a crash. [`Calibrator::tick`] does a few milliseconds of work
//! and returns, so the screen keeps drawing and can say what is happening while it happens.

use std::time::{Duration, Instant};

use rungstar_audio::latency::{self, Heard};
use rungstar_audio::{CaptureBackend, DeviceConfig, PlayerBuffers};
use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};
use sdl3::AudioSubsystem;

use crate::SdlCapture;

const RATE: u32 = 44_100;

/// How many times the sweep is played at each microphone.
///
/// An odd number, so a median has a middle, and enough that a majority has to agree.
pub const PASSES: usize = 5;

/// How long the microphone is listened to and thrown away before each sweep.
///
/// Long enough for a device that has just been opened to settle: the first blocks out of one
/// are often a burst or silence, and either would be correlated against.
const SETTLE: Duration = Duration::from_millis(250);

/// What one microphone came to.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub name: String,
    /// The delay in milliseconds, or why there is not one.
    pub settled: Result<f32, String>,
    /// Every pass, kept whether or not they added up to an answer: "it did not work" and "it
    /// did not work, and here is what each attempt heard" are very different things to be
    /// handed when the microphone is across the room from the speakers.
    pub passes: Vec<Heard>,
}

impl Outcome {
    /// The loudest thing any pass recorded, which says whether the microphone works at all.
    pub fn loudest(&self) -> f32 {
        self.passes.iter().map(|p| p.level).fold(0.0, f32::max)
    }
}

/// What the measurement is doing right now, for a screen to show.
#[derive(Debug, Clone)]
pub struct Progress {
    /// The microphone being measured.
    pub device: String,
    /// Which microphone of how many.
    pub device_index: usize,
    pub devices: usize,
    /// Which pass of how many, one-based.
    pub pass: usize,
    pub passes: usize,
    /// Whether the sweep is playing, or the room is being listened to beforehand.
    pub playing: bool,
    /// The loudest thing heard so far this pass, so the meter moves when somebody talks and
    /// sits still when the microphone is dead.
    pub level: f32,
}

enum Stage {
    /// Listening and discarding, so the device settles and the buffer is empty.
    Settling,
    /// The sweep has been pushed; everything recorded from here is the measurement.
    Listening,
}

struct Running {
    device: DeviceConfig,
    capture: SdlCapture,
    buffers: PlayerBuffers,
    stage: Stage,
    until: Instant,
    recorded: Vec<i16>,
    heard: Vec<Heard>,
    level: f32,
}

/// A measurement in progress across one or more microphones.
pub struct Calibrator {
    audio: AudioSubsystem,
    output: AudioStreamOwner,
    reference: Vec<i16>,
    /// Still to measure, in reverse so the next one pops off the end.
    queue: Vec<DeviceConfig>,
    total: usize,
    current: Option<Running>,
    done: Vec<Outcome>,
    /// Anything that stopped it before it could start, which is a fault rather than a result.
    pub trouble: Option<String>,
}

impl Calibrator {
    /// Open the speakers and line up the microphones to measure.
    pub fn start(audio: &AudioSubsystem, devices: &[DeviceConfig]) -> Result<Self, String> {
        if devices.is_empty() {
            return Err("no microphone to measure".to_owned());
        }
        let spec = AudioSpec {
            freq: Some(RATE as i32),
            channels: Some(1),
            format: Some(AudioFormat::S16LE),
        };
        let output = AudioDevice::open_playback(audio, None, &spec)
            .map_err(|e| format!("no speakers to play through: {e}"))?
            .open_device_stream(Some(&spec))
            .map_err(|e| format!("no speakers to play through: {e}"))?;
        output.resume().map_err(|e| e.to_string())?;

        let mut queue = devices.to_vec();
        queue.reverse();
        Ok(Self {
            audio: audio.clone(),
            output,
            reference: latency::sweep(RATE),
            total: queue.len(),
            queue,
            current: None,
            done: Vec::new(),
            trouble: None,
        })
    }

    /// Do a few milliseconds of work. `false` once there is nothing left to do.
    pub fn tick(&mut self) -> bool {
        if self.current.is_none() && !self.next_device() {
            return false;
        }
        let Some(running) = &mut self.current else {
            return false;
        };

        // Drained every tick whatever the stage: while settling this is what empties the
        // device, and while listening it is the measurement.
        if let Err(error) = running.capture.drain(&mut running.buffers) {
            self.finish_current(Err(error.to_string()));
            return true;
        }
        let fresh = running.buffers.player(1);
        running.level = fresh
            .iter()
            .map(|s| f32::from(s.saturating_abs()) / 32768.0)
            .fold(running.level, f32::max);
        if matches!(running.stage, Stage::Listening) {
            running.recorded.extend_from_slice(fresh);
        }
        running.buffers.clear();

        if Instant::now() < running.until {
            return true;
        }
        match running.stage {
            // Settled. The next three statements are the whole measurement, in this order and
            // with nothing between them: empty the microphone of everything up to now, start
            // the sweep, and from here on position in the recording *is* elapsed time.
            Stage::Settling => {
                running.recorded.clear();
                running.level = 0.0;
                if let Err(error) = self.output.put_data_i16(&self.reference) {
                    self.finish_current(Err(format!("could not play the sweep: {error}")));
                    return true;
                }
                let listen_for = latency::SWEEP_SECS + latency::MAX_DELAY_SECS + 0.2;
                running.stage = Stage::Listening;
                running.until = Instant::now() + Duration::from_secs_f32(listen_for);
            }
            Stage::Listening => {
                let heard = latency::find(&self.reference, &running.recorded, RATE);
                running.heard.push(heard.unwrap_or(Heard {
                    millis: 0.0,
                    confidence: 0.0,
                    level: running.level,
                }));
                if running.heard.len() >= PASSES {
                    let settled = latency::settle(&running.heard).map_err(str::to_owned);
                    self.finish_current(settled);
                } else {
                    running.stage = Stage::Settling;
                    running.until = Instant::now() + SETTLE;
                    running.level = 0.0;
                }
            }
        }
        true
    }

    /// What to put on screen, or `None` once it is finished.
    pub fn progress(&self) -> Option<Progress> {
        let running = self.current.as_ref()?;
        Some(Progress {
            device: running.device.name.clone(),
            device_index: self.done.len() + 1,
            devices: self.total,
            pass: running.heard.len() + 1,
            passes: PASSES,
            playing: matches!(running.stage, Stage::Listening),
            level: running.level,
        })
    }

    /// Everything measured so far.
    pub fn outcomes(&self) -> &[Outcome] {
        &self.done
    }

    fn next_device(&mut self) -> bool {
        let Some(device) = self.queue.pop() else {
            return false;
        };
        let listening = DeviceConfig {
            name: device.name.clone(),
            occurrence: device.occurrence,
            latency_ms: rungstar_audio::capture::LATENCY_AUTODETECT,
            // Every channel into one slot: which channel of a stereo microphone carries the
            // audio is not known here and does not matter, since only timing is being measured.
            channel_to_player: vec![1; device.channels().max(1)],
        };
        let mut capture = SdlCapture::new(self.audio.clone());
        if let Err(error) = capture.start(std::slice::from_ref(&listening), RATE) {
            self.done.push(Outcome {
                name: device.name.clone(),
                settled: Err(format!("would not open: {error}")),
                passes: Vec::new(),
            });
            // Straight on to the next one rather than giving up: one microphone refusing to
            // open should not stop the others being measured.
            return self.next_device();
        }
        self.current = Some(Running {
            device,
            capture,
            buffers: PlayerBuffers::new(),
            stage: Stage::Settling,
            until: Instant::now() + SETTLE,
            recorded: Vec::new(),
            heard: Vec::new(),
            level: 0.0,
        });
        true
    }

    fn finish_current(&mut self, settled: Result<f32, String>) {
        if let Some(mut running) = self.current.take() {
            running.capture.stop();
            self.done.push(Outcome {
                name: running.device.name.clone(),
                settled,
                passes: running.heard,
            });
        }
    }
}

impl Drop for Calibrator {
    fn drop(&mut self) {
        if let Some(running) = &mut self.current {
            running.capture.stop();
        }
        let _ = self.output.pause();
    }
}
