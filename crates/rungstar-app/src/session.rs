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
use rungstar_party::{Effects, Watch};
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

/// Whether a beat continues the previous run of sung marks.
///
/// The interesting half is `same_place`. A hit continues for as long as it is the same note,
/// whatever pitch the detector reported: within the tolerance band every one of those beats
/// counts and they are all drawn on the note, so splitting the run because the detector moved
/// a semitone turns one bubble into a row of touching ones. A miss splits per pitch, because
/// a miss is drawn where the singer actually was and two different wrong notes are two
/// different pieces of information.
fn continues_run(last: Option<&Sung>, beat: f64, pitch: i32, hit: bool, same_note: bool) -> bool {
    let Some(last) = last else {
        return false;
    };
    let contiguous = (last.start + last.duration - beat).abs() < 1.5;
    let same_place = if hit { same_note } else { last.pitch == pitch };
    contiguous && last.hit == hit && same_place
}

/// How a song is to be played this time.
///
/// Kept as one value rather than four more arguments, and defaulted, so the ordinary case
/// stays `Plan::default()` and a medley or a challenge is the same call with one field set.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Where to begin, in seconds. `None` starts at the top.
    pub start_secs: Option<f64>,
    /// Where to stop, in seconds. Narrows `#END`; it never extends it.
    pub end_secs: Option<f64>,
    /// What the challenge changes.
    pub effects: Effects,
    /// Only used by the Deaf challenge, and taken rather than drawn so a run is reproducible.
    pub seed: u64,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            start_secs: None,
            end_secs: None,
            effects: Effects::PLAIN,
            seed: 0,
        }
    }
}

/// One song being played.
pub struct Session {
    clock: MasterClock,
    playback: Playback,
    buffers: PlayerBuffers,
    analysers: Vec<Analyzer>,
    scorers: Vec<Scorer>,
    /// The lines of each part. One entry for an ordinary song, two for a duet.
    parts: Vec<Vec<Line>>,
    /// Which part each singer sings.
    singer_part: Vec<usize>,
    /// The names the song gives its parts.
    part_names: Vec<String>,
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
    /// The last beat any note ends on, for knowing when the outro has started.
    last_note_beat: f64,
    /// The challenge rules, watching the same lines the scorer does.
    watch: Watch,
    /// The line of the first part that the watch has already been told about.
    watched_line: usize,
    /// Each singer's rating for the line they last finished, for the watch.
    line_rating: Vec<f32>,
    /// How loud the song plays when it is not being cut out by a challenge.
    volume: f32,
    /// Whether the backing track is audible right now.
    audible: bool,
    /// The song's video, when it has one and videos are on.
    video: Option<rungstar_video::Video>,
    /// The note each singer's last recorded run was scored against.
    ///
    /// Runs merge on the note, not on the pitch. A singer holding one note wobbles across the
    /// tolerance band — 60, 61, 60 — and every one of those beats is a hit, drawn at the
    /// note's own pitch. Splitting the run whenever the detector moved produced a row of
    /// separate bubbles sitting against each other where there should be one.
    last_target: Vec<Option<usize>>,
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
        video_path: Option<&Path>,
        mut capture: SdlCapture,
        plan: &Plan,
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

        // A duet has two tracks and its singers are split between them: odd-numbered players
        // take the first part, even the second. That is UltraStar's arrangement, and it means
        // two people with two microphones each get their own part without configuring
        // anything.
        let mut parts = vec![song.tracks.track_1.clone()];
        if let Some(second) = song.tracks.track_2.clone() {
            parts.push(second);
        }
        let part_names = if parts.len() > 1 {
            vec![
                song.headers
                    .p1
                    .clone()
                    .unwrap_or_else(|| "Part 1".to_owned()),
                song.headers
                    .p2
                    .clone()
                    .unwrap_or_else(|| "Part 2".to_owned()),
            ]
        } else {
            Vec::new()
        };
        let singer_part: Vec<usize> = (0..players.max(1)).map(|i| i % parts.len()).collect();
        let tracks: Vec<ScoreTrack> = parts
            .iter()
            .map(|lines| ScoreTrack::from_lines(lines))
            .collect();
        let mut timing = Timing::new(song.bpm().value(), song.headers.gap as f64);
        timing.mic_delay = mic_delay_ms / 1000.0;

