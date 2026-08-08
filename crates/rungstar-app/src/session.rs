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
use rungstar_ui::singscreen::{Note, NoteKind, Singer, Sung, Syllable};

const SAMPLE_RATE: u32 = 44_100;

/// How often the microphones are analysed. UltraStar's own rate, and well inside what the
/// detectors cost even for six singers.
const ANALYSIS_INTERVAL: Duration = Duration::from_millis(10);

/// How much of what was sung stays on screen behind the playhead, in beats.
const SUNG_HISTORY: f64 = 24.0;

/// How long after the last note the song is considered over, in beats.
const TAIL_BEATS: f64 = 8.0;

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
    last_note_beat: f64,
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
        let devices = choose_devices(&capture, players);
        let has_microphone = !devices.is_empty();
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
        let last_note_beat = notes.iter().map(|n| n.end()).fold(0.0, f64::max);

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
            last_note_beat,
        })
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
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

        // Over when the audio runs out, or when the last note is well behind — a song with a
        // long silent outro should not hold the screen for it.
        if self.playback.is_finished() || beats.visual > self.last_note_beat + TAIL_BEATS {
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

        // Forget what has scrolled off, so a long song does not grow without bound.
        let cutoff = detection_beat - SUNG_HISTORY;
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

    /// The syllables of the line being sung, and the text of the one after it.
    ///
    /// Lyrics have to appear slightly before they are sung or nobody can read them in time,
    /// so this returns the upcoming line once the previous one has ended.
    pub fn lyrics(&self, beat: f64) -> (Vec<Syllable>, String) {
        let whole = beat as i32;
        let index = self
            .lines
            .iter()
            .position(|line| !line.notes.is_empty() && whole < line.end())
            .or(self.lines.len().checked_sub(1));
        let Some(index) = index else {
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

/// Flatten the track's notes for drawing.
fn collect_notes(lines: &[Line]) -> Vec<Note> {
    lines
        .iter()
        .flat_map(|line| line.notes.iter())
        .map(|note| Note {
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
        })
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

/// Choose devices for `players` singers, one channel each.
///
/// Channels are filled before devices are, so a stereo microphone pair carries two singers.
/// That is how the cheap dual-USB sets work, and the only way to reach six singers without
/// six separate devices.
pub fn choose_devices(capture: &SdlCapture, players: usize) -> Vec<DeviceConfig> {
    let Ok(devices) = capture.devices() else {
        return Vec::new();
    };
    let mut configs = Vec::new();
    let mut assigned = 0u8;
    for (index, device) in devices.into_iter().enumerate() {
        if assigned as usize >= players {
            break;
        }
        if looks_virtual(&device.name) {
            continue;
        }
        let channels = device.channels.max(1);
        let mut mapping = vec![0u8; channels];
        for slot in mapping.iter_mut() {
            if (assigned as usize) < players {
                assigned += 1;
                *slot = assigned;
            }
        }
        configs.push(DeviceConfig {
            name: device.name.clone(),
            input_index: index as u32,
            latency_ms: rungstar_audio::capture::LATENCY_AUTODETECT,
            channel_to_player: mapping,
        });
    }
    configs
}
