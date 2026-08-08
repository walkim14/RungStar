//! Song videos, decoded on a background thread and delivered on the song's clock.
//!
//! The decoder runs ahead of playback into a small queue and the game takes whichever frame
//! belongs to the moment being drawn. That ordering matters: decoding on the render thread
//! would tie the frame rate to the video's, and a song with a 4K background would stutter the
//! lyrics — which are the part you cannot afford to drop.
//!
//! **The video follows the audio, never the other way round.** Its timeline is
//! `VIDEOGAP + audio_position`, read fresh each frame, so a video that cannot keep up loses
//! frames rather than dragging the song out of time with the singing.
//!
//! Why FFmpeg and not something lighter: a real 8,000 song library measured 88.8% AV1 and
//! 11.2% H.264. Rust has a practical decoder for H.264 alone, which would have played one
//! video in nine.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// A decoded frame, ready to upload.
pub struct Frame {
    /// Tightly packed RGBA, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// When this frame should be shown, in seconds from the start of the video.
    pub at: f64,
}

/// Why a video could not be played.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("ffmpeg: {0}")]
    Ffmpeg(String),
    #[error("{0} has no video stream")]
    NoVideoStream(PathBuf),
}

impl From<ffmpeg_next::Error> for VideoError {
    fn from(error: ffmpeg_next::Error) -> Self {
        Self::Ffmpeg(error.to_string())
    }
}

/// How many decoded frames to keep ahead of playback.
///
/// Enough to ride out a slow frame or a keyframe-heavy passage, small enough that the memory
/// is bounded: at 854x480 a frame is 1.6 MB, so this is about twelve megabytes.
const QUEUE_DEPTH: usize = 8;

/// The widest a decoded frame is scaled to.
///
/// Song videos are a background behind lyrics, and the screen is at most a couple of thousand
/// pixels across. Decoding a 4K source at full size costs several times as much for a picture
/// nobody can distinguish once it is behind a scrim.
const MAX_WIDTH: u32 = 1280;

/// A video being played alongside a song.
pub struct Video {
    frames: mpsc::Receiver<Frame>,
    /// The frame that should be on screen now.
    current: Option<Frame>,
    /// The next frame, held back until its moment arrives.
    pending: Option<Frame>,
    stop: Arc<AtomicBool>,
    /// Set by the decoder when it fails, so the game can say why rather than showing nothing.
    failure: Arc<Mutex<Option<String>>>,
    /// Seconds the video should lead or lag the audio, from `#VIDEOGAP`.
    gap: f64,
    width: Arc<AtomicU32>,
    height: Arc<AtomicU32>,
    finished: bool,
}

impl Video {
    /// Start decoding a file.
    ///
    /// `gap` is `#VIDEOGAP` in seconds — positive means the video starts later than the audio.
    /// Returns immediately; decoding happens on its own thread.
    pub fn open(path: impl AsRef<Path>, gap: f64) -> Result<Self, VideoError> {
        let path = path.as_ref().to_path_buf();
        // Cheap and idempotent, but it must have happened before any other call.
        ffmpeg_next::init().map_err(|e| VideoError::Ffmpeg(e.to_string()))?;

        // Probed on this thread so a file with no video stream is an error the caller sees,
        // rather than a thread that quietly does nothing.
        probe(&path)?;

        let (sender, frames) = mpsc::sync_channel(QUEUE_DEPTH);
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let width = Arc::new(AtomicU32::new(0));
        let height = Arc::new(AtomicU32::new(0));

        let thread_stop = Arc::clone(&stop);
        let thread_failure = Arc::clone(&failure);
        let thread_width = Arc::clone(&width);
        let thread_height = Arc::clone(&height);
        std::thread::Builder::new()
            .name("video".to_owned())
            .spawn(move || {
                if let Err(error) =
                    decode_loop(&path, &sender, &thread_stop, &thread_width, &thread_height)
                {
                    *thread_failure.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(error.to_string());
                }
            })
            .map_err(|e| VideoError::Ffmpeg(e.to_string()))?;

        Ok(Self {
            frames,
            current: None,
            pending: None,
            stop,
            failure,
            gap,
            width,
            height,
            finished: false,
        })
    }

