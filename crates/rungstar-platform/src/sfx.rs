//! Interface sounds.
//!
//! A small mixer of its own rather than another [`crate::Playback`]. A song is one long stream
//! read from a decoder; an interface sound is a fifty-millisecond sample that has to start
//! *now*, on top of whatever else is playing, possibly three at once when somebody holds a
//! direction. Those are different problems and the second one is much easier.
//!
//! So: everything is decoded into memory once, and a fixed-size mixing buffer is filled every
//! frame and pushed to its own output stream. Playing a sound is adding it into that buffer,
//! which cannot fail, cannot block and cannot allocate.
//!
//! **A sound is never a reason for anything to go wrong.** A missing file, a device that will
//! not open, a machine with no sound card at all: every one of those leaves a working game
//! that is quiet. Nothing here returns an error to a caller.

use std::collections::HashMap;
use std::path::Path;

use crate::assets::asset;

use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};
use sdl3::AudioSubsystem;

/// Which sound to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    /// The cursor moved. Heard more than everything else together.
    Move,
    Select,
    Back,
    /// A song is starting.
    Start,
    /// A golden note was hit.
    Golden,
    /// A line was sung well.
    Line,
    /// A song ended.
    Finish,
    /// Something was refused.
    No,
}

impl Sound {
    pub const ALL: [Sound; 8] = [
        Sound::Move,
        Sound::Select,
        Sound::Back,
        Sound::Start,
        Sound::Golden,
        Sound::Line,
        Sound::Finish,
        Sound::No,
    ];

    pub fn file(self) -> &'static str {
        match self {
            Self::Move => "move.wav",
            Self::Select => "select.wav",
            Self::Back => "back.wav",
            Self::Start => "start.wav",
            Self::Golden => "golden.wav",
            Self::Line => "line.wav",
            Self::Finish => "finish.wav",
            Self::No => "no.wav",
        }
    }
}

/// The output format. The sounds are generated at this rate, so nothing is resampled.
const RATE: i32 = 44_100;
/// How much is mixed ahead. Small enough that a sound starts within a frame of being asked
/// for, large enough that a late frame does not leave a gap.
const AHEAD: usize = RATE as usize / 20;

/// One sound in flight.
struct Playing {
    sound: Sound,
    /// Where in the sample the next frame comes from.
    at: usize,
    gain: f32,
}

/// The interface mixer.
pub struct Sfx {
    stream: Option<AudioStreamOwner>,
    samples: HashMap<Sound, Vec<i16>>,
    playing: Vec<Playing>,
    buffer: Vec<i16>,
    volume: f32,
}

impl Sfx {
    /// Open the device and read whatever sounds are shipped.
    ///
    /// Never fails. A machine with no sound card, a missing folder and a corrupt file all
    /// produce a working `Sfx` that happens to be silent.
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

        let mut samples = HashMap::new();
        for sound in Sound::ALL {
            match asset("sounds", sound.file())
                .ok_or_else(|| "not there".to_owned())
                .and_then(|path| read_wav(&path))
            {
                Ok(data) => {
                    samples.insert(sound, data);
                }
                Err(error) => tracing::debug!("no {} sound: {error}", sound.file()),
            }
        }
        if samples.is_empty() {
            tracing::info!("no interface sounds found; the game will be quiet");
        }

