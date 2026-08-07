//! Microphone capture through SDL3.
//!
//! Every configured device gets its own pull-based stream, polled from the game loop rather
//! than delivered on a callback thread. That is deliberate: a callback would have to hand
//! samples across a lock to the analysis code, and the lock would sit in the audio driver's
//! real-time path. Polling costs a memcpy and cannot cause a dropout.

use rungstar_audio::{AudioError, CaptureBackend, DeviceConfig, DeviceInfo, PlayerBuffers};
use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};
use sdl3::AudioSubsystem;

/// One open device and what its channels feed.
struct OpenDevice {
    config: DeviceConfig,
    stream: AudioStreamOwner,
}

/// SDL3-backed microphone capture.
pub struct SdlCapture {
    audio: AudioSubsystem,
    open: Vec<OpenDevice>,
    /// Reused between drains so steady-state capture does not allocate.
    scratch: Vec<i16>,
}

impl SdlCapture {
    pub fn new(audio: AudioSubsystem) -> Self {
        Self {
            audio,
            open: Vec::new(),
            scratch: vec![0; 8192],
        }
    }

    /// Find a device by the name the backend reported.
    ///
    /// Names are matched rather than ids because ids are reassigned when hardware is
    /// re-plugged, and a saved setup has to survive that.
    fn find(&self, name: &str) -> Result<sdl3::audio::AudioDeviceID, AudioError> {
        let ids = self
            .audio
            .audio_recording_device_ids()
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        ids.into_iter()
            .find(|id| id.name().is_ok_and(|n| n == name))
            .ok_or_else(|| AudioError::DeviceNotFound(name.to_owned()))
    }
}

impl CaptureBackend for SdlCapture {
    fn devices(&self) -> Result<Vec<DeviceInfo>, AudioError> {
        let ids = self
            .audio
            .audio_recording_device_ids()
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                let name = id.name().ok()?;
                // SDL does not report a channel count before the device is opened. Two is
                // the useful assumption: the dual-microphone adapters are all stereo, and a
                // mono device simply leaves its second channel unmapped.
                Some(DeviceInfo {
                    name,
                    channels: 2,
                    sample_rates: Vec::new(),
                })
            })
            .collect())
    }

    fn start(&mut self, configs: &[DeviceConfig], sample_rate: u32) -> Result<(), AudioError> {
        self.stop();
        for config in configs {
            // Nothing to do for a device with every channel switched off.
            if config
                .channel_to_player
                .iter()
                .all(|p| *p == rungstar_audio::CHANNEL_OFF)
            {
                continue;
            }
            let id = self.find(&config.name)?;
            let spec = AudioSpec {
                freq: Some(sample_rate as i32),
                channels: Some(config.channels() as i32),
                format: Some(AudioFormat::S16LE),
            };
            let device = AudioDevice::open_recording(&self.audio, &id, &spec).map_err(|e| {
                AudioError::UnsupportedFormat {
                    device: config.name.clone(),
                    reason: e.to_string(),
                }
            })?;
            let stream = device
                .open_device_stream(Some(&spec))
                .map_err(|e| AudioError::Backend(e.to_string()))?;
            stream
                .resume()
                .map_err(|e| AudioError::Backend(e.to_string()))?;
            self.open.push(OpenDevice {
                config: config.clone(),
                stream,
            });
        }
        Ok(())
    }

    fn stop(&mut self) {
        for device in &self.open {
            let _ = device.stream.pause();
        }
        self.open.clear();
    }

    fn drain(&mut self, out: &mut PlayerBuffers) -> Result<(), AudioError> {
        for device in &mut self.open {
            loop {
                let available = device
                    .stream
                    .available_bytes()
                    .map_err(|e| AudioError::Backend(e.to_string()))?;
                if available <= 0 {
                    break;
                }
                // Whole frames only, so a block never splits a stereo pair across two reads.
                let channels = device.config.channels().max(1);
                let samples = (available as usize / 2).min(self.scratch.len());
                let frames = samples / channels;
                if frames == 0 {
                    break;
                }
                let wanted = frames * channels;
                let read = device
                    .stream
                    .read_i16_samples(&mut self.scratch[..wanted])
                    .map_err(|e| AudioError::Backend(e.to_string()))?;
                if read == 0 {
                    break;
                }
                out.route(&device.config, &self.scratch[..read]);
            }
        }
        Ok(())
    }
}