        // The pitch scale and the end of the song come from every part, so a duet's two
        // staves share one scale and a note sits at the same height on both.
        let notes: Vec<Note> = parts
            .iter()
            .flat_map(|lines| collect_notes(lines))
            .collect();
        let last_note_beat = notes.iter().map(Note::end).fold(0.0, f64::max);
        // `#END` is in milliseconds, unlike `#START` and `#PREVIEWSTART` which are seconds.
        // The format is inconsistent about this and it is an easy place to be wrong.
        // A video that will not open is not worth refusing to sing over.
        let video = video_path.and_then(|path| {
            let gap = song.headers.videogap.unwrap_or(0.0);
            match rungstar_video::Video::open(path, gap) {
                Ok(video) => Some(video),
                Err(error) => {
                    tracing::warn!("no video for this song: {error}");
                    None
                }
            }
        });

        let end_secs = song
            .headers
            .end
            .filter(|ms| *ms > 0)
            .map(|ms| ms as f64 / 1000.0);
        // A plan narrows the song, never widens it: a medley that runs past `#END` would play
        // over whatever the file was trimmed to avoid.
        let end_secs = match (end_secs, plan.end_secs) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };

        let mut clock = MasterClock::new(timing);
        // Seek before starting, so the first frame is already at the medley point rather than
        // playing a moment of the intro and jumping.
        if let Some(start) = plan.start_secs.filter(|s| *s > 0.0) {
            let _ = playback.seek(start);
            clock.seek(start);
        }
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
                .map(|player| Scorer::new(tracks[player % tracks.len()].clone(), difficulty))
                .collect(),
            parts,
            singer_part,
            part_names,
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
            video,
            last_note_beat,
            last_target: vec![None; players],
            watch: Watch::new(plan.effects, players, last_note_beat.max(1.0), plan.seed),
            watched_line: 0,
            line_rating: vec![0.0; players],
            volume: 1.0,
            audible: true,
        })
    }

    /// Change how loud the song plays, while it is playing.
    ///
    /// Remembered rather than only applied, because the Deaf challenge cuts the track in and
    /// out underneath and has to put it back to whatever the player actually chose.
    /// The audio being played, for measuring how loud it is.
    pub fn clip(&self) -> &rungstar_audio::AudioClip {
        self.playback.clip()
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.playback
            .set_volume(if self.audible { volume } else { 0.0 });
    }

    /// The challenge rules watching this song.
    pub fn watch(&self) -> &Watch {
        &self.watch
    }

    /// Whether the backing track is audible, for the screen to say so.
    pub fn audible(&self) -> bool {
        self.audible
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

        // Deaf cuts the backing track in and out. Muting rather than pausing on purpose: the
        // clock has to keep running or the notes stop with the music and there is nothing left
        // to sing against.
        if !self.clock.is_paused() {
            let audible = self.watch.music_at(self.playback.position());
            if audible != self.audible {
                self.audible = audible;
                self.playback
                    .set_volume(if audible { self.volume } else { 0.0 });
            }
        }

        if song_over(
            self.playback.is_finished(),
            self.playback.position(),
            self.end_secs,
        ) {
            self.watch.song_ended();
        }
        if self.watch.ending().is_some() {
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
                if let Some(target) = result.target {
                    self.hitting[player] = Some(result.hit);
                    if let Some(pitch) = sung {
                        self.record_sung(player, beat, pitch, target, result.hit);
                    }
                }

                // Close any line the beat has now passed the end of.
                let lines = &self.parts[self.singer_part[player]];
                while self.next_line[player] < lines.len() {
                    let line = &lines[self.next_line[player]];
                    if line.notes.is_empty() || beat < line.end() {
                        break;
                    }
                    if let Some(result) = self.scorers[player].end_line(self.next_line[player]) {
                        self.ratings[player] = Some((result.rating, Instant::now()));
                        self.line_rating[player] = result.perfection as f32;
                    }
                    self.next_line[player] += 1;
                }
            }
        }
        self.scored_through = detection_beat;
        self.watch_lines(detection_beat);

        // Forget what is behind the line being sung, so a long song does not grow without
        // bound, but never anything still on screen.
        let cutoff = self
            .line_at(0, detection_beat)
            .and_then(|index| self.parts[0].get(index))
            .map(|line| line.start() as f64 - SUNG_KEEP_BEFORE_LINE)
            .unwrap_or(detection_beat - 64.0);
        for history in &mut self.sung {
            history.retain(|s| s.start + s.duration >= cutoff);
        }
    }

    /// Tell the challenge rules about every line the first singer has finished.
    ///
    /// Driven by the first part rather than per singer, because a challenge compares people to
    /// each other and that only means anything at a shared moment. In a duet the parts do not
    /// share line boundaries, so one of them has to be the clock.
    fn watch_lines(&mut self, beat: f64) {
        let finished = self.next_line.first().copied().unwrap_or(0);
        if finished <= self.watched_line {
            return;
        }
        self.watched_line = finished;
        let scores: Vec<i32> = self
            .scorers
            .iter()
            .map(|scorer| scorer.totals().total as i32)
            .collect();
        self.watch.line_ended(beat, &scores, &self.line_rating);
        self.line_rating.fill(0.0);
    }

    /// Extend the last run rather than pushing a bar per beat, so a held note draws as one
    /// stretch instead of a dotted line.
    ///
    /// A hit continues the run for as long as it is the same note, whatever pitch the
    /// detector reported: within the tolerance band every one of those beats counts, and they
    /// are all drawn on the note, so ending the run because the detector moved a semitone
    /// splits one bubble into several touching ones. A miss is kept separate per pitch,
    /// because a miss is drawn where the singer actually was.
    fn record_sung(&mut self, player: usize, beat: i32, pitch: i32, target: usize, hit: bool) {
        let beat = beat as f64;
        let same_note = self.last_target[player] == Some(target);
        let continues = continues_run(self.sung[player].last(), beat, pitch, hit, same_note);

        if continues {
            if let Some(last) = self.sung[player].last_mut() {
                last.duration = (beat + 1.0) - last.start;
            }
        } else {
            self.sung[player].push(Sung {
                start: beat,
                duration: 1.0,
                pitch,
                hit,
            });
        }
        self.last_target[player] = Some(target);
    }

    /// The video frame belonging to this moment, if the song has a video.
    ///
    /// Driven off the audio position rather than a wall clock, so a video that cannot keep up
    /// drops frames instead of dragging the song out of time with the singing.
    pub fn video_frame(&mut self) -> Option<&rungstar_video::Frame> {
        let position = self.playback.position();
        self.video.as_mut()?.frame_at(position)
    }

    /// The video's shape, for letterboxing it.
    pub fn video_aspect(&self) -> Option<f32> {
        self.video.as_ref()?.aspect()
    }

    /// Whether the last note has gone by, so there is nothing left to sing.
    pub fn past_last_note(&self) -> bool {
        self.clock.beats().visual > self.last_note_beat
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
    fn line_at(&self, part: usize, beat: f64) -> Option<usize> {
        let lines = self.parts.get(part)?;
        let whole = beat as i32;
        lines
            .iter()
            .position(|line| !line.notes.is_empty() && whole < line.end())
            .or_else(|| lines.iter().rposition(|line| !line.notes.is_empty()))
    }

    /// How many parts the song has: one, or two for a duet.
    pub fn part_count(&self) -> usize {
        self.parts.len()
    }

    /// The names the song gives its parts, empty when it is not a duet.
    pub fn part_names(&self) -> &[String] {
        &self.part_names
    }

    /// Which part each singer is on.
    pub fn singer_parts(&self) -> &[usize] {
        &self.singer_part
    }

    /// How many people this session is scoring.
    ///
    /// The screen takes its panel count from here rather than from the settings, because the
    /// two disagree the moment somebody picks fewer singers than there are microphones, and a
    /// panel with no scorer behind it sits at zero for the whole song.
    pub fn players(&self) -> usize {
        self.scorers.len()
    }

    /// The notes of the line being sung, and the beats it spans.
    pub fn current_line(&self, part: usize, beat: f64) -> NoteLine {
        let Some(index) = self.line_at(part, beat) else {
            return NoteLine::default();
        };
        let line = &self.parts[part][index];
        let notes: Vec<Note> = line
            .notes
            .iter()
            .map(|note| Note {
                part,
                ..convert_note(note)
            })
            .collect();
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
    pub fn lyrics(&self, part: usize, beat: f64) -> (Vec<Syllable>, String) {
        let Some(index) = self.line_at(part, beat) else {
            return (Vec::new(), String::new());
        };
        let syllables = self.parts[part][index]
            .notes
            .iter()
            .map(|note| Syllable {
                text: note.text.clone(),
                start: note.start as f64,
                duration: note.duration as f64,
                golden: note.kind.is_golden(),
            })
            .collect();
        let next = self.parts[part]
            .get(index + 1)
            .map(|line| line.text())
            .unwrap_or_default();
        (syllables, next)
    }
}

/// Turn a parsed note into one the screen can draw.
fn convert_note(note: &rungstar_song::Note) -> Note {
    Note {
        // Filled in by the caller for a duet's second part; a single-part song is all part
        // zero and the staff does not colour by it.
        part: 0,
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

/// What to call a device on screen.
///
/// Two identical microphones report identical names, so the list shows the same line twice and
/// there is no way to tell which meter belongs to which hand. Numbered only when there is more
/// than one — a lone microphone called "Logitech USB Microphone (1)" invites the question of
/// where the other one is.
fn label_for(device: &DeviceConfig, all: &[DeviceConfig]) -> String {
    let same_name = all.iter().filter(|d| d.name == device.name).count();
    if same_name < 2 {
        return device.name.clone();
    }
    format!("{} ({})", device.name, device.occurrence + 1)
}

/// Apply a saved assignment to the devices that are actually present.
///
/// Matched by name **and which device of that name**, and anything unrecognised keeps whatever
/// was worked out for it, so plugging in a new microphone does not discard the setup for the
/// old ones. Matching on the name alone gave both of a pair of identical microphones the same
/// assignment, which is the same singer twice and nobody on the other one.
pub fn apply_saved(devices: &mut [DeviceConfig], saved: &[rungstar_ui::settings::MicAssignment]) {
    for device in devices.iter_mut() {
        let found = saved
            .iter()
            .find(|s| s.name == device.name && s.occurrence == device.occurrence)
            // A setting written before occurrences existed has none, and there is only one
            // sensible thing it can mean: the first device of that name.
            .or_else(|| {
                saved
                    .iter()
                    .find(|s| s.name == device.name && s.occurrence == 0 && device.occurrence == 0)
            });
        if let Some(entry) = found {
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
        .map(|(_, device)| DeviceConfig {
            name: device.name.clone(),
            occurrence: device.occurrence,
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
                occurrence: device.occurrence,
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
                // A slot per channel, with no ceiling. Clamping at the singer limit put every
                // channel past the sixth into the same slot, so two microphones shared a
                // meter and the ones after them reported nothing arriving.
                let mapping = (0..device.channels())
                    .map(|_| {
                        *next += 1;
                        *next
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
                name: label_for(device, &self.devices),
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

    fn mic(name: &str, occurrence: u32) -> DeviceConfig {
        DeviceConfig {
            name: name.to_owned(),
            occurrence,
            latency_ms: rungstar_audio::capture::LATENCY_AUTODETECT,
            channel_to_player: vec![0, 0],
        }
    }

    fn saved(name: &str, occurrence: u32, channels: &[u8]) -> rungstar_ui::settings::MicAssignment {
        rungstar_ui::settings::MicAssignment {
            name: name.to_owned(),
            occurrence,
            channels: channels.to_vec(),
        }
    }

    #[test]
    fn two_microphones_of_the_same_model_are_two_microphones() {
        // A pair of identical USB karaoke microphones report identical names — which is the
        // ordinary way somebody ends up with two, not a curiosity. Matched on the name alone,
        // both got the first one's assignment: the same singer twice, and nobody at all on the
        // other microphone, with nothing on screen to say why.
        let mut found = vec![mic("Logitech USB Mic", 0), mic("Logitech USB Mic", 1)];
        apply_saved(
            &mut found,
            &[
                saved("Logitech USB Mic", 0, &[1, 0]),
                saved("Logitech USB Mic", 1, &[2, 0]),
            ],
        );
        assert_eq!(found[0].channel_to_player, vec![1, 0]);
        assert_eq!(found[1].channel_to_player, vec![2, 0]);
    }

    #[test]
    fn a_setting_written_before_they_were_told_apart_still_reads() {
        // No `occurrence` in the file means the first device of that name, which is what it
        // meant when it was written. It must not silently apply to the second one as well.
        let mut found = vec![mic("Built-in Microphone", 0), mic("Built-in Microphone", 1)];
        let mut old = saved("Built-in Microphone", 0, &[3, 0]);
        old.occurrence = 0;
        apply_saved(&mut found, &[old]);
        assert_eq!(found[0].channel_to_player, vec![3, 0]);
        assert_eq!(
            found[1].channel_to_player,
            vec![0, 0],
            "the twin was claimed"
        );
    }

    #[test]
    fn duplicates_are_numbered_and_a_lone_microphone_is_not() {
        // A single microphone called "Logitech USB Mic (1)" invites the question of where the
        // other one is.
        let pair = vec![mic("Logitech USB Mic", 0), mic("Logitech USB Mic", 1)];
        assert_eq!(label_for(&pair[0], &pair), "Logitech USB Mic (1)");
        assert_eq!(label_for(&pair[1], &pair), "Logitech USB Mic (2)");

        let alone = vec![mic("Built-in Microphone", 0)];
        assert_eq!(label_for(&alone[0], &alone), "Built-in Microphone");
    }

    #[test]
    fn each_of_a_pair_gets_its_own_singer() {
        // The automatic assignment, which is what somebody sees before they touch anything:
        // one singer per device before a second channel of any device.
        let mut found = vec![mic("Logitech USB Mic", 0), mic("Logitech USB Mic", 1)];
        let mut assigned = 0u8;
        for config in found.iter_mut() {
            assigned += 1;
            config.channel_to_player[0] = assigned;
        }
        assert_eq!(found[0].channel_to_player[0], 1);
        assert_eq!(found[1].channel_to_player[0], 2);
        assert_ne!(
            found[0].occurrence, found[1].occurrence,
            "they must be openable as different devices"
        );
    }

    #[test]
    fn a_song_plays_its_outro_rather_than_stopping_on_the_last_note() {
        // The bug this replaces: the end was eight *grid* beats after the last note, and the
        // grid runs at four times the written BPM. At 300 in the file that is 1200 beats a
        // minute, so eight of them is four tenths of a second — the song stopped dead on its
        // final syllable every time.
        assert!(!song_over(false, 200.0, None), "audio still playing");
        assert!(song_over(true, 200.0, None), "audio ran out");
    }

    fn run(start: f64, duration: f64, pitch: i32, hit: bool) -> Sung {
        Sung {
            start,
            duration,
            pitch,
            hit,
        }
    }

    #[test]
    fn a_held_note_stays_one_bubble_while_the_detector_wobbles() {
        // The reported defect: a singer holding one note drifts across the tolerance band,
        // 60 then 61 then 60, and every beat is a hit. They were drawn as separate bubbles
        // sitting against each other instead of one.
        let last = run(8.0, 1.0, 60, true);
        assert!(
            continues_run(Some(&last), 9.0, 61, true, true),
            "a wobble within the same note split the run"
        );
        assert!(continues_run(Some(&last), 9.0, 59, true, true));
        assert!(continues_run(Some(&last), 9.0, 60, true, true));
    }

    #[test]
    fn a_new_note_starts_a_new_bubble() {
        // Two hits in a row against different notes are two bubbles, however close in pitch.
        let last = run(8.0, 1.0, 60, true);
        assert!(!continues_run(Some(&last), 9.0, 60, true, false));
    }

    #[test]
    fn a_miss_is_kept_apart_from_a_hit_and_from_another_pitch() {
        let hit = run(8.0, 1.0, 60, true);
        assert!(
            !continues_run(Some(&hit), 9.0, 60, false, true),
            "hit and miss merged"
        );

        // Two different wrong notes are two different pieces of information.
        let miss = run(8.0, 1.0, 64, false);
        assert!(continues_run(Some(&miss), 9.0, 64, false, true));
        assert!(!continues_run(Some(&miss), 9.0, 67, false, true));
    }

    #[test]
    fn a_gap_starts_a_new_run() {
        // Stopping and starting again on the same note is two marks, because the silence
        // between them is the thing worth seeing.
        let last = run(8.0, 1.0, 60, true);
        assert!(continues_run(Some(&last), 9.0, 60, true, true));
        assert!(!continues_run(Some(&last), 12.0, 60, true, true));
        assert!(!continues_run(None, 9.0, 60, true, true));
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
