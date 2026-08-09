//! Playing every song at the same loudness.
//!
//! A library assembled from a thousand uploads is not level with itself: fifteen decibels
//! between the loudest and the quietest is ordinary, so somebody reaches for the volume between
//! every song, and browsing previews is worse still because the difference lands every time the
//! cursor moves.
//!
//! [`rungstar_audio::loudness`] is the measurement. This is when it happens, which is the part
//! that had to be decided.
//!
//! **Not during a scan.** Measuring needs the whole song decoded, which is about a fifth of a
//! second each — a few minutes across eight thousand songs, added to a rescan that currently
//! takes half of one. A library that takes minutes to open is a worse problem than one that is
//! uneven.
//!
//! **So: the first time the audio is decoded anyway**, which is the first time the song is
//! played or previewed. Decoding runs a thousand times faster than playback, so the whole file
//! is usually in memory within a second of the song starting — the measurement lands during
//! the same preview that triggered it, and the correction is applied then. Every play after
//! that has it from the start. The work happens on its own thread, because filtering a
//! three-minute stereo song is tens of millions of operations and a frame is sixteen
//! milliseconds.
//!
//! **And it is remembered in the library index**, in the song's own row, written the way a play
//! count is written: outside a scan and never by one.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};

use rungstar_audio::loudness::{self, Measured};
use rungstar_audio::AudioClip;

/// How much of a song is measured, in seconds.
///
/// The gate already discards quiet passages, so measuring more than a few minutes buys
/// accuracy nobody can hear, and an hour-long file — which a live set or a mistagged album rip
/// is — would otherwise cost a hundred times what a song does.
const MEASURE_SECS: f64 = 240.0;

/// What is known about how loud each song is.
pub struct Loudness {
    /// Whether to correct at all.
    pub enabled: bool,
    known: HashMap<i64, Measured>,
    /// Already handed to a worker, so a song sitting under the cursor is not measured on
    /// every frame.
    started: HashSet<i64>,
    results: Receiver<(i64, Option<Measured>)>,
    finished: Sender<(i64, Option<Measured>)>,
}

impl Default for Loudness {
    fn default() -> Self {
        Self::new()
    }
}

impl Loudness {
    pub fn new() -> Self {
        let (finished, results) = std::sync::mpsc::channel();
        Self {
            enabled: true,
            known: HashMap::new(),
            started: HashSet::new(),
            results,
            finished,
        }
    }

    /// Take a measurement the index already had.
    ///
    /// Both halves or neither: a loudness with no peak cannot be turned into a safe gain, so a
    /// row written by an older version is treated as unmeasured and measured again.
    pub fn remember(&mut self, id: i64, lufs: Option<f32>, peak: Option<f32>) {
        if let (Some(lufs), Some(peak)) = (lufs, peak) {
            self.known.insert(id, Measured { lufs, peak });
            // Never measured again: a song's loudness is a property of its file, and the file
            // is the same one it was.
            self.started.insert(id);
        }
    }

    /// The multiplier to play this song at.
    ///
    /// `1.0` when correction is off, or when nothing is known yet — which is what makes the
    /// first play of a new song simply normal rather than wrong in some other way.
    pub fn gain(&self, id: Option<i64>) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        id.and_then(|id| self.known.get(&id))
            .copied()
            .map_or(1.0, loudness::playback_gain)
    }

    /// Measure a song, once its audio has finished decoding.
    ///
    /// Cheap to call every frame: it returns immediately unless the clip is complete and this
    /// song has never been measured.
    pub fn measure(&mut self, id: i64, clip: &AudioClip) {
        if !self.enabled || self.started.contains(&id) || !clip.is_complete() {
            return;
        }
        self.started.insert(id);

        let clip = clip.clone();
        let finished = self.finished.clone();
        // Detached, and its result is picked up whenever it arrives. If the game closes first
        // the song is measured again next time, which costs a second of one core and no
        // correctness.
        let spawned = std::thread::Builder::new()
            .name(format!("loudness {id}"))
            .spawn(move || {
                let _ = finished.send((id, measure_clip(&clip)));
            });
        if spawned.is_err() {
            // Out of threads. Allow another attempt later rather than marking it done.
            self.started.remove(&id);
        }
    }

    /// Collect anything the workers finished, and say which songs they were.
    ///
    /// The caller writes them to the index; this crate holds no database of its own.
    pub fn collect(&mut self) -> Vec<(i64, Measured)> {
        let mut written = Vec::new();
        while let Ok((id, measured)) = self.results.try_recv() {
            match measured {
                Some(measured) => {
                    self.known.insert(id, measured);
                    written.push((id, measured));
                }
                // Unmeasurable — silence, or a file too short to gate. Left unknown so it
                // plays as it is, and `started` keeps it from being retried all session.
                None => tracing::debug!("song {id} could not be measured"),
            }
        }
        written
    }
}

/// Read a decoded clip out and measure it.
fn measure_clip(clip: &AudioClip) -> Option<Measured> {
    let channels = clip.channels().max(1);
    let rate = clip.sample_rate().max(1);
    let frames = clip
        .ready_frames()
        .min((MEASURE_SECS * f64::from(rate)) as usize);
    if frames == 0 {
        return None;
    }
    let mut samples = vec![0i16; frames * channels];
    let read = clip.read(0, &mut samples);
    samples.truncate(read * channels);
    loudness::analyse(&samples, channels, rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_corrected_until_something_is_known() {
        let mut held = Loudness::new();
        assert_eq!(held.gain(Some(1)), 1.0);
        assert_eq!(held.gain(None), 1.0);

        held.remember(1, Some(-28.0), Some(0.3));
        assert!(held.gain(Some(1)) > 1.0, "a quiet song should be turned up");
        assert_eq!(held.gain(Some(2)), 1.0, "and its neighbour left alone");
    }

    #[test]
    fn turning_it_off_leaves_every_song_alone() {
        let mut held = Loudness::new();
        held.remember(1, Some(-28.0), Some(0.3));
        held.enabled = false;
        assert_eq!(held.gain(Some(1)), 1.0);
    }

    #[test]
    fn a_song_the_index_already_knew_is_never_measured_again() {
        // The measurement is a property of the file, and the file has not changed. Re-running
        // it on every launch would be a second of one core per song for the same answer.
        let mut held = Loudness::new();
        held.remember(7, Some(-14.0), Some(0.9));
        assert!(held.started.contains(&7));
        assert_eq!(held.known.get(&7).map(|m| m.lufs), Some(-14.0));
    }

    #[test]
    fn an_unknown_song_is_not_marked_as_known() {
        let mut held = Loudness::new();
        held.remember(7, None, None);
        assert!(!held.started.contains(&7));
        assert!(!held.known.contains_key(&7));
    }

    #[test]
    fn a_loudness_with_no_peak_is_not_usable() {
        // A row written before peaks were kept. Half a measurement cannot produce a safe
        // gain, so it counts as no measurement and the song is measured again.
        let mut held = Loudness::new();
        held.remember(7, Some(-24.0), None);
        assert_eq!(held.gain(Some(7)), 1.0);
        assert!(!held.started.contains(&7));
    }
}
