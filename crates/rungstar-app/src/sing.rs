//! Singing a song: the game, reduced to its core loop.
//!
//! Everything below this file has been built and tested separately — the parser, the pitch
//! detectors, the scoring model, the clock, capture routing, decoding, playback. This wires
//! them together and draws the result.
//!
//! Usage: `rungstar-sing <song.txt>`
//!
//! The clock is the part worth understanding. Playback reports where the audio actually is;
//! the clock is dragged toward that gradually rather than snapped to it, because device
//! positions arrive quantised and jittery and following them directly makes the beat visibly
//! stutter. Notes are drawn from the visual beat and scored from the detection beat, which
//! trails it by the microphone round trip.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use rungstar_audio::{AudioClip, CaptureBackend, DeviceConfig, MasterClock, PlayerBuffers, Timing};
use rungstar_pitch::{Analyzer, AnalyzerConfig};
use rungstar_platform::{Action, InputMapper, Playback, SdlCapture};
use rungstar_score::{Difficulty, Rating, ScoreTrack, Scorer};
use rungstar_song::{Line, SongTxt};
use sdl3::event::Event;
use sdl3::pixels::Color;
use sdl3::render::FRect;

const WINDOW_SIZE: (u32, u32) = (1280, 800);
const SAMPLE_RATE: u32 = 44_100;
const ANALYSIS_INTERVAL: Duration = Duration::from_millis(10);

const BACKGROUND: Color = Color::RGB(12, 14, 20);
const STAFF: Color = Color::RGB(26, 30, 40);
const GRID: Color = Color::RGB(44, 50, 64);
const TARGET: Color = Color::RGB(92, 104, 130);
const GOLDEN: Color = Color::RGB(255, 200, 90);
const SUNG_HIT: Color = Color::RGB(120, 230, 160);
const SUNG_MISS: Color = Color::RGB(220, 96, 110);
const TEXT: Color = Color::RGB(232, 238, 248);
const DIM: Color = Color::RGB(120, 132, 152);
const ACTIVE: Color = Color::RGB(120, 210, 255);
const WARN: Color = Color::RGB(255, 140, 110);

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Everything the loop needs about the song being sung.
struct Session {
    song: SongTxt,
    scorer: Scorer,
    /// Lines of the track being sung, alongside the scorer's flattened copy.
    lines: Vec<Line>,
    /// Index of the next line whose bonus has not been awarded yet.
    next_line: usize,
    /// Highest detection beat already scored, so no beat is counted twice.
    scored_through: f64,
    last_rating: Option<(i32, Instant)>,
    /// The most recent scored beat, so the screen can say plainly whether it counted.
    last_beat: Option<(rungstar_score::BeatResult, Instant)>,
}

/// Find a file beside the song, tolerating a case mismatch in the header.
fn resolve_beside(directory: &Path, name: &str) -> Option<PathBuf> {
    let direct = directory.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        })
        .map(|entry| entry.path())
}

/// Names that belong to virtual or loopback devices rather than a microphone.
///
/// These sit at the top of the device list on a machine with Steam installed and produce
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
    VIRTUAL_DEVICES.iter().any(|marker| lower.contains(marker))
}

/// Pick a capture device, with its first channel feeding player one.
///
/// `preference` matches case-insensitively on any part of the device name, so `--mic auna`
/// is enough. Without one, the first device that does not look virtual wins.
fn choose_microphone(capture: &SdlCapture, preference: Option<&str>) -> Option<DeviceConfig> {
    let devices = capture.devices().ok()?;
    if devices.is_empty() {
        println!("no capture devices at all");
        return None;
    }

    println!("capture devices:");
    for device in &devices {
        let note = if looks_virtual(&device.name) {
            "  (virtual)"
        } else {
            ""
        };
        println!("  {}{note}", device.name);
    }

    let chosen = match preference {
        Some(wanted) => {
            let wanted = wanted.to_lowercase();
            devices
                .iter()
                .find(|d| d.name.to_lowercase().contains(&wanted))
                .or_else(|| {
                    println!("no device matches '{wanted}', falling back to automatic choice");
                    devices.iter().find(|d| !looks_virtual(&d.name))
                })
        }
        None => devices.iter().find(|d| !looks_virtual(&d.name)),
    }
    .or_else(|| devices.first())?;

    println!(
        "using: {}   (override with --mic <part of the name>)",
        chosen.name
    );
    let mut config = DeviceConfig::silent(&chosen.name, chosen.channels);
    config.assign(0, 1);
    Some(config)
}

