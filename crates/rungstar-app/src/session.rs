//! Playing a song: the audio clock, the microphones, and the scorer.
//!
//! Everything here touches a device, which is why it is not in `rungstar-ui`. The screen is
//! handed the result and draws it.
//!
//! Multiple singers are the normal case rather than a special one: the capture layer has
//! always routed channels to players, and this holds one analyser and one scorer per singer,
//! so one and six differ only in how many were configured.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use rungstar_audio::{AudioClip, CaptureBackend, DeviceConfig, MasterClock, PlayerBuffers, Timing};
use rungstar_pitch::{Analyzer, AnalyzerConfig};
use rungstar_platform::{Playback, SdlCapture};
use rungstar_score::{Difficulty, ScoreTrack, Scorer};
use rungstar_song::{Line, SongTxt};
use rungstar_ui::singscreen::{Note, NoteKind, NoteLine, Singer, Sung, Syllable};

const SAMPLE_RATE: u32 = 44_100;

/// How often the microphones are analysed. UltraStar's own rate, and well inside what the
/// detectors cost even for six singers.
const ANALYSIS_INTERVAL: Duration = Duration::from_millis(10);

/// How far back beyond the current line what was sung is kept, in beats.
///
/// Trimming to a fixed window behind the playhead made the marks at the start of a long line
/// disappear while the line was still on screen — the one place they need to stay, since the
/// whole point of drawing a line at a time is being able to look back over it.
const SUNG_KEEP_BEFORE_LINE: f64 = 8.0;

/// Whether the song is over.
///
/// It ends when the audio does, or at `#END` for a song that names one. Deliberately not when
/// the notes run out: an earlier version stopped eight beats after the last note, and the note
/// grid runs at four times the written BPM — so at a typical tempo that was under half a
/// second, and every song cut off dead on its final syllable. An outro is part of the song,
/// and `#END` is how a song says to skip it.
fn song_over(audio_finished: bool, position: f64, end_secs: Option<f64>) -> bool {
    audio_finished || end_secs.is_some_and(|end| position >= end)
}

/// One song being played.
pub struct Session {
    clock: MasterClock,
    playback: Playback,
    buffers: PlayerBuffers,
    analysers: Vec<Analyzer>,
    scorers: Vec<Scorer>,
    lines: Vec<Line>,
    /// Next line whose bonus has not been awarded, per singer.
    next_line: Vec<usize>,
    scored_through: f64,
    last_frame: Instant,
    last_analysis: Instant,
    sung: Vec<Vec<Sung>>,
    ratings: Vec<Option<(i32, Instant)>>,
    ever_heard: Vec<bool>,
    levels: Vec<f32>,
    pitches: Vec<Option<i32>>,
    hitting: Vec<Option<bool>>,
    gate: f32,
    notes: Vec<Note>,
    capture: SdlCapture,
    has_microphone: bool,
    finished: bool,
    /// Where `#END` says to stop, in seconds. `None` means play to the end of the audio.
    end_secs: Option<f64>,
}