        Self {
            stream,
            samples,
            playing: Vec::new(),
            buffer: vec![0; AHEAD],
            volume: 1.0,
        }
    }

    /// How loud the interface is, `0.0..=1.0`. Zero silences it entirely.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// How many of the interface sounds were found and are usable.
    ///
    /// Reported by `--check` rather than only logged: a release that ships without its sounds
    /// looks exactly like one that ships with them until somebody runs it.
    pub fn loaded(&self) -> usize {
        self.samples.len()
    }

    /// Whether the output device opened.
    pub fn has_device(&self) -> bool {
        self.stream.is_some()
    }

    /// How much audio is waiting to be played, in bytes.
    ///
    /// Exists so `--check` can prove a sound actually reached the device rather than that a
    /// file parsed — the two failed independently while this was being built.
    pub fn queued(&self) -> usize {
        self.stream
            .as_ref()
            .and_then(|stream| stream.queued_bytes().ok())
            .unwrap_or(0)
            .max(0) as usize
    }

    /// Whether anything is loaded, for the settings screen to be honest about.
    pub fn is_silent(&self) -> bool {
        self.samples.is_empty() || self.stream.is_none() || self.volume <= 0.0
    }

    /// Start a sound.
    pub fn play(&mut self, sound: Sound) {
        self.play_at(sound, 1.0);
    }

    /// Start a sound at a fraction of the interface volume.
    pub fn play_at(&mut self, sound: Sound, gain: f32) {
        if self.is_silent() || !self.samples.contains_key(&sound) {
            return;
        }
        // A cap on how many overlap. Holding a direction on a fast list can ask for one every
        // frame, and forty copies of the same blip is not louder, it is distortion.
        const AT_ONCE: usize = 8;
        if self.playing.len() >= AT_ONCE {
            self.playing.remove(0);
        }
        // The same sound retriggered replaces its own oldest copy rather than stacking with
        // it: two identical samples a frame apart is a flam, which sounds like a fault.
        if let Some(index) = self
            .playing
            .iter()
            .position(|held| held.sound == sound && held.at < RATE as usize / 200)
        {
            self.playing.remove(index);
        }
        self.playing.push(Playing {
            sound,
            at: 0,
            gain: gain.clamp(0.0, 1.0),
        });
    }

    /// Mix and push. Called once a frame.
    pub fn tick(&mut self) {
        let Some(stream) = &self.stream else {
            return;
        };
        if self.playing.is_empty() {
            return;
        }
        // Only ever get as far ahead as the buffer: a stream that has fallen behind should be
        // caught up by the device draining it, not by pushing more into it.
        if stream.queued_bytes().unwrap_or(0) as usize > AHEAD * 2 {
            return;
        }

        self.buffer.fill(0);
        let volume = self.volume;
        for voice in &mut self.playing {
            let Some(sample) = self.samples.get(&voice.sound) else {
                continue;
            };
            let gain = voice.gain * volume;
            let taken = (sample.len() - voice.at).min(self.buffer.len());
            for (out, value) in self
                .buffer
                .iter_mut()
                .zip(&sample[voice.at..voice.at + taken])
            {
                // Saturating, so a pile-up clips rather than wrapping — wrapping turns a loud
                // moment into a burst of noise, which is the worst possible failure here.
                *out = out.saturating_add((*value as f32 * gain) as i16);
            }
            voice.at += taken;
        }
        self.playing.retain(|voice| {
            self.samples
                .get(&voice.sound)
                .is_some_and(|s| voice.at < s.len())
        });

        let _ = stream.put_data_i16(&self.buffer);
    }
}