/// Apply an action. Returns true when the player wants to stop.
fn handle(action: Action, playback: &mut Playback, clock: &mut MasterClock) -> Result<bool> {
    match action {
        Action::Back => return Ok(true),
        Action::Pause => {
            if playback.is_playing() {
                let _ = playback.pause();
                clock.pause();
            } else {
                let _ = playback.start();
                clock.start();
            }
        }
        _ => {}
    }
    Ok(false)
}

/// Score every whole detection beat that has passed since the last frame.
///
/// Driven off the clock rather than the frame rate, so the score does not depend on how fast
/// the machine draws.
fn score_elapsed_beats(session: &mut Session, detection_beat: f64, sung: Option<i32>) {
    if session.scored_through.is_infinite() {
        session.scored_through = detection_beat - 1.0;
    }
    for beat in MasterClock::beats_crossed(session.scored_through, detection_beat) {
        let result = session.scorer.sing_beat(beat, sung);
        if result.target.is_some() {
            session.last_beat = Some((result, Instant::now()));
        }

        // Close any line the beat has now passed the end of.
        while session.next_line < session.lines.len() {
            let line = &session.lines[session.next_line];
            if line.notes.is_empty() || beat < line.end() {
                break;
            }
            if let Some(result) = session.scorer.end_line(session.next_line) {
                session.last_rating = Some((result.rating, Instant::now()));
            }
            session.next_line += 1;
        }
    }
    session.scored_through = detection_beat;
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut song_path: Option<PathBuf> = None;
    let mut mic_preference: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mic" => mic_preference = args.next(),
            other => song_path = Some(PathBuf::from(other)),
        }
    }
    let Some(path) = song_path else {
        eprintln!("usage: rungstar-sing <song.txt> [--mic <part of the device name>]");
        std::process::exit(2);
    };

    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let parsed = SongTxt::parse_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("{} is not a usable song: {e}", path.display()))?;
    for warning in &parsed.warnings {
        eprintln!("note: {warning}");
    }
    let song = parsed.song;

    let directory = path.parent().unwrap_or(Path::new("."));
    let Some(audio_name) = song.headers.audio_file() else {
        bail!("{} names no audio file", path.display());
    };
    let audio_path = resolve_beside(directory, audio_name)
        .with_context(|| format!("the audio file '{audio_name}' is not next to the song"))?;

    println!("{}", song.headers.artist_title());

    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!("SDL init failed: {e}"))?;
    let video = sdl.video().map_err(|e| anyhow::anyhow!("no video: {e}"))?;
    let audio_subsystem = sdl.audio().map_err(|e| anyhow::anyhow!("no audio: {e}"))?;
    let gamepads = sdl
        .gamepad()
        .map_err(|e| anyhow::anyhow!("no gamepad subsystem: {e}"))?;

    let title = format!("RungStar - {}", song.headers.artist_title());
    let window = video
        .window(&title, WINDOW_SIZE.0, WINDOW_SIZE.1)
        .position_centered()
        .resizable()
        .build()
        .context("could not open a window")?;
    let mut canvas = window.into_canvas();

    let clip = AudioClip::open(&audio_path).context("could not decode the audio")?;
    // Start with a cushion so playback does not stall on the first frames.
    clip.wait_for(0.5, Duration::from_secs(20));
    if let Some(error) = clip.error() {
        bail!("audio decoding failed: {error}");
    }
    let mut playback = Playback::new(&audio_subsystem, clip)
        .map_err(|e| anyhow::anyhow!("could not open the output device: {e}"))?;

    // One microphone, one singer. The routing supports far more; this is the smallest thing
    // that proves the whole chain end to end.
    let mut capture = SdlCapture::new(audio_subsystem);
    let mic = choose_microphone(&capture, mic_preference.as_deref());
    if mic.is_none() {
        println!("no microphone - the song will play but nothing will score");
    }
    if let Some(config) = &mic {
        if let Err(error) = capture.start(std::slice::from_ref(config), SAMPLE_RATE) {
            println!("capture could not start: {error}");
        }
    }

    let lines = song.tracks.track_1.clone();
    let track = ScoreTrack::from_lines(&lines);
    let timing = Timing::new(song.bpm().value(), song.headers.gap as f64);
    let mut session = Session {
        scorer: Scorer::new(track, Difficulty::Medium),
        lines,
        next_line: 0,
        scored_through: f64::NEG_INFINITY,
        last_rating: None,
        last_beat: None,
        song,
    };

    let mut clock = MasterClock::new(timing);
    let mut analyzer = Analyzer::new(AnalyzerConfig {
        sample_rate: SAMPLE_RATE,
        ..AnalyzerConfig::default()
    });
    let mut buffers = PlayerBuffers::new();
    let mapper = InputMapper::default();
    let mut event_pump = sdl
        .event_pump()
        .map_err(|e| anyhow::anyhow!("no event pump: {e}"))?;
    let mut open_pads = Vec::new();

    playback
        .start()
        .map_err(|e| anyhow::anyhow!("could not start playback: {e}"))?;
    clock.start();

    let mut last_frame = Instant::now();
    let mut last_analysis = Instant::now();
    let mut detection: Option<rungstar_pitch::Detection> = None;
    let mut level = 0.0f32;
    let mut ever_heard = false;

    'running: loop {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => break 'running,
                Event::KeyDown {
                    keycode: Some(key),
                    repeat,
                    ..
                } => {
                    if let Some(input) = mapper.key(key, true, repeat) {
                        if handle(input.action, &mut playback, &mut clock)? {
                            break 'running;
                        }
                    }
                }
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Ok(pad) = gamepads.open(sdl3::joystick::JoystickId::new(which)) {
                        open_pads.push(pad);
                    }
                }
                Event::ControllerButtonDown { which, button, .. } => {
                    if let Some(input) = mapper.button(which, button, true) {
                        if handle(input.action, &mut playback, &mut clock)? {
                            break 'running;
                        }
                    }
                }
                _ => {}
            }
        }

        playback
            .pump()
            .map_err(|e| anyhow::anyhow!("playback failed: {e}"))?;
        let elapsed = last_frame.elapsed();
        last_frame = Instant::now();
        clock.tick(elapsed);
        clock.synchronize(playback.position());

        buffers.clear();
        let _ = capture.drain(&mut buffers);
        let samples = buffers.player(1);
        if !samples.is_empty() {
            analyzer.push(samples);
            ever_heard = true;
        }
        if last_analysis.elapsed() >= ANALYSIS_INTERVAL {
            last_analysis = Instant::now();
            detection = analyzer.detect();
            // Decays rather than tracking instantaneously, so a meter reading is legible
            // instead of flickering.
            level = (level * 0.8).max(analyzer.volume());
        }
        let detected = detection.map(|d| d.pitch_class);

        let beats = clock.beats();
        if clock.is_paused() {
            // A paused clock must not keep scoring silence.
            session.scored_through = beats.detection;
        } else {
            score_elapsed_beats(&mut session, beats.detection, detected);
        }

        let done = playback.is_finished() || session.next_line >= session.lines.len();
        let hud = Hud {
            level,
            threshold: analyzer.config().threshold,
            detection,
            ever_heard,
            has_microphone: mic.is_some(),
        };
        draw(&mut canvas, &session, beats.visual, &hud, done)?;
        if done && playback.is_finished() {
            break 'running;
        }
    }

    capture.stop();
    let _ = playback.pause();

    let totals = session.scorer.totals();
    println!();
    println!("notes      {:>6}", totals.notes);
    println!("golden     {:>6}", totals.golden);
    println!("line bonus {:>6}", totals.line_bonus);
    println!(
        "total      {:>6}   {}",
        totals.total,
        Rating::from_score(totals.total).as_str()
    );
    Ok(())
}

