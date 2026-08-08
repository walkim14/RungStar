//! Decode the opening of some real files and report what happened.
//!
//! `cargo run --release --example decode_check -p rungstar-audio -- <folder> [count]`
//!
//! Written after a real library turned out to be almost entirely Ogg Vorbis, which the
//! decoder had not been built with support for. Nothing said so: the songs simply refused to
//! play, which read as flakiness rather than as a missing codec.

use std::time::Duration;

use rungstar_audio::AudioClip;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: decode_check <folder> [count]");
        std::process::exit(2);
    };
    let wanted: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(20);

    // One audio file per song folder, so the sample spans the library rather than one artist.
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut dirs: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        // Spread across the library rather than taking the first N alphabetically.
        let step = (dirs.len() / wanted.max(1)).max(1);
        for directory in dirs.iter().step_by(step) {
            if files.len() >= wanted {
                break;
            }
            if let Ok(inner) = std::fs::read_dir(directory) {
                for entry in inner.filter_map(Result::ok) {
                    let path = entry.path();
                    let audio = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase())
                        .is_some_and(|e| {
                            matches!(
                                e.as_str(),
                                "ogg" | "mp3" | "m4a" | "flac" | "wav" | "opus" | "webm" | "aac"
                            )
                        });
                    if audio {
                        files.push(path);
                        break;
                    }
                }
            }
        }
    }
    if files.is_empty() {
        eprintln!("no audio files found under {root}");
        std::process::exit(1);
    }

    let mut ok = 0;
    let mut by_kind: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for path in &files {
        let kind = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("?")
            .to_lowercase();
        let entry = by_kind.entry(kind).or_default();
        entry.1 += 1;
        let shown = path.file_name().unwrap_or_default().to_string_lossy();
        match AudioClip::open(path) {
            Ok(clip) => {
                // How fast the decoder runs relative to real time decides whether a preview
                // can seek into the middle of a song at all: seeking to fifty seconds needs
                // fifty seconds of audio decoded first.
                let started = std::time::Instant::now();
                let ready = clip.wait_for(30.0, Duration::from_secs(10));
                let elapsed = started.elapsed().as_secs_f64().max(0.0001);
                match clip.error() {
                    Some(error) => println!(
                        "FAIL  {shown}
        {error}"
                    ),
                    None => {
                        ok += usize::from(ready);
                        entry.0 += usize::from(ready);
                        println!(
                            "{}  {:>5.1} s in {:>5.2} s = {:>4.0}x realtime  {shown}",
                            if ready { "ok  " } else { "SLOW" },
                            clip.ready_secs(),
                            elapsed,
                            clip.ready_secs() / elapsed
                        );
                    }
                }
            }
            Err(error) => println!(
                "FAIL  {shown}
        {error}"
            ),
        }
    }

    println!();
    for (kind, (good, total)) in by_kind {
        println!("{kind:<6}{good} of {total}");
    }
    println!("{ok} of {} decoded", files.len());
}
