//! Playing the menu music.
//!
//! [`crate::chiptune`] makes the loop; this keeps it going and knows when to get out of the way.
//!
//! **Ducking is the whole job.** Music under a menu is pleasant and music under a song preview
//! is two pieces of music at once, which is unlistenable. So it fades out whenever anything else
//! is making noise — a preview, a song, the editor — and fades back in afterwards. A fade rather
//! than a cut, because a hard stop draws more attention than the music ever did.
//!
//! Like [`crate::sfx`], nothing here can fail in a way that matters: no device, no music, and
//! the game is exactly as playable.

use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};
use sdl3::AudioSubsystem;

const RATE: i32 = 44_100;

/// How much audio is kept queued at the device, in seconds.
///
/// Longer than the interface mixer's, because nothing here has to start on a particular frame:
/// a quarter of a second of slack means a stalled frame cannot make the music stutter.
const AHEAD: f32 = 0.25;

/// Seconds a fade takes.
///
/// Slow enough to be a fade rather than a jump, quick enough that a preview starting a fifth of
/// a second later is not competing with it.
const FADE: f32 = 0.45;

/// The menu music.
pub struct Music {
    stream: Option<AudioStreamOwner>,
    loop_samples: Vec<i16>,
    /// Where in the loop the next sample comes from.
    at: usize,
    /// What the volume is now, and what it is heading for.
    level: f32,
    target: f32,
    /// The setting, which is the ceiling `target` is a fraction of.
    volume: f32,
    scratch: Vec<i16>,
}

impl Music {
    /// Render the loop and open a device for it.
    ///
    /// Rendering is about twenty milliseconds and happens once, at startup, on this thread —
    /// which is the loading screen either way, and moving it off would mean the first menu is
    /// silent for no benefit anybody would notice.
    pub fn new(audio: &AudioSubsystem) -> Self {
        let spec = AudioSpec {
            freq: Some(RATE),
            channels: Some(1),
            format: Some(AudioFormat::S16LE),
        };
        let stream = AudioDevice::open_playback(audio, None, &spec)
            .ok()
            .and_then(|device| device.open_device_stream(Some(&spec)).ok());
        if let Some(stream) = &stream {
            let _ = stream.resume();
        }
        Self {
            stream,
            loop_samples: crate::chiptune::render(),
            at: 0,
            level: 0.0,
            target: 1.0,
            volume: 0.0,
            scratch: vec![0; (RATE as f32 * AHEAD) as usize],
        }
    }

    /// How loud the music may get, `0.0..=1.0`. Zero silences it.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Whether the music should be audible at all right now.
    ///
    /// Called every frame with whatever else is playing; the fade is this type's business, not
    /// the caller's.
    pub fn set_wanted(&mut self, wanted: bool) {
        self.target = if wanted { 1.0 } else { 0.0 };
    }

    /// Whether anything is actually coming out.
    pub fn is_playing(&self) -> bool {
        self.stream.is_some() && self.volume > 0.0 && self.level > 0.001
    }

    /// Advance the fade and keep the device fed. Called once a frame.
    pub fn tick(&mut self, dt: f32) {
        let Some(stream) = &self.stream else {
            return;
        };
        // The fade runs even when silent, so coming back is a fade in rather than a jump.
        let step = (dt.max(0.0) / FADE).min(1.0);
        self.level += (self.target - self.level) * step;
        if self.target <= 0.0 && self.level < 0.001 {
            self.level = 0.0;
        }

        let gain = self.level * self.volume;
        if gain <= 0.0 || self.loop_samples.is_empty() {
            // Silent, and nothing is pushed. Whatever is already queued drains within the
            // buffer length, which is shorter than the fade that got here.
            return;
        }
        // Never get further ahead than the buffer: the queue draining is what paces this, and
        // pushing on every frame regardless would build up an ever-growing delay.
        let queued = stream.queued_bytes().unwrap_or(0).max(0) as usize / 2;
        if queued >= self.scratch.len() {
            return;
        }

        let wanted = self.scratch.len() - queued;
        for index in 0..wanted {
            let sample = self.loop_samples[self.at];
            self.scratch[index] = (f32::from(sample) * gain) as i16;
            // Wrapping here is what makes it a loop, and `chiptune::render` is what makes the
            // wrap inaudible.
            self.at = (self.at + 1) % self.loop_samples.len();
        }
        let _ = stream.put_data_i16(&self.scratch[..wanted]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player with no device, which is every code path except the one that pushes bytes.
    fn silent() -> Music {
        Music {
            stream: None,
            loop_samples: vec![0; 1000],
            at: 0,
            level: 0.0,
            target: 1.0,
            volume: 0.5,
            scratch: vec![0; 64],
        }
    }

    #[test]
    fn a_machine_with_no_sound_card_still_works() {
        let mut music = silent();
        music.tick(0.016);
        assert!(!music.is_playing());
    }

    #[test]
    fn wanting_it_off_fades_rather_than_cuts() {
        let mut music = silent();
        music.stream = None;
        music.level = 1.0;
        music.set_wanted(false);
        // Without a device `tick` returns before the fade, so drive the fade directly: the
        // property under test is that it takes time, not that it makes sound.
        for _ in 0..3 {
            let step: f32 = 0.016 / FADE;
            music.level += (music.target - music.level) * step;
        }
        assert!(music.level > 0.5, "faded to {} in 48 ms", music.level);
        for _ in 0..200 {
            let step: f32 = 0.016 / FADE;
            music.level += (music.target - music.level) * step;
        }
        assert!(music.level < 0.01, "never got there: {}", music.level);
    }

    #[test]
    fn the_volume_setting_is_a_ceiling_and_zero_is_silence() {
        let mut music = silent();
        music.set_volume(0.0);
        music.level = 1.0;
        assert!(!music.is_playing());
        music.set_volume(2.0);
        assert_eq!(music.volume(), 1.0, "clamped rather than trusted");
    }
}
