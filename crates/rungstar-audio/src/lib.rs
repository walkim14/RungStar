//! Audio timing and capture.
//!
//! Two responsibilities, both of which the rest of the game depends on being right:
//!
//! * [`clock`] answers "how far into the song are we", which is the question scoring and
//!   lyric drawing are both really asking.
//! * [`capture`] gets each singer's microphone onto its own buffer, including the
//!   two-players-per-stereo-device arrangement that karaoke hardware actually uses.
//!
//! Device I/O lives behind [`CaptureBackend`] so the timing and routing logic can be tested
//! without a sound card — which is most of what there is to get wrong.

#![forbid(unsafe_code)]

pub mod capture;
pub mod clock;
pub mod decode;
pub mod loudness;

pub use capture::{
    validate, ConfigProblem, DeviceConfig, PlayerBuffers, CHANNEL_OFF, LATENCY_AUTODETECT,
    MAX_PLAYERS,
};
pub use clock::{Beats, MasterClock, Timing, DEFAULT_MIC_DELAY};
pub use decode::{AudioClip, DecodeError};

/// A capture device the backend has found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    /// Which device of this name it is, counting from zero in enumeration order.
    ///
    /// **Two identical microphones report identical names**, which is exactly what a pair of
    /// the same USB karaoke mics is, and it is the common case rather than a curiosity. Without
    /// this they are one device as far as everything downstream is concerned: both singers are
    /// routed to whichever one the backend happened to list first, and the second microphone is
    /// dead with no indication why.
    pub occurrence: u32,
    pub channels: usize,
    /// Rates the device will accept, best first. Empty when the backend does not say.
    pub sample_rates: Vec<u32>,
}

impl DeviceInfo {
    /// Whether this device could carry two singers on its own.
    pub fn supports_two_players(&self) -> bool {
        self.channels >= 2
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio backend available")]
    NoBackend,
    #[error("capture device '{0}' not found")]
    DeviceNotFound(String),
    #[error("device '{device}' rejected the requested format: {reason}")]
    UnsupportedFormat { device: String, reason: String },
    #[error("audio backend failed: {0}")]
    Backend(String),
}

/// Somewhere microphone audio comes from.
///
/// Implemented over the real sound system, and over a canned source in tests. Blocks arrive
/// as interleaved 16-bit samples, which is what every backend can produce and what the pitch
/// detector wants.
pub trait CaptureBackend {
    /// Devices currently attached.
    fn devices(&self) -> Result<Vec<DeviceInfo>, AudioError>;

    /// Begin capturing from every configured device.
    fn start(&mut self, configs: &[DeviceConfig], sample_rate: u32) -> Result<(), AudioError>;

    /// Stop capturing and release the devices.
    fn stop(&mut self);

    /// Drain whatever has been captured since the last call into `out`.
    ///
    /// Non-blocking: an empty result means nothing new has arrived, not that anything failed.
    fn drain(&mut self, out: &mut PlayerBuffers) -> Result<(), AudioError>;
}

/// A backend that plays back canned audio, for tests and for running without a sound card.
#[derive(Debug, Default)]
pub struct ScriptedCapture {
    configs: Vec<DeviceConfig>,
    /// Interleaved blocks to hand out, one per `drain`, per device.
    scripted: Vec<Vec<Vec<i16>>>,
    position: usize,
    running: bool,
}

impl ScriptedCapture {
    /// Build a backend that will replay `blocks[device][call]` on successive drains.
    pub fn new(blocks: Vec<Vec<Vec<i16>>>) -> Self {
        Self {
            configs: Vec::new(),
            scripted: blocks,
            position: 0,
            running: false,
        }
    }
}

impl CaptureBackend for ScriptedCapture {
    fn devices(&self) -> Result<Vec<DeviceInfo>, AudioError> {
        Ok(self
            .scripted
            .iter()
            .enumerate()
            .map(|(i, _)| DeviceInfo {
                name: format!("Scripted {i}"),
                occurrence: 0,
                channels: 2,
                sample_rates: vec![44_100],
            })
            .collect())
    }

    fn start(&mut self, configs: &[DeviceConfig], _sample_rate: u32) -> Result<(), AudioError> {
        self.configs = configs.to_vec();
        self.position = 0;
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn drain(&mut self, out: &mut PlayerBuffers) -> Result<(), AudioError> {
        if !self.running {
            return Ok(());
        }
        for (device, config) in self.configs.iter().enumerate() {
            if let Some(block) = self.scripted.get(device).and_then(|b| b.get(self.position)) {
                out.route(config, block);
            }
        }
        self.position += 1;
        Ok(())
    }
}