/// Read a 16-bit PCM WAV.
///
/// Hand-written rather than a dependency, because the only files it ever reads are the ones
/// `tools/make-sounds.py` writes: mono, 16-bit, 44.1 kHz, no extensible header. Anything else
/// is refused rather than guessed at.
fn read_wav(path: &Path) -> Result<Vec<i16>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".to_owned());
    }
    let word =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let short = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);

    // Walk the chunks rather than assuming the layout: a writer is free to put a LIST before
    // the data, and Python's `wave` module has been known to.
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = word(at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            format = Some((
                short(body),
                short(body + 2),
                word(body + 4),
                short(body + 14),
            ));
        } else if id == b"data" {
            let (encoding, channels, rate, bits) =
                format.ok_or_else(|| "no format chunk".to_owned())?;
            if encoding != 1 || bits != 16 {
                return Err(format!("not 16-bit PCM (encoding {encoding}, {bits} bits)"));
            }
            if channels != 1 {
                return Err(format!("expected mono, got {channels} channels"));
            }
            if rate as i32 != RATE {
                return Err(format!("expected {RATE} Hz, got {rate}"));
            }
            let end = (body + size).min(bytes.len());
            return Ok(bytes[body..end]
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect());
        }
        // Chunks are padded to an even length.
        at = body + size + (size & 1);
    }
    Err("no data chunk".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped(sound: Sound) -> Vec<i16> {
        let path = asset("sounds", sound.file())
            .unwrap_or_else(|| panic!("{} is missing from assets/sounds", sound.file()));
        read_wav(&path).unwrap_or_else(|e| panic!("{} is not usable: {e}", sound.file()))
    }

    #[test]
    fn every_sound_is_shipped_and_the_mixer_will_take_it() {
        // A WAV the reader refuses is silence at runtime and nothing at all in a log anybody
        // reads, so the format is asserted here rather than discovered on a player's machine.
        for sound in Sound::ALL {
            assert!(!shipped(sound).is_empty(), "{} is empty", sound.file());
        }
    }

    #[test]
    fn nothing_clips_and_nothing_is_inaudible() {
        for sound in Sound::ALL {
            let data = shipped(sound);
            let peak = data.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            // Headroom, because up to eight of these are summed into one buffer.
            assert!(
                peak < 30_000,
                "{} peaks at {peak}, which leaves nothing for anything mixed with it",
                sound.file()
            );
            assert!(peak > 3_000, "{} is too quiet to hear", sound.file());
        }
    }

    #[test]
    fn nothing_starts_or_ends_on_a_click() {
        // A sample that begins away from zero is a step change, which is a click — and these
        // play hundreds of times an hour, over music, which is where a click is most obvious.
        for sound in Sound::ALL {
            let data = shipped(sound);
            assert!(
                data[0].abs() < 200 && data[data.len() - 1].abs() < 200,
                "{} starts at {} and ends at {}",
                sound.file(),
                data[0],
                data[data.len() - 1]
            );
        }
    }

    #[test]
    fn the_menu_blip_is_short() {
        // It is heard more than everything else together. Anything long enough to still be
        // playing at the next press turns a fast scroll into a drone.
        let seconds = shipped(Sound::Move).len() as f32 / RATE as f32;
        assert!(seconds < 0.12, "the move sound lasts {seconds:.2}s");
    }

    #[test]
    fn a_sound_the_music_has_to_survive_is_quieter_than_a_menu_one() {
        // Golden notes and line bonuses land on top of somebody singing. A confirm does not.
        let loudest = |sound| {
            shipped(sound)
                .iter()
                .map(|s| s.unsigned_abs())
                .max()
                .unwrap_or(0)
        };
        assert!(loudest(Sound::Golden) < loudest(Sound::Select));
        assert!(loudest(Sound::Line) < loudest(Sound::Select));
    }

    #[test]
    fn a_wav_that_is_not_what_the_mixer_wants_is_refused() {
        // Refused rather than guessed at: a stereo or 48 kHz file played as mono 44.1 comes
        // out at the wrong pitch and half the length, which sounds like a bug in the game
        // rather than in the file.
        let dir = std::env::temp_dir().join("rungstar-sfx-test");
        std::fs::create_dir_all(&dir).unwrap();

        let mut stereo = Vec::new();
        stereo.extend_from_slice(b"RIFF");
        stereo.extend_from_slice(&40u32.to_le_bytes());
        stereo.extend_from_slice(b"WAVEfmt ");
        stereo.extend_from_slice(&16u32.to_le_bytes());
        stereo.extend_from_slice(&1u16.to_le_bytes()); // PCM
        stereo.extend_from_slice(&2u16.to_le_bytes()); // two channels
        stereo.extend_from_slice(&(RATE as u32).to_le_bytes());
        stereo.extend_from_slice(&(RATE as u32 * 4).to_le_bytes());
        stereo.extend_from_slice(&4u16.to_le_bytes());
        stereo.extend_from_slice(&16u16.to_le_bytes());
        stereo.extend_from_slice(b"data");
        stereo.extend_from_slice(&4u32.to_le_bytes());
        stereo.extend_from_slice(&[0, 0, 0, 0]);

        let path = dir.join("stereo.wav");
        std::fs::write(&path, &stereo).unwrap();
        let error = read_wav(&path).unwrap_err();
        assert!(error.contains("mono"), "{error}");

        std::fs::write(&path, b"not a wav at all, not even close").unwrap();
        assert!(read_wav(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
