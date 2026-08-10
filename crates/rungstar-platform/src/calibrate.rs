//! Running the microphone delay measurement against real hardware.
//!
//! [`rungstar_audio::latency`] is the arithmetic and is tested without a sound card. This is the
//! part that needs one: play the sweep, record what comes back, and hand the two to it.
//!
//! **The two timelines have to be pinned to the same instant**, or the answer is out by
//! whatever the pinning was wrong by. The trick is the order of three lines: drain the capture
//! and note how many frames that is, then push the sweep, then keep draining. Everything the
//! microphone had already heard by the moment of the push is behind that mark, so the sweep's
//! position after it is the delay and nothing else.
//!
//! What it measures is the device's own output buffer, the trip through the air, and the whole
//! capture path. Not SDL's output queue — the song clock is taken from frames the device has
//! *consumed*, so that part is already accounted for and counting it again would double it.

use rungstar_audio::latency::{self, Heard};
use rungstar_audio::{CaptureBackend, DeviceConfig, PlayerBuffers};
use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec};
use sdl3::AudioSubsystem;

use crate::SdlCapture;

const RATE: u32 = 44_100;

/// How many times the sweep is played.
///
/// An odd number, so a median has a middle. Five takes about four seconds and is enough for a
/// majority to have to agree.
pub const PASSES: usize = 5;

/// What the whole measurement found.
///
/// The passes are kept whether or not they added up to an answer. "It did not work" and "it
/// did not work, and here is what each attempt heard" are very different things to be handed
/// when the microphone is in another room from the speakers.
#[derive(Debug, Clone)]
pub struct Calibration {
    /// The delay to use in milliseconds, or why there is not one.
    pub settled: Result<f32, String>,
    /// Every pass, in order.
    pub passes: Vec<Heard>,
}

/// Measure the delay on one capture device.
///
/// Plays a sweep out of the default output and listens for it. Needs **speakers**: with
/// headphones on there is nothing for the microphone to hear, and that is reported rather than
/// guessed around.
pub fn measure(
    audio: &AudioSubsystem,
    device: &DeviceConfig,
    passes: usize,
) -> Result<Calibration, String> {
    let reference = latency::sweep(RATE);

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

    // Every channel into one slot: which channel of a stereo microphone carries the audio is
    // not known here and does not matter, since only the timing is being measured.
    let listening = DeviceConfig {
        name: device.name.clone(),
        occurrence: device.occurrence,
        latency_ms: rungstar_audio::capture::LATENCY_AUTODETECT,
        channel_to_player: vec![1; device.channels().max(1)],
    };
    let mut capture = SdlCapture::new(audio.clone());
    capture
        .start(std::slice::from_ref(&listening), RATE)
        .map_err(|e| format!("could not listen on {}: {e}", device.name))?;

    let mut heard = Vec::with_capacity(passes);
    for _ in 0..passes {
        heard.push(one_pass(&output, &mut capture, &reference)?);
    }
    capture.stop();
    let _ = output.pause();

    // Only hardware faults are errors here. A measurement that ran and did not settle is a
    // result: it has five passes in it, and those say more than the sentence does.
    Ok(Calibration {
        settled: latency::settle(&heard).map_err(str::to_owned),
        passes: heard,
    })
}

/// One sweep, played and listened for.
fn one_pass(
    output: &sdl3::audio::AudioStreamOwner,
    capture: &mut SdlCapture,
    reference: &[i16],
) -> Result<Heard, String> {
    let mut buffers = PlayerBuffers::new();

    // Everything the microphone heard before now, thrown away. This is also what settles a
    // device that has just been opened: the first few blocks out of one are often silence or
    // a burst, and either would be correlated against.
    let settle_until = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < settle_until {
        std::thread::sleep(std::time::Duration::from_millis(10));
        capture.drain(&mut buffers).map_err(|e| e.to_string())?;
        buffers.clear();
    }

    // The three lines the whole measurement rests on, in this order and with nothing between
    // them. The last drain empties the microphone of everything up to now; the push starts the
    // sweep; from here on, position in the recording *is* elapsed time since it started.
    capture.drain(&mut buffers).map_err(|e| e.to_string())?;
    buffers.clear();
    output
        .put_data_i16(reference)
        .map_err(|e| format!("could not play the sweep: {e}"))?;

    // Long enough for the sweep itself plus the furthest delay worth looking for, and a little
    // over so the tail is not clipped off the end of the search window.
    let listen_for = latency::SWEEP_SECS + latency::MAX_DELAY_SECS + 0.2;
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f32(listen_for);
    let mut recorded: Vec<i16> = Vec::with_capacity((listen_for * RATE as f32) as usize);
    while std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
        capture.drain(&mut buffers).map_err(|e| e.to_string())?;
        recorded.extend_from_slice(buffers.player(1));
        buffers.clear();
    }

    latency::find(reference, &recorded, RATE)
        .ok_or_else(|| "nothing was recorded at all".to_owned())
}