impl Session {
    /// Start a song.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        audio_subsystem: &sdl3::AudioSubsystem,
        song: &SongTxt,
        audio_path: &Path,
        players: usize,
        difficulty: Difficulty,
        threshold: f32,
        mic_delay_ms: f64,
        saved_microphones: &[rungstar_ui::settings::MicAssignment],
        mut capture: SdlCapture,
    ) -> Result<Self> {
        let clip = AudioClip::open(audio_path).context("could not decode the audio")?;
        // A cushion, so playback does not stall on the first frames.
        clip.wait_for(0.5, Duration::from_secs(20));
        if let Some(error) = clip.error() {
            bail!("audio decoding failed: {error}");
        }
        let mut playback = Playback::new(audio_subsystem, clip)
            .map_err(|e| anyhow::anyhow!("could not open the output device: {e}"))?;

        let players = players.clamp(1, rungstar_audio::capture::MAX_PLAYERS);
        let mut devices = choose_devices(&capture, players);
        // Whatever the setup screen was told wins over the automatic assignment.
        apply_saved(&mut devices, saved_microphones);
        let has_microphone = devices
            .iter()
            .any(|d| d.channel_to_player.iter().any(|p| *p != 0));
        if has_microphone {
            if let Err(error) = capture.start(&devices, SAMPLE_RATE) {
                tracing::warn!("capture could not start: {error}");
            }
        }

        let lines = song.tracks.track_1.clone();
        let track = ScoreTrack::from_lines(&lines);
        let mut timing = Timing::new(song.bpm().value(), song.headers.gap as f64);
        timing.mic_delay = mic_delay_ms / 1000.0;

        let notes = collect_notes(&lines);
        // `#END` is in milliseconds, unlike `#START` and `#PREVIEWSTART` which are seconds.
        // The format is inconsistent about this and it is an easy place to be wrong.
        let end_secs = song
            .headers
            .end
            .filter(|ms| *ms > 0)
            .map(|ms| ms as f64 / 1000.0);

        let mut clock = MasterClock::new(timing);
        playback
            .start()
            .map_err(|e| anyhow::anyhow!("could not start playback: {e}"))?;
        clock.start();

        let config = AnalyzerConfig {
            sample_rate: SAMPLE_RATE,
            ..AnalyzerConfig::default()
        };
        let gate = config.threshold;

        Ok(Self {
            clock,
            playback,
            buffers: PlayerBuffers::new(),
            analysers: (0..players).map(|_| Analyzer::new(config)).collect(),
            scorers: (0..players)
                .map(|_| Scorer::new(track.clone(), difficulty))
                .collect(),
            lines,
            next_line: vec![0; players],
            scored_through: f64::NEG_INFINITY,
            last_frame: Instant::now(),
            last_analysis: Instant::now(),
            sung: vec![Vec::new(); players],
            ratings: vec![None; players],
            ever_heard: vec![false; players],
            levels: vec![0.0; players],
            pitches: vec![None; players],
            hitting: vec![None; players],
            gate: if threshold > 0.0 { threshold } else { gate },
            notes,
            capture,
            has_microphone,
            finished: false,
            end_secs,
        })
    }

    /// Change how loud the song plays, while it is playing.
    pub fn set_volume(&self, volume: f32) {
        self.playback.set_volume(volume);
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn position(&self) -> f32 {
        self.playback.position() as f32
    }

    pub fn duration(&self) -> f32 {
        self.playback.duration() as f32
    }

    pub fn pause(&mut self) {
        if self.playback.is_playing() {
            let _ = self.playback.pause();
            self.clock.pause();
        }
    }

    pub fn resume(&mut self) {
        if !self.playback.is_playing() {
            let _ = self.playback.start();
            self.clock.start();
        }
    }

    /// Stop playing and release the devices.
    pub fn stop(&mut self) {
        self.capture.stop();
        let _ = self.playback.pause();
    }

    /// Advance the clock, read the microphones and score whatever beats have passed.
    pub fn tick(&mut self) -> Result<()> {
        self.playback
            .pump()
            .map_err(|e| anyhow::anyhow!("playback failed: {e}"))?;

        let elapsed = self.last_frame.elapsed();
        self.last_frame = Instant::now();
        self.clock.tick(elapsed);
        self.clock.synchronize(self.playback.position());

        self.buffers.clear();
        let _ = self.capture.drain(&mut self.buffers);
        for (player, analyser) in self.analysers.iter_mut().enumerate() {
            // Players are one-based in the routing, because zero means "channel off".
            let samples = self.buffers.player(player as u8 + 1);
            if !samples.is_empty() {
                analyser.push(samples);
                self.ever_heard[player] = true;
            }
        }

        // Analysis runs on its own interval rather than per frame, so scoring does not depend
        // on how fast the machine draws.
        if self.last_analysis.elapsed() >= ANALYSIS_INTERVAL {
            self.last_analysis = Instant::now();
            for (player, analyser) in self.analysers.iter_mut().enumerate() {
                self.pitches[player] = analyser.detect().map(|d| d.pitch_class);
                // Decays rather than tracking instantaneously, so the meter is legible
                // instead of flickering.
                self.levels[player] = (self.levels[player] * 0.8).max(analyser.volume());
            }
        }

        let beats = self.clock.beats();
        if self.clock.is_paused() {
            // A paused clock must not keep scoring silence.
            self.scored_through = beats.detection;
        } else {
            self.score_elapsed(beats.detection);
        }

        if song_over(
            self.playback.is_finished(),
            self.playback.position(),
            self.end_secs,
        ) {
            self.finished = true;
        }
        Ok(())
    }

    /// Score every whole detection beat that has passed, for every singer.
    fn score_elapsed(&mut self, detection_beat: f64) {
        if self.scored_through.is_infinite() {
            self.scored_through = detection_beat - 1.0;
        }
        for beat in MasterClock::beats_crossed(self.scored_through, detection_beat) {
            for player in 0..self.scorers.len() {
                let sung = self.pitches[player].filter(|_| self.levels[player] >= self.gate);
                let result = self.scorers[player].sing_beat(beat, sung);
                if result.target.is_some() {
                    self.hitting[player] = Some(result.hit);
                    if let Some(pitch) = sung {
                        self.record_sung(player, beat, pitch, result.hit);
                    }
                }

                // Close any line the beat has now passed the end of.
                while self.next_line[player] < self.lines.len() {
                    let line = &self.lines[self.next_line[player]];
                    if line.notes.is_empty() || beat < line.end() {
                        break;
                    }
                    if let Some(result) = self.scorers[player].end_line(self.next_line[player]) {
                        self.ratings[player] = Some((result.rating, Instant::now()));
                    }
                    self.next_line[player] += 1;
                }
            }
        }
        self.scored_through = detection_beat;

        // Forget what is behind the line being sung, so a long song does not grow without
        // bound, but never anything still on screen.
        let cutoff = self
            .line_at(detection_beat)
            .and_then(|index| self.lines.get(index))
            .map(|line| line.start() as f64 - SUNG_KEEP_BEFORE_LINE)
            .unwrap_or(detection_beat - 64.0);
        for history in &mut self.sung {
            history.retain(|s| s.start + s.duration >= cutoff);
        }
    }

    /// Extend the last run rather than pushing a bar per beat, so a held note draws as one
    /// stretch instead of a dotted line.
    fn record_sung(&mut self, player: usize, beat: i32, pitch: i32, hit: bool) {
        let history = &mut self.sung[player];
        let beat = beat as f64;
        match history.last_mut() {
            Some(last)
                if last.hit == hit
                    && last.pitch == pitch
                    && (last.start + last.duration - beat).abs() < 1.5 =>
            {
                last.duration = (beat + 1.0) - last.start;
            }
            _ => history.push(Sung {
                start: beat,
                duration: 1.0,
                pitch,
                hit,
            }),
        }
    }

    /// The drawing beat, which runs ahead of the scoring beat by the microphone delay.
    pub fn visual_beat(&self) -> f64 {
        self.clock.beats().visual
    }

    /// Fill in the screen's singers from the current state.
    pub fn update_singers(&self, singers: &mut [Singer]) {
        for (player, singer) in singers.iter_mut().enumerate() {
            if player >= self.scorers.len() {
                break;
            }
            let totals = self.scorers[player].totals();
            singer.score = totals.total as i32;
            singer.fraction = totals.total as f32 / 10_000.0;
            singer.level = self.levels[player];
            singer.gate = self.gate;
            singer.pitch = self.pitches[player];
            singer.hitting = self.hitting[player];
            singer.ever_heard = self.ever_heard[player];
            singer.has_microphone = self.has_microphone;
            singer.rating =
                self.ratings[player].map(|(rating, at)| (rating, at.elapsed().as_secs_f32()));
            singer.sung.clone_from(&self.sung[player]);
        }
    }

    /// Which line is being sung at `beat`.
    ///
    /// Lyrics have to appear slightly before they are sung or nobody can read them in time,
    /// so this returns the upcoming line once the previous one has ended. Notes and lyrics
    /// both go through it, because a staff showing one line while the words show another is
    /// worse than either being slightly early.
    fn line_at(&self, beat: f64) -> Option<usize> {
        let whole = beat as i32;
        self.lines
            .iter()
            .position(|line| !line.notes.is_empty() && whole < line.end())
            .or_else(|| self.lines.iter().rposition(|line| !line.notes.is_empty()))
    }

    /// The notes of the line being sung, and the beats it spans.
    pub fn current_line(&self, beat: f64) -> NoteLine {
        let Some(index) = self.line_at(beat) else {
            return NoteLine::default();
        };
        let line = &self.lines[index];
        let notes: Vec<Note> = line.notes.iter().map(convert_note).collect();
        let start = notes.iter().map(|n| n.start).fold(f64::MAX, f64::min);
        let end = notes.iter().map(Note::end).fold(f64::MIN, f64::max);
        NoteLine { notes, start, end }
    }

    /// The lowest and highest pitch in the whole song, for a scale that does not move.
    pub fn pitch_range(&self) -> (i32, i32) {
        let (low, high) = self.notes.iter().fold((i32::MAX, i32::MIN), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        if low > high {
            (0, 12)
        } else {
            // A song sitting on two notes would otherwise get a two-row staff, where every
            // wobble looks like a wrong note.
            let span = (high - low).max(7);
            (low, low + span)
        }
    }

    /// The syllables of the line being sung, and the text of the one after it.
    pub fn lyrics(&self, beat: f64) -> (Vec<Syllable>, String) {
        let Some(index) = self.line_at(beat) else {
            return (Vec::new(), String::new());
        };
        let syllables = self.lines[index]
            .notes
            .iter()
            .map(|note| Syllable {
                text: note.text.clone(),
                start: note.start as f64,
                duration: note.duration as f64,
                golden: note.kind.is_golden(),
            })
            .collect();
        let next = self
            .lines
            .get(index + 1)
            .map(|line| line.text())
            .unwrap_or_default();
        (syllables, next)
    }
}

/// Turn a parsed note into one the screen can draw.
fn convert_note(note: &rungstar_song::Note) -> Note {
    Note {
        start: note.start as f64,
        duration: note.duration as f64,
        pitch: note.pitch,
        kind: match note.kind {
            rungstar_song::NoteKind::Golden => NoteKind::Golden,
            rungstar_song::NoteKind::Freestyle => NoteKind::Freestyle,
            rungstar_song::NoteKind::Rap => NoteKind::Rap,
            rungstar_song::NoteKind::GoldenRap => NoteKind::GoldenRap,
            rungstar_song::NoteKind::Regular => NoteKind::Normal,
        },
    }
}

/// Flatten the track's notes, for the song's overall range and its end.
fn collect_notes(lines: &[Line]) -> Vec<Note> {
    lines
        .iter()
        .flat_map(|line| line.notes.iter())
        .map(convert_note)
        .collect()
}

/// Names that belong to virtual or loopback devices rather than a microphone.
///
/// These sit at the top of the device list on a machine with Steam installed and deliver
/// silence forever, which is indistinguishable from a broken setup unless they are skipped.
const VIRTUAL_DEVICES: [&str; 5] = [
    "steam streaming",
    "virtual",
    "voicemeeter",
    "vb-audio",
    "cable output",
];

fn looks_virtual(name: &str) -> bool {
    let lower = name.to_lowercase();
    VIRTUAL_DEVICES.iter().any(|bad| lower.contains(bad))
}

/// Apply a saved assignment to the devices that are actually present.
///
/// Matched by name, and anything unrecognised keeps whatever was worked out for it, so
/// plugging in a new microphone does not discard the setup for the old ones.
pub fn apply_saved(devices: &mut [DeviceConfig], saved: &[rungstar_ui::settings::MicAssignment]) {
    for device in devices.iter_mut() {
        if let Some(entry) = saved.iter().find(|s| s.name == device.name) {
            // Channel counts can change when a device is re-enumerated; keep whatever fits.
            for (channel, player) in entry.channels.iter().enumerate() {
                if channel < device.channel_to_player.len() {
                    device.channel_to_player[channel] = *player;
                }
            }
        }
    }
}

/// Choose which channel each singer sings into.
///
/// One singer per *device* first, and only then a second channel of a device that has one.
/// Nearly every USB microphone reports two channels and is mono on both, so filling channels
/// first puts player two on a channel that either duplicates player one or is silent — which
/// is exactly the setup that cannot work.
///
/// The case that does want two channels on one device is the cheap dual-USB karaoke set,
/// where the two microphones genuinely are left and right. That is reached on the second
/// pass, once every device already has a singer.
pub fn choose_devices(capture: &SdlCapture, players: usize) -> Vec<DeviceConfig> {
    let Ok(devices) = capture.devices() else {
        return Vec::new();
    };
    let mut configs: Vec<DeviceConfig> = devices
        .into_iter()
        .enumerate()
        .filter(|(_, device)| !looks_virtual(&device.name))
        .map(|(index, device)| DeviceConfig {
            name: device.name.clone(),
            input_index: index as u32,
            latency_ms: rungstar_audio::capture::LATENCY_AUTODETECT,
            channel_to_player: vec![0; device.channels.max(1)],
        })
        .collect();

    let mut assigned = 0u8;
    // First pass: the first channel of each device.
    for config in configs.iter_mut() {
        if assigned as usize >= players {
            break;
        }
        assigned += 1;
        config.channel_to_player[0] = assigned;
    }
    // Second pass: the remaining channels, for a stereo pair carrying two singers.
    for config in configs.iter_mut() {
        for channel in 1..config.channel_to_player.len() {
            if assigned as usize >= players {
                break;
            }
            assigned += 1;
            config.channel_to_player[channel] = assigned;
        }
    }
    // A device with nothing on it is still worth listing, so the setup screen can show its
    // meter and let it be chosen.
    configs
}

/// Capture running purely so the microphone screen can show live meters.
///
/// Separate from [`Session`] because it has no song, no clock and no scoring — it exists to
/// answer "is anything arriving on this channel", which is the one question the setup screen
/// is for and the one UltraStar's own record screen does not answer.
pub struct Monitor {
    capture: SdlCapture,
    players: usize,
    /// What was saved last time, so opening the screen shows the setup rather than the
    /// defaults it would have worked out from scratch.
    saved: Vec<rungstar_ui::settings::MicAssignment>,
    devices: Vec<DeviceConfig>,
    buffers: PlayerBuffers,
    /// Peak level per device per channel.
    levels: Vec<Vec<f32>>,
    heard: Vec<Vec<bool>>,
    last_analysis: Instant,
}

impl Monitor {
    /// Open every microphone, with each channel routed to its own slot.
    ///
    /// Routing is by channel rather than by device, so a stereo pair shows two independent
    /// meters — which is what makes it obvious that only one of the two microphones is live.
    pub fn start(
        capture: SdlCapture,
        players: usize,
        saved: &[rungstar_ui::settings::MicAssignment],
    ) -> Self {
        let mut monitor = Self {
            capture,
            players: players.max(1),
            saved: saved.to_vec(),
            devices: Vec::new(),
            buffers: PlayerBuffers::new(),
            levels: Vec::new(),
            heard: Vec::new(),
            last_analysis: Instant::now(),
        };
        monitor.rescan_with(players);
        monitor
    }

    /// Look for devices again.
    pub fn rescan(&mut self) {
        let players = self.players;
        self.rescan_with(players);
    }

    fn rescan_with(&mut self, players: usize) {
        self.capture.stop();
        let mut found = choose_devices(&self.capture, players);
        // Whatever was saved wins over the automatic assignment, or the screen would show
        // defaults every time it opened and the setup would appear not to have been kept.
        apply_saved(&mut found, &self.saved);
        self.levels = found.iter().map(|d| vec![0.0; d.channels()]).collect();
        self.heard = found.iter().map(|d| vec![false; d.channels()]).collect();
        self.devices = found;
        self.restart();
    }

    /// Apply an assignment the screen has changed.
    pub fn reassign(&mut self, devices: &[rungstar_ui::micscreen::Device]) {
        for (config, shown) in self.devices.iter_mut().zip(devices) {
            config.channel_to_player.clone_from(&shown.assignment);
        }
        self.saved = self.saved_assignment();
        self.restart();
    }

    fn saved_assignment(&self) -> Vec<rungstar_ui::settings::MicAssignment> {
        self.devices
            .iter()
            .map(|device| rungstar_ui::settings::MicAssignment {
                name: device.name.clone(),
                channels: device.channel_to_player.clone(),
            })
            .collect()
    }

    fn restart(&mut self) {
        self.capture.stop();
        if self.devices.is_empty() {
            return;
        }
        // Every channel is monitored, whatever it is assigned to, so a channel set to "off"
        // still shows a level. Otherwise turning a channel off hides the evidence you need to
        // decide whether it should be on.
        let monitored: Vec<DeviceConfig> = self
            .devices
            .iter()
            .scan(0u8, |next, device| {
                let mapping = (0..device.channels())
                    .map(|_| {
                        *next += 1;
                        (*next).min(rungstar_audio::capture::MAX_PLAYERS as u8)
                    })
                    .collect();
                Some(DeviceConfig {
                    channel_to_player: mapping,
                    ..device.clone()
                })
            })
            .collect();
        if let Err(error) = self.capture.start(&monitored, SAMPLE_RATE) {
            tracing::warn!("microphone monitor could not start: {error}");
        }
    }

    /// Read the microphones and update the meters.
    pub fn tick(&mut self) {
        self.buffers.clear();
        let _ = self.capture.drain(&mut self.buffers);

        let mut slot = 0u8;
        for (device, levels) in self.levels.iter_mut().enumerate() {
            for (channel, level) in levels.iter_mut().enumerate() {
                slot += 1;
                let samples = self.buffers.player(slot);
                if !samples.is_empty() {
                    let peak = samples
                        .iter()
                        .map(|s| (*s as f32 / i16::MAX as f32).abs())
                        .fold(0.0f32, f32::max);
                    *level = level.max(peak);
                    if peak > 0.0005 {
                        // "Something arrived once" is a different fact from "something is
                        // arriving now", and a setup screen needs both: a microphone that has
                        // never produced a sample is unplugged, not quiet.
                        self.heard[device][channel] = true;
                    }
                }
            }
        }

        // The decay runs on its own interval so the meter falls at the same rate whatever the
        // frame rate is.
        if self.last_analysis.elapsed() >= ANALYSIS_INTERVAL {
            self.last_analysis = Instant::now();
            for levels in &mut self.levels {
                for level in levels.iter_mut() {
                    *level *= 0.85;
                }
            }
        }
    }

    /// What the screen should show.
    pub fn devices(&self) -> Vec<rungstar_ui::micscreen::Device> {
        self.devices
            .iter()
            .enumerate()
            .map(|(index, device)| rungstar_ui::micscreen::Device {
                name: device.name.clone(),
                assignment: device.channel_to_player.clone(),
                levels: self.levels.get(index).cloned().unwrap_or_default(),
                heard: self.heard.get(index).cloned().unwrap_or_default(),
            })
            .collect()
    }

    /// The assignment, for saving. A setup that does not survive a restart is not a setup.
    pub fn saved(&self) -> Vec<rungstar_ui::settings::MicAssignment> {
        self.saved_assignment()
    }

    pub fn stop(&mut self) {
        self.capture.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_plays_its_outro_rather_than_stopping_on_the_last_note() {
        // The bug this replaces: the end was eight *grid* beats after the last note, and the
        // grid runs at four times the written BPM. At 300 in the file that is 1200 beats a
        // minute, so eight of them is four tenths of a second — the song stopped dead on its
        // final syllable every time.
        assert!(!song_over(false, 200.0, None), "audio still playing");
        assert!(song_over(true, 200.0, None), "audio ran out");
    }

    #[test]
    fn a_song_that_names_an_end_stops_there() {
        // `#END` is in milliseconds, unlike `#START` and `#PREVIEWSTART` which are seconds.
        // The format is inconsistent and this is an easy place to be wrong.
        let end = Some(180_000.0 / 1000.0);
        assert!(!song_over(false, 179.9, end));
        assert!(song_over(false, 180.0, end));
        assert!(song_over(false, 200.0, end));
        // And the audio running out still ends it, even before `#END`.
        assert!(song_over(true, 10.0, end));
    }
}
