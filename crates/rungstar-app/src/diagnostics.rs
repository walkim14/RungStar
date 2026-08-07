//! Microphone, pitch and controller diagnostics.
//!
//! The first thing worth running on real hardware. It answers what a test suite cannot: are
//! the microphones found, does the left/right channel split put two singers on one adapter,
//! does the pitch detector follow a real voice, and does the gamepad work.
//!
//! Deliberately drawn with SDL's own renderer and its built-in debug font rather than the
//! game's renderer. This is a tool, not a screen, and it should keep working even while the
//! renderer is being rewritten around it.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rungstar_audio::{CaptureBackend, DeviceConfig, PlayerBuffers, CHANNEL_OFF, MAX_PLAYERS};
use rungstar_pitch::{Algorithm, Analyzer, AnalyzerConfig, Detection};
use rungstar_platform::{Action, InputMapper, SdlCapture};
use sdl3::event::Event;
use sdl3::pixels::Color;
use sdl3::render::FRect;

/// The Steam Deck's native resolution, so layout problems show up here first.
const WINDOW_SIZE: (u32, u32) = (1280, 800);
const SAMPLE_RATE: u32 = 44_100;
/// How often pitch is re-analysed. Far above the beat rate, well below the cost ceiling.
const ANALYSIS_INTERVAL: Duration = Duration::from_millis(10);

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

const BACKGROUND: Color = Color::RGB(16, 18, 24);
const PANEL: Color = Color::RGB(28, 32, 42);
const TEXT: Color = Color::RGB(226, 232, 240);
const DIM: Color = Color::RGB(120, 132, 150);
const ACCENT: Color = Color::RGB(96, 200, 255);
const HOT: Color = Color::RGB(255, 196, 96);

/// What one player's row shows.
struct PlayerState {
    analyzer: Analyzer,
    latest: Option<Detection>,
    /// Peak level seen recently, so brief peaks stay visible between frames.
    peak: f32,
    /// Which device and channel feeds this player, for the label.
    source: String,
}

/// Map every recording device's channels onto players, in order.
///
/// Channel-by-channel rather than device-by-device, because the usual karaoke adapter is one
/// stereo device carrying two singers.
fn auto_configure(capture: &SdlCapture) -> Result<Vec<DeviceConfig>> {
    let devices = capture
        .devices()
        .context("could not list capture devices")?;
    let mut configs = Vec::new();
    let mut next_player = 1u8;

    for device in devices {
        let mut config = DeviceConfig::silent(&device.name, device.channels);
        for channel in 0..device.channels {
            if usize::from(next_player) > MAX_PLAYERS {
                break;
            }
            config.assign(channel, next_player);
            next_player += 1;
        }
        if config.channel_to_player.iter().any(|p| *p != CHANNEL_OFF) {
            configs.push(config);
        }
    }
    Ok(configs)
}

fn build_players(configs: &[DeviceConfig]) -> Vec<PlayerState> {
    let mut players: Vec<PlayerState> = Vec::new();
    for config in configs {
        for (channel, &player) in config.channel_to_player.iter().enumerate() {
            if player == CHANNEL_OFF {
                continue;
            }
            let side = match (channel, config.channels()) {
                (0, 2) => "left",
                (1, 2) => "right",
                (c, _) => return_channel_label(c),
            };
            while players.len() < usize::from(player) {
                players.push(PlayerState {
                    analyzer: Analyzer::new(AnalyzerConfig {
                        sample_rate: SAMPLE_RATE,
                        ..AnalyzerConfig::default()
                    }),
                    latest: None,
                    peak: 0.0,
                    source: String::new(),
                });
            }
            players[usize::from(player) - 1].source = format!("{} ({side})", config.name);
        }
    }
    players
}

/// Label for a channel on a device with more than two of them.
fn return_channel_label(channel: usize) -> &'static str {
    const LABELS: [&str; 8] = ["ch1", "ch2", "ch3", "ch4", "ch5", "ch6", "ch7", "ch8"];
    LABELS.get(channel).copied().unwrap_or("ch?")
}