    /// The frame that belongs at this point in the song, or `None` before the first one.
    ///
    /// `audio_position` is where the *audio* is, not a wall clock. Frames that are already
    /// late are dropped rather than shown: a video behind the music is worse than a video
    /// missing a frame, and catching up by playing every frame would never catch up at all.
    pub fn frame_at(&mut self, audio_position: f64) -> Option<&Frame> {
        let wanted = audio_position - self.gap;
        loop {
            // Promote the held frame once its moment has come, then look for another. Several
            // may fall due at once if a frame took a long time to draw, and only the last of
            // them is worth showing — playing every late frame in turn would never catch up.
            if let Some(next) = &self.pending {
                if next.at > wanted {
                    break;
                }
                self.current = self.pending.take();
            }
            match self.frames.try_recv() {
                Ok(frame) => self.pending = Some(frame),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
        self.current.as_ref()
    }

    /// The decoded size, once the first frame has arrived.
    pub fn size(&self) -> Option<(u32, u32)> {
        let (w, h) = (
            self.width.load(Ordering::Relaxed),
            self.height.load(Ordering::Relaxed),
        );
        (w > 0 && h > 0).then_some((w, h))
    }

    /// The video's aspect ratio, for letterboxing.
    pub fn aspect(&self) -> Option<f32> {
        self.size().map(|(w, h)| w as f32 / h as f32)
    }

    /// Whether the decoder has run out of frames.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Why decoding stopped, if it failed.
    pub fn error(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        // The decoder blocks on a full queue, so it has to be told to stop *and* have room
        // made — otherwise the thread outlives the song it belongs to.
        self.stop.store(true, Ordering::Relaxed);
        self.pending = None;
        while self.frames.try_recv().is_ok() {}
    }
}

/// Check the file has a video stream before spawning a thread for it.
fn probe(path: &Path) -> Result<(), VideoError> {
    let input = ffmpeg_next::format::input(path)?;
    input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .map(|_| ())
        .ok_or_else(|| VideoError::NoVideoStream(path.to_path_buf()))
}

/// Decode the whole file into the queue, stopping when asked.
fn decode_loop(
    path: &Path,
    sender: &mpsc::SyncSender<Frame>,
    stop: &AtomicBool,
    out_width: &AtomicU32,
    out_height: &AtomicU32,
) -> Result<(), VideoError> {
    let mut input = ffmpeg_next::format::input(path)?;
    let stream = input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .ok_or_else(|| VideoError::NoVideoStream(path.to_path_buf()))?;
    let index = stream.index();
    // Presentation timestamps are in the stream's own units; this turns them into seconds.
    let time_base = f64::from(stream.time_base().numerator())
        / f64::from(stream.time_base().denominator().max(1));

    let context = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?;
    let mut decoder = context.decoder().video()?;

    let (source_w, source_h) = (decoder.width().max(1), decoder.height().max(1));
    let (width, height) = scaled_size(source_w, source_h);
    out_width.store(width, Ordering::Relaxed);
    out_height.store(height, Ordering::Relaxed);

    let mut scaler = ffmpeg_next::software::scaling::Context::get(
        decoder.format(),
        source_w,
        source_h,
        ffmpeg_next::format::Pixel::RGBA,
        width,
        height,
        ffmpeg_next::software::scaling::Flags::BILINEAR,
    )?;

    let mut decoded = ffmpeg_next::frame::Video::empty();
    let mut rgba = ffmpeg_next::frame::Video::empty();

    for (packet_stream, packet) in input.packets() {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if packet_stream.index() != index {
            continue;
        }
        if decoder.send_packet(&packet).is_err() {
            // A damaged packet is not worth abandoning the song for. Real libraries are full
            // of files that decode with complaints.
            continue;
        }
        if !drain(
            &mut decoder,
            &mut scaler,
            &mut decoded,
            &mut rgba,
            time_base,
            sender,
            stop,
        )? {
            return Ok(());
        }
    }

    // Whatever the decoder is still holding.
    let _ = decoder.send_eof();
    drain(
        &mut decoder,
        &mut scaler,
        &mut decoded,
        &mut rgba,
        time_base,
        sender,
        stop,
    )?;
    Ok(())
}

/// Push every frame the decoder is holding. Returns `false` when the game has gone away.
#[allow(clippy::too_many_arguments)]
fn drain(
    decoder: &mut ffmpeg_next::decoder::Video,
    scaler: &mut ffmpeg_next::software::scaling::Context,
    decoded: &mut ffmpeg_next::frame::Video,
    rgba: &mut ffmpeg_next::frame::Video,
    time_base: f64,
    sender: &mpsc::SyncSender<Frame>,
    stop: &AtomicBool,
) -> Result<bool, VideoError> {
    while decoder.receive_frame(decoded).is_ok() {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        scaler.run(decoded, rgba)?;

        // `data(0)` is padded to the scaler's stride, which is rarely the row width. Copying
        // row by row is what makes the buffer safe to hand to a texture upload.
        let width = rgba.width();
        let height = rgba.height();
        let stride = rgba.stride(0);
        let row = (width * 4) as usize;
        let source = rgba.data(0);
        let mut packed = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let from = y * stride;
            packed.extend_from_slice(&source[from..from + row]);
        }

        let at = decoded.timestamp().unwrap_or(0) as f64 * time_base;
        let frame = Frame {
            rgba: packed,
            width,
            height,
            at,
        };
        // Blocks when the queue is full, which is the point: it is what keeps the decoder
        // from running away with memory on a machine that decodes faster than it draws.
        if sender.send(frame).is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The size to decode at, never wider than [`MAX_WIDTH`] and never upscaled.
fn scaled_size(width: u32, height: u32) -> (u32, u32) {
    if width <= MAX_WIDTH {
        return (width, height);
    }
    let scale = MAX_WIDTH as f64 / width as f64;
    // Even dimensions, because odd ones upset some scalers and cost nothing to avoid.
    let scaled = ((height as f64 * scale).round() as u32).max(2);
    (MAX_WIDTH, scaled & !1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_is_never_upscaled_and_keeps_its_shape() {
        // Decoding a 4K source at full size costs several times as much for a picture nobody
        // can tell apart once it is behind a scrim.
        assert_eq!(
            scaled_size(854, 480),
            (854, 480),
            "small sources are left alone"
        );
        assert_eq!(scaled_size(1280, 720), (1280, 720));

        let (w, h) = scaled_size(3840, 2160);
        assert_eq!(w, MAX_WIDTH);
        let source = 3840.0 / 2160.0;
        let scaled = w as f64 / h as f64;
        assert!(
            (source - scaled).abs() < 0.01,
            "aspect changed from {source} to {scaled}"
        );
        assert_eq!(h % 2, 0, "an odd height upsets some scalers");
    }

    #[test]
    fn a_portrait_video_scales_on_its_width_too() {
        let (w, h) = scaled_size(2160, 3840);
        assert_eq!(w, MAX_WIDTH);
        assert!(h > w, "a portrait video should stay portrait");
        assert_eq!(h % 2, 0);
    }
}