/// The line being sung at `beat`, or the next one due.
///
/// Lyrics have to appear slightly before they are sung or nobody can read them in time, so
/// this returns the upcoming line once the previous one has ended.
fn current_line(session: &Session, beat: f64) -> Option<usize> {
    let beat = beat as i32;
    session
        .lines
        .iter()
        .position(|line| !line.notes.is_empty() && beat < line.end())
        .or(session.lines.len().checked_sub(1))
}

/// What the input side of the screen needs to show.
struct Hud {
    /// Peak level of the analysis window, `0.0..=1.0`.
    level: f32,
    /// The gate below which pitch detection does not even run.
    threshold: f32,
    detection: Option<rungstar_pitch::Detection>,
    /// Whether a single sample has ever arrived from the device.
    ever_heard: bool,
    has_microphone: bool,
}

fn draw(
    canvas: &mut sdl3::render::Canvas<sdl3::video::Window>,
    session: &Session,
    beat: f64,
    hud: &Hud,
    finished: bool,
) -> Result<()> {
    let (width, height) = canvas.output_size().map_err(|e| anyhow::anyhow!("{e}"))?;
    let (width, height) = (width as f32, height as f32);

    canvas.set_draw_color(BACKGROUND);
    canvas.clear();

    let totals = session.scorer.totals();
    canvas.set_draw_color(TEXT);
    let _ = canvas.draw_debug_text(&session.song.headers.artist_title(), (24.0, 20.0));
    let _ = canvas.draw_debug_text(&format!("{:>6}", totals.total), (width - 90.0, 20.0));

    if finished {
        let rating = Rating::from_score(totals.total);
        canvas.set_draw_color(ACTIVE);
        let _ = canvas.draw_debug_text(
            &format!("{}  -  {} points", rating.as_str(), totals.total),
            (width / 2.0 - 140.0, height / 2.0),
        );
        canvas.set_draw_color(DIM);
        let _ = canvas.draw_debug_text(
            &format!(
                "notes {}   golden {}   line bonus {}",
                totals.notes, totals.golden, totals.line_bonus
            ),
            (width / 2.0 - 140.0, height / 2.0 + 22.0),
        );
        canvas.present();
        return Ok(());
    }

    let Some(index) = current_line(session, beat) else {
        canvas.present();
        return Ok(());
    };
    let line = &session.lines[index];
    if line.notes.is_empty() {
        canvas.present();
        return Ok(());
    }

    // The staff shows exactly one line of lyrics, the way UltraStar does: the notes of the
    // phrase being sung, stretched across the width, so their relative timing is readable.
    let staff = FRect::new(40.0, 140.0, width - 80.0, height - 340.0);
    canvas.set_draw_color(STAFF);
    let _ = canvas.fill_rect(staff);

    let start = f64::from(line.start());
    let span = f64::from(line.end() - line.start()).max(1.0);
    let pitches: Vec<i32> = line.notes.iter().map(|n| n.pitch).collect();
    let low = pitches.iter().copied().min().unwrap_or(0) - 2;
    let high = pitches.iter().copied().max().unwrap_or(12) + 2;
    let range = f64::from(high - low).max(1.0);

    let x_of = |b: f64| staff.x + ((b - start) / span) as f32 * staff.w;
    let y_of = |pitch: f64| staff.y + staff.h - ((pitch - f64::from(low)) / range) as f32 * staff.h;
    let note_height = (staff.h / range as f32).clamp(6.0, 20.0);

    // Semitone grid, so pitch distance is visible rather than implied.
    canvas.set_draw_color(GRID);
    for step in low..=high {
        let y = y_of(f64::from(step));
        let _ = canvas.fill_rect(FRect::new(staff.x, y, staff.w, 1.0));
    }

    // Target notes.
    for note in &line.notes {
        let x = x_of(f64::from(note.start));
        let w = ((f64::from(note.duration) / span) as f32 * staff.w).max(3.0);
        let y = y_of(f64::from(note.pitch)) - note_height / 2.0;
        canvas.set_draw_color(if note.kind.is_golden() {
            GOLDEN
        } else {
            TARGET
        });
        let _ = canvas.fill_rect(FRect::new(x, y, w, note_height));
    }

    // What the singer actually produced, over the top.
    for sung in session.scorer.sung_notes() {
        let sung_start = f64::from(sung.start);
        if sung_start + f64::from(sung.duration) < start || sung_start > start + span {
            continue;
        }
        let x = x_of(sung_start);
        let w = ((f64::from(sung.duration) / span) as f32 * staff.w).max(3.0);
        // Sung pitch is a pitch class; place it in the octave the line is written in.
        let folded = rungstar_score::fold_to_octave(sung.tone, line.notes[0].pitch);
        let y = y_of(f64::from(folded)) - note_height / 4.0;
        canvas.set_draw_color(if sung.hit { SUNG_HIT } else { SUNG_MISS });
        let _ = canvas.fill_rect(FRect::new(x, y, w, note_height / 2.0));
    }

    // Where the singer is right now, drawn whether or not a note is due. This is the line
    // that makes a working microphone visible: without it, silence and a dead device look
    // exactly the same.
    if let Some(detection) = &hud.detection {
        let folded = rungstar_score::fold_to_octave(detection.pitch_class, line.notes[0].pitch);
        let y = y_of(f64::from(folded));
        let head = x_of(beat).clamp(staff.x, staff.x + staff.w);
        canvas.set_draw_color(SUNG_HIT);
        let _ = canvas.fill_rect(FRect::new(staff.x, y - 1.0, head - staff.x, 2.0));
        let _ = canvas.fill_rect(FRect::new(head - 6.0, y - 6.0, 12.0, 12.0));
    }

    // The playhead.
    canvas.set_draw_color(ACTIVE);
    let _ = canvas.fill_rect(FRect::new(
        x_of(beat).clamp(staff.x, staff.x + staff.w),
        staff.y,
        2.0,
        staff.h,
    ));

    // Lyrics, with the syllable being sung picked out.
    let mut x = 40.0;
    let lyric_y = height - 150.0;
    for note in &line.notes {
        let text = note.text.replace('~', "");
        if text.is_empty() {
            continue;
        }
        let active =
            beat >= f64::from(note.start) && beat < f64::from(note.start + note.duration.max(1));
        canvas.set_draw_color(if active { ACTIVE } else { TEXT });
        let _ = canvas.draw_debug_text(&text, (x, lyric_y));
        // SDL's debug font is a fixed 8 pixels wide per character.
        x += text.chars().count() as f32 * 8.0;
    }

    // The next line, dimmed, so there is time to read ahead.
    if let Some(next) = session.lines.get(index + 1) {
        canvas.set_draw_color(DIM);
        let _ = canvas.draw_debug_text(&next.text().replace('~', ""), (40.0, lyric_y + 24.0));
    }

    if let Some((rating, shown)) = session.last_rating {
        if shown.elapsed() < Duration::from_millis(1200) {
            canvas.set_draw_color(GOLDEN);
            let _ = canvas.draw_debug_text(&format!("{rating}/8"), (width - 90.0, 44.0));
        }
    }

    draw_input_panel(canvas, hud, session.last_beat.as_ref(), height);

    canvas.set_draw_color(DIM);
    let _ = canvas.draw_debug_text(
        &format!("beat {beat:>7.1}   [p] pause   [esc] quit"),
        (24.0, height - 26.0),
    );

    canvas.present();
    Ok(())
}