fn main() -> Result<()> {
    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!("SDL init failed: {e}"))?;
    let video = sdl
        .video()
        .map_err(|e| anyhow::anyhow!("no video subsystem: {e}"))?;
    let audio = sdl
        .audio()
        .map_err(|e| anyhow::anyhow!("no audio subsystem: {e}"))?;
    let gamepads = sdl
        .gamepad()
        .map_err(|e| anyhow::anyhow!("no gamepad subsystem: {e}"))?;

    let window = video
        .window("RungStar diagnostics", WINDOW_SIZE.0, WINDOW_SIZE.1)
        .position_centered()
        .resizable()
        .build()
        .context("could not open a window")?;
    let mut canvas = window.into_canvas();

    let mut capture = SdlCapture::new(audio);
    let configs = auto_configure(&capture)?;
    let mut players = build_players(&configs);

    println!("capture devices in use:");
    for config in &configs {
        println!(
            "  {} -> channels {:?}",
            config.name, config.channel_to_player
        );
    }
    if let Err(error) = capture.start(&configs, SAMPLE_RATE) {
        // Keep going: the controller half of the tool is still worth having, and a failure
        // here is exactly the kind of thing the tool exists to report.
        println!("capture could not start: {error}");
    }

    let mut mapper = InputMapper::default();
    let mut event_pump = sdl
        .event_pump()
        .map_err(|e| anyhow::anyhow!("no event pump: {e}"))?;
    let mut open_pads = Vec::new();
    let mut buffers = PlayerBuffers::new();
    let mut algorithm = Algorithm::Mpm;
    let mut last_action = String::from("none yet");
    let mut last_analysis = Instant::now();
    let mut frames = 0u32;
    let mut fps = 0.0f32;
    let mut fps_since = Instant::now();

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
                        last_action = format!("{:?} (keyboard)", input.action);
                        match input.action {
                            Action::Back => break 'running,
                            Action::Left | Action::Right => {
                                algorithm = match algorithm {
                                    Algorithm::Mpm => Algorithm::Camdf,
                                    Algorithm::Camdf => Algorithm::Mpm,
                                };
                                for player in &mut players {
                                    let mut config = *player.analyzer.config();
                                    config.algorithm = algorithm;
                                    player.analyzer.set_config(config);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::ControllerDeviceAdded { which, .. } => {
                    // A pad must be opened before SDL reports its buttons.
                    match gamepads.open(sdl3::joystick::JoystickId::new(which)) {
                        Ok(pad) => {
                            last_action =
                                format!("gamepad connected: {}", pad.name().unwrap_or_default());
                            open_pads.push(pad);
                        }
                        Err(error) => last_action = format!("gamepad open failed: {error}"),
                    }
                }
                Event::ControllerDeviceRemoved { which, .. } => {
                    mapper.forget_gamepad(which);
                    open_pads.retain(|pad| pad.id().is_ok_and(|id| id.0 != which));
                    last_action = "gamepad disconnected".to_owned();
                }
                Event::ControllerButtonDown { which, button, .. } => {
                    match mapper.button(which, button, true) {
                        Some(input) => {
                            last_action = format!("{:?} (pad {which}, {button:?})", input.action);
                            if input.action == Action::Back {
                                break 'running;
                            }
                        }
                        None => last_action = format!("unbound pad button {button:?}"),
                    }
                }
                Event::ControllerAxisMotion {
                    which, axis, value, ..
                } => {
                    for input in mapper.axis(which, axis, value) {
                        if input.pressed {
                            last_action = format!("{:?} (pad {which}, stick)", input.action);
                        }
                    }
                }
                _ => {}
            }
        }

        buffers.clear();
        if let Err(error) = capture.drain(&mut buffers) {
            last_action = format!("capture error: {error}");
        }
        for (index, player) in players.iter_mut().enumerate() {
            let samples = buffers.player(index as u8 + 1);
            if !samples.is_empty() {
                player.analyzer.push(samples);
            }
            // Decay the peak so the meter falls back rather than sticking at a shout.
            player.peak = (player.peak * 0.92).max(player.analyzer.volume());
        }

        if last_analysis.elapsed() >= ANALYSIS_INTERVAL {
            last_analysis = Instant::now();
            for player in &mut players {
                player.latest = player.analyzer.detect();
            }
        }

        frames += 1;
        if fps_since.elapsed() >= Duration::from_millis(500) {
            fps = frames as f32 / fps_since.elapsed().as_secs_f32();
            frames = 0;
            fps_since = Instant::now();
        }

        draw(
            &mut canvas,
            &players,
            algorithm,
            &last_action,
            fps,
            open_pads.len(),
        )?;
    }

    capture.stop();
    Ok(())
}

