//! Reproduce the browser's preview path headlessly and report why each one failed.
//!
//! `cargo run --release --example preview_check -p rungstar-platform -- <song folder> [count]`
//!
//! Written because "most songs do not preview" is unanswerable from the game: the preview code
//! swallowed every failure with `.ok()?`, so a missing file, a refused device and a bad seek
//! all looked identical — like nothing happening.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rungstar_audio::AudioClip;
use rungstar_platform::Playback;

/// Find a file beside the song, tolerating a case mismatch in the header.
fn resolve_beside(directory: &Path, name: &str) -> Option<PathBuf> {
    let direct = directory.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    let wanted = name.to_lowercase();
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().to_lowercase() == wanted)
        .map(|entry| entry.path())
}

/// The `#MP3:` or `#AUDIO:` header, without pulling in the whole parser.
fn audio_header(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines().take(60) {
        let upper = line.to_uppercase();
        for tag in ["#MP3:", "#AUDIO:"] {
            if let Some(rest) = upper.strip_prefix(tag) {
                let _ = rest;
                return Some(line[tag.len()..].trim().to_owned());
            }
        }
    }
    None
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: preview_check <song folder> [count]");
        std::process::exit(2);
    };
    let wanted: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(25);

    let sdl = sdl3::init().expect("sdl init");
    let audio = sdl.audio().expect("audio subsystem");

    let mut songs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        let step = (dirs.len() / wanted.max(1)).max(1);
        for directory in dirs.iter().step_by(step).take(wanted) {
            if let Ok(inner) = std::fs::read_dir(directory) {
                for entry in inner.filter_map(Result::ok) {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("txt") {
                        songs.push(path);
                        break;
                    }
                }
            }
        }
    }

    let mut played = 0;
    let mut reasons: std::collections::BTreeMap<&str, usize> = Default::default();
    for song in &songs {
        let shown = song
            .parent()
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let directory = song.parent().unwrap_or(Path::new("."));

        let Some(name) = audio_header(song) else {
            println!("no header    {shown}");
            *reasons.entry("no audio header").or_default() += 1;
            continue;
        };
        let Some(path) = resolve_beside(directory, &name) else {
            println!("missing      {shown}  ({name})");
            *reasons.entry("file not beside the song").or_default() += 1;
            continue;
        };
        let clip = match AudioClip::open(&path) {
            Ok(clip) => clip,
            Err(error) => {
                println!("undecodable  {shown}\n             {error}");
                *reasons.entry("could not open").or_default() += 1;
                continue;
            }
        };
        clip.wait_for(0.4, Duration::from_millis(400));
        if let Some(error) = clip.error() {
            println!("decode error {shown}\n             {error}");
            *reasons.entry("decode error").or_default() += 1;
            continue;
        }

        let mut playback = match Playback::new(&audio, clip) {
            Ok(playback) => playback,
            Err(error) => {
                println!("no device    {shown}\n             {error}");
                *reasons.entry("output device refused").or_default() += 1;
                continue;
            }
        };
        // The browser seeks a quarter of the way in, which is past the intro on almost
        // everything. Reaching that point needs that much audio decoded.
        let length = playback.duration();
        let start = length * 0.25;
        playback
            .clip()
            .wait_for(start + 1.0, Duration::from_secs(2));
        let _ = playback.seek(start);
        playback.set_volume(0.0);
        if let Err(error) = playback.start() {
            println!("no start     {shown}\n             {error}");
            *reasons.entry("stream would not resume").or_default() += 1;
            continue;
        }

        // Pump the way the frame loop does and see whether the position actually advances.
        let began = Instant::now();
        let mut pumped = 0usize;
        while began.elapsed() < Duration::from_millis(250) {
            if playback.pump().is_ok() {
                pumped += 1;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        let moved = playback.position() - start;
        if moved > 0.02 {
            played += 1;
            println!("ok           {shown}  (+{moved:.2} s in {pumped} pumps)");
        } else {
            println!("silent       {shown}  (seek {start:.1} s of {length:.1} s, no movement)");
            *reasons.entry("played nothing after seeking").or_default() += 1;
        }
        let _ = playback.pause();
    }

    println!();
    println!("{played} of {} previewed", songs.len());
    for (reason, count) in reasons {
        println!("  {count:>4}  {reason}");
    }
}