/// The strip that answers "is my microphone working, and am I hitting the note".
///
/// Three separate questions, three separate readouts, because they fail independently: the
/// device may deliver nothing, it may deliver something too quiet to pitch, or it may be
/// working perfectly while the singer is on the wrong note.
fn draw_input_panel(
    canvas: &mut sdl3::render::Canvas<sdl3::video::Window>,
    hud: &Hud,
    last_beat: Option<&(rungstar_score::BeatResult, Instant)>,
    height: f32,
) {
    let y = height - 92.0;

    // 1. Is audio arriving at all?
    let meter = FRect::new(24.0, y, 320.0, 16.0);
    canvas.set_draw_color(Color::RGB(40, 46, 60));
    let _ = canvas.fill_rect(meter);
    let filled = (hud.level.clamp(0.0, 1.0) * meter.w).max(1.0);
    let loud_enough = hud.level >= hud.threshold;
    canvas.set_draw_color(if loud_enough { SUNG_HIT } else { DIM });
    let _ = canvas.fill_rect(FRect::new(meter.x, meter.y, filled, meter.h));
    // The gate: below this mark nothing is even analysed.
    canvas.set_draw_color(WARN);
    let _ = canvas.fill_rect(FRect::new(
        meter.x + hud.threshold * meter.w,
        y - 3.0,
        2.0,
        22.0,
    ));

    let status = if !hud.has_microphone {
        "no microphone selected".to_owned()
    } else if !hud.ever_heard {
        "no audio from the device yet".to_owned()
    } else if !loud_enough {
        format!(
            "too quiet to score  ({:.0}% of the gate)",
            hud.level / hud.threshold * 100.0
        )
    } else {
        "hearing you".to_owned()
    };
    canvas.set_draw_color(if hud.has_microphone && hud.ever_heard {
        DIM
    } else {
        WARN
    });
    let _ = canvas.draw_debug_text(&status, (24.0, y + 24.0));

    // 2. What pitch is it hearing?
    canvas.set_draw_color(TEXT);
    let pitch = match &hud.detection {
        Some(detection) => {
            let name = NOTE_NAMES[detection.pitch_class.rem_euclid(12) as usize];
            let octave = detection.halftone / 12 + 2;
            match detection.frequency {
                Some(hz) => format!("you: {name}{octave}  {hz:>6.1} Hz"),
                None => format!("you: {name}{octave}"),
            }
        }
        None => "you: -".to_owned(),
    };
    let _ = canvas.draw_debug_text(&pitch, (400.0, y));

    // 3. Did the last note count?
    if let Some((result, at)) = last_beat {
        if at.elapsed() < Duration::from_millis(600) {
            let (label, colour) = if result.hit {
                ("HIT", SUNG_HIT)
            } else {
                ("MISS", SUNG_MISS)
            };
            canvas.set_draw_color(colour);
            let _ = canvas.draw_debug_text(label, (400.0, y + 24.0));
            if let Some(target) = result.target_tone {
                canvas.set_draw_color(DIM);
                let name = NOTE_NAMES[target.rem_euclid(12) as usize];
                let _ = canvas.draw_debug_text(&format!("target {name}"), (460.0, y + 24.0));
            }
        }
    }
}