/// Draw the whole screen: a header, then one row per player.
fn draw(
    canvas: &mut sdl3::render::Canvas<sdl3::video::Window>,
    players: &[PlayerState],
    algorithm: Algorithm,
    last_action: &str,
    fps: f32,
    pads: usize,
) -> Result<()> {
    let (width, height) = canvas.output_size().map_err(|e| anyhow::anyhow!("{e}"))?;
    let width = width as f32;

    canvas.set_draw_color(BACKGROUND);
    canvas.clear();

    canvas.set_draw_color(TEXT);
    let _ = canvas.draw_debug_text("RungStar diagnostics", (16.0, 14.0));
    canvas.set_draw_color(DIM);
    let _ = canvas.draw_debug_text(
        &format!(
            "algorithm {algorithm:?}  (left/right to switch)   gamepads {pads}   {fps:.0} fps   \
             escape to quit"
        ),
        (16.0, 32.0),
    );
    let _ = canvas.draw_debug_text(&format!("last input: {last_action}"), (16.0, 50.0));

    if players.is_empty() {
        canvas.set_draw_color(HOT);
        let _ = canvas.draw_debug_text(
            "no capture devices found - plug in a microphone and restart",
            (16.0, 90.0),
        );
        canvas.present();
        return Ok(());
    }

    let top = 80.0;
    let row_height = ((height as f32 - top - 16.0) / players.len() as f32).min(120.0);

    for (index, player) in players.iter().enumerate() {
        let y = top + index as f32 * row_height;
        let panel = FRect::new(16.0, y, width - 32.0, row_height - 8.0);
        canvas.set_draw_color(PANEL);
        let _ = canvas.fill_rect(panel);

        canvas.set_draw_color(TEXT);
        let _ = canvas.draw_debug_text(
            &format!("Player {}  -  {}", index + 1, player.source),
            (28.0, y + 10.0),
        );

        // Level meter. Its purpose is to prove the right microphone reaches the right player,
        // so it is the widest thing on the row.
        let meter = FRect::new(28.0, y + 30.0, width - 380.0, 14.0);
        canvas.set_draw_color(Color::RGB(48, 54, 68));
        let _ = canvas.fill_rect(meter);
        let filled = (player.peak.clamp(0.0, 1.0) * meter.w).max(1.0);
        canvas.set_draw_color(if player.peak > 0.9 { HOT } else { ACCENT });
        let _ = canvas.fill_rect(FRect::new(meter.x, meter.y, filled, meter.h));

        match &player.latest {
            Some(detection) => {
                let name = NOTE_NAMES[detection.pitch_class.rem_euclid(12) as usize];
                let octave = detection.halftone / 12 + 2;
                let frequency = detection
                    .frequency
                    .map_or_else(|| "-".to_owned(), |f| format!("{f:7.1} Hz"));
                canvas.set_draw_color(ACCENT);
                let _ = canvas.draw_debug_text(
                    &format!(
                        "{name}{octave:<2} {frequency}  clarity {:.2}",
                        detection.clarity
                    ),
                    (width - 330.0, y + 30.0),
                );

                // Piano-roll marker: where in C2..C6 the detected note sits.
                let span = width - 380.0;
                let position =
                    f32::from(detection.halftone as i16) / rungstar_pitch::HALFTONE_COUNT as f32;
                let marker = FRect::new(28.0 + position * span, y + 50.0, 6.0, 18.0);
                canvas.set_draw_color(ACCENT);
                let _ = canvas.fill_rect(marker);
            }
            None => {
                canvas.set_draw_color(DIM);
                let _ = canvas.draw_debug_text("silent", (width - 330.0, y + 30.0));
            }
        }

        // The semitone grid, so the marker above has something to be read against.
        canvas.set_draw_color(Color::RGB(44, 50, 62));
        let span = width - 380.0;
        for halftone in (0..rungstar_pitch::HALFTONE_COUNT).step_by(12) {
            let x = 28.0 + halftone as f32 / rungstar_pitch::HALFTONE_COUNT as f32 * span;
            let _ = canvas.fill_rect(FRect::new(x, y + 50.0, 1.0, 18.0));
        }
    }

    canvas.present();
    Ok(())
}
