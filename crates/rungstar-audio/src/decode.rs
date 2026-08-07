//! Turning a song's audio file into samples.
//!
//! Decoding happens on a background thread and playback starts as soon as the first second
//! is ready, rather than waiting for the whole file. A four-minute MP3 takes a noticeable
//! moment to decode in full, and a pause between choosing a song and hearing it is exactly
//! the friction this project exists to remove.
//!
//! Samples are kept in memory for the whole song. That costs around forty megabytes for a
//! long track, which buys instant, exact seeking — worth it when the restart button and the
//! editor both seek constantly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("could not open '{path}': {source}")]
    Open {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("unsupported or corrupt audio: {0}")]
    Unsupported(String),
    #[error("no audio track in the file")]
    NoTrack,
}

/// Decoded audio that may still be filling.
///
/// Cheap to clone: every handle shares one buffer.
#[derive(Clone)]
pub struct AudioClip {
    inner: Arc<Shared>,
}

struct Shared {
    /// Interleaved samples. Only the first `ready_frames * channels` are valid.
    samples: Mutex<Vec<i16>>,
    ready_frames: AtomicUsize,
    finished: AtomicBool,
    failed: Mutex<Option<String>>,
    channels: usize,
    sample_rate: u32,
}

impl AudioClip {
    pub fn channels(&self) -> usize {
        self.inner.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate
    }

    /// Frames decoded so far.
    pub fn ready_frames(&self) -> usize {
        self.inner.ready_frames.load(Ordering::Acquire)
    }

    /// Whether decoding has run to completion.
    pub fn is_complete(&self) -> bool {
        self.inner.finished.load(Ordering::Acquire)
    }

    /// The decode error, if the background thread gave up.
    pub fn error(&self) -> Option<String> {
        self.inner
            .failed
            .lock()
            .ok()
            .and_then(|error| error.clone())
    }

    /// Seconds of audio decoded so far.
    pub fn ready_secs(&self) -> f64 {
        self.ready_frames() as f64 / f64::from(self.inner.sample_rate)
    }

    /// Copy interleaved samples starting at `frame` into `out`, returning frames written.
    ///
    /// Stops at whatever has been decoded, so a caller that outruns the decoder gets a short
    /// read rather than silence spliced into the middle of the song.
    pub fn read(&self, frame: usize, out: &mut [i16]) -> usize {
        let channels = self.inner.channels;
        let ready = self.ready_frames();
        if frame >= ready || channels == 0 {
            return 0;
        }
        let frames = (out.len() / channels).min(ready - frame);
        let Ok(samples) = self.inner.samples.lock() else {
            return 0;
        };
        let start = frame * channels;
        out[..frames * channels].copy_from_slice(&samples[start..start + frames * channels]);
        frames
    }
}

impl AudioClip {
    /// Start decoding `path` in the background.
    ///
    /// Returns as soon as the format has been identified, so the caller knows the sample rate
    /// and channel count immediately and can open an output device with them.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DecodeError> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path).map_err(|source| DecodeError::Open {
            path: path.clone(),
            source,
        })?;

        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| DecodeError::Unsupported(e.to_string()))?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or(DecodeError::NoTrack)?;
        let track_id = track.id;

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| DecodeError::Unsupported(e.to_string()))?;

        let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
        let channels = track.codec_params.channels.map_or(2, |c| c.count()).max(1);
        // Pre-size from the declared length when there is one, so the buffer does not
        // reallocate repeatedly while the song is playing out of it.
        let expected = track
            .codec_params
            .n_frames
            .map_or(0, |f| f as usize * channels);

        let shared = Arc::new(Shared {
            samples: Mutex::new(Vec::with_capacity(expected)),
            ready_frames: AtomicUsize::new(0),
            finished: AtomicBool::new(false),
            failed: Mutex::new(None),
            channels,
            sample_rate,
        });

        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("rungstar-decode".to_owned())
            .spawn(move || {
                let mut buffer: Option<SampleBuffer<i16>> = None;
                // Any demuxer error ends the loop: at this point it means the stream is
                // over, which is how a file normally finishes.
                while let Ok(packet) = format.next_packet() {
                    if packet.track_id() != track_id {
                        continue;
                    }
                    match decoder.decode(&packet) {
                        Ok(decoded) => {
                            let target = buffer.get_or_insert_with(|| {
                                SampleBuffer::new(decoded.capacity() as u64, *decoded.spec())
                            });
                            target.copy_interleaved_ref(decoded);
                            if let Ok(mut store) = worker.samples.lock() {
                                store.extend_from_slice(target.samples());
                                let frames = store.len() / worker.channels;
                                worker.ready_frames.store(frames, Ordering::Release);
                            }
                        }
                        // A corrupt packet mid-song should skip, not stop: the rest of the
                        // track is still worth hearing.
                        Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                        Err(error) => {
                            if let Ok(mut failed) = worker.failed.lock() {
                                *failed = Some(error.to_string());
                            }
                            break;
                        }
                    }
                }
                worker.finished.store(true, Ordering::Release);
            })
            .map_err(|e| DecodeError::Unsupported(e.to_string()))?;

        Ok(Self { inner: shared })
    }

    /// Block until at least `secs` of audio is ready, or decoding ends.
    ///
    /// Used once at song start so playback begins with a cushion; after that the decoder
    /// stays comfortably ahead and nothing waits.
    pub fn wait_for(&self, secs: f64, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.ready_secs() >= secs || self.is_complete() {
                return self.ready_secs() >= secs;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

impl std::fmt::Debug for AudioClip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioClip")
            .field("channels", &self.channels())
            .field("sample_rate", &self.sample_rate())
            .field("ready_frames", &self.ready_frames())
            .field("complete", &self.is_complete())
            .finish()
    }
}
