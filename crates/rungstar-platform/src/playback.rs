//! Playing a song, and knowing exactly where it is.
//!
//! Position is the whole point. Scoring compares when a note should be sung against when a
//! sound arrived, so "how far into the song are we" has to be answered in milliseconds, from
//! the device rather than from a wall clock that drifts against it.
//!
//! SDL is fed from the game loop rather than a callback: the amount still queued in the
//! device is exactly the amount pushed but not yet heard, which makes the played position a
//! subtraction instead of an estimate.

use rungstar_audio::{AudioClip, AudioError};
use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};
use sdl3::AudioSubsystem;

/// How much audio to keep queued in the device.
///
/// Enough to survive a slow frame, short enough that a pause or a seek takes effect without a
/// noticeable tail.
const TARGET_QUEUE_SECS: f64 = 0.10;

/// A song being played.
pub struct Playback {
    stream: AudioStreamOwner,
    clip: AudioClip,
    channels: usize,
    sample_rate: u32,
    /// Total frames handed to the device since the last seek.
    pushed_frames: usize,
    /// Frames already played before the last seek, so position survives seeking.
    base_frames: usize,
    playing: bool,
    scratch: Vec<i16>,
}

impl Playback {
    /// Open an output device matching the clip's format.
    pub fn new(audio: &AudioSubsystem, clip: AudioClip) -> Result<Self, AudioError> {
        let spec = AudioSpec {
            freq: Some(clip.sample_rate() as i32),
            channels: Some(clip.channels() as i32),
            format: Some(AudioFormat::S16LE),
        };
        let device = AudioDevice::open_playback(audio, None, &spec)
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        let stream = device
            .open_device_stream(Some(&spec))
            .map_err(|e| AudioError::Backend(e.to_string()))?;

        let channels = clip.channels().max(1);
        let sample_rate = clip.sample_rate().max(1);
        Ok(Self {
            stream,
            clip,
            channels,
            sample_rate,
            pushed_frames: 0,
            base_frames: 0,
            playing: false,
            scratch: vec![0; channels * sample_rate as usize / 4],
        })
    }

    pub fn clip(&self) -> &AudioClip {
        &self.clip
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn start(&mut self) -> Result<(), AudioError> {
        self.stream
            .resume()
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        self.playing = true;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), AudioError> {
        self.stream
            .pause()
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        self.playing = false;
        Ok(())
    }

    /// Set the volume, `0.0..=1.0`.
    pub fn set_volume(&self, volume: f32) {
        let _ = self.stream.set_gain(volume.clamp(0.0, 1.0));
    }

    /// Jump to a position in seconds.
    ///
    /// Whatever is already queued is discarded, or the old audio would keep playing for a
    /// tenth of a second after the picture had moved.
    pub fn seek(&mut self, secs: f64) -> Result<(), AudioError> {
        let frame = (secs.max(0.0) * f64::from(self.sample_rate)) as usize;
        self.stream
            .clear()
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        self.base_frames = frame;
        self.pushed_frames = 0;
        Ok(())
    }

    /// Frames still sitting in the device, waiting to be heard.
    fn queued_frames(&self) -> usize {
        let bytes = self.stream.queued_bytes().unwrap_or(0).max(0) as usize;
        bytes / (2 * self.channels)
    }

    /// Where the song actually is, in seconds.
    pub fn position(&self) -> f64 {
        let played = self.base_frames + self.pushed_frames.saturating_sub(self.queued_frames());
        played as f64 / f64::from(self.sample_rate)
    }

    /// Top the device's queue back up. Call once per frame.
    ///
    /// A short read means the decoder has not caught up yet; that is left alone rather than
    /// padded with silence, so the audio simply waits instead of developing a gap.
    pub fn pump(&mut self) -> Result<(), AudioError> {
        if !self.playing {
            return Ok(());
        }
        let target = (TARGET_QUEUE_SECS * f64::from(self.sample_rate)) as usize;
        let queued = self.queued_frames();
        if queued >= target {
            return Ok(());
        }

        let wanted = (target - queued).min(self.scratch.len() / self.channels);
        let from = self.base_frames + self.pushed_frames;
        let got = self
            .clip
            .read(from, &mut self.scratch[..wanted * self.channels]);
        if got == 0 {
            return Ok(());
        }
        self.stream
            .put_data_i16(&self.scratch[..got * self.channels])
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        self.pushed_frames += got;
        Ok(())
    }

    /// Whether the song has been decoded in full and played out.
    pub fn is_finished(&self) -> bool {
        self.clip.is_complete()
            && self.base_frames + self.pushed_frames >= self.clip.ready_frames()
            && self.queued_frames() == 0
    }

    /// Total length in seconds, as far as the decoder currently knows.
    pub fn duration(&self) -> f64 {
        self.clip.ready_secs()
    }
}
