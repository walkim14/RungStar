//! Decode real song videos and report what came out.
//!
//! `cargo run --release --example playback_check -p rungstar-video -- <song folder> [count]`
//!
//! Checks the two things that decide whether a video is usable during a song: that it decodes
//! at all, and that it decodes fast enough. Anything under a few times realtime would mean the
//! decoder cannot keep ahead of the music on a slower machine than this one.

use std::time::Instant;

use rungstar_video::Video;

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: playback_check <song folder> [count]");
    let wanted: usize = std::env::args()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or(10);

    let mut videos = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut dirs: Vec<_> = entries
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
                    let extension = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or_default()
                        .to_lowercase();
                    if matches!(extension.as_str(), "mp4" | "webm" | "avi" | "mkv") {
                        videos.push(path);
                        break;
                    }
                }
            }
        }
    }
    if videos.is_empty() {
        eprintln!("no videos found under {root}");
        std::process::exit(1);
    }

    let mut played = 0;
    let mut slowest = f64::MAX;
    for path in &videos {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut video = match Video::open(path, 0.0) {
            Ok(video) => video,
            Err(error) => {
                println!("FAIL  {name}\n        {error}");
                continue;
            }
        };

        // Decoding happens on another thread, so the first frame takes a moment to arrive.
        // An earlier version of this harness walked the whole song in a tenth of a millisecond
        // and concluded nothing had decoded.
        let waiting = Instant::now();
        while video.frame_at(0.0).is_none() && waiting.elapsed().as_secs_f64() < 5.0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        // Walk ten seconds of song in real time, asking for the frame at each step the way the
        // sing screen does. Real time, because a decoder that keeps up only when nothing else
        // is happening does not keep up.
        let started = Instant::now();
        let mut seen = 0;
        let mut last = f64::MIN;
        let mut first_at = None;
        while started.elapsed().as_secs_f64() < 2.0 {
            let position = started.elapsed().as_secs_f64() * 5.0;
            if let Some(frame) = video.frame_at(position) {
                if frame.at > last {
                    if first_at.is_none() {
                        first_at = Some(frame.at);
                    }
                    last = frame.at;
                    seen += 1;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        // Ten seconds of song walked in two seconds of wall clock, so five times realtime is
        // the bar the decoder had to clear.
        let elapsed = started.elapsed().as_secs_f64().max(0.0001);
        let times_realtime = last.max(0.0) / elapsed;
        slowest = slowest.min(times_realtime);

        match (seen, video.error()) {
            (_, Some(error)) => println!("ERROR {name}\n        {error}"),
            (0, None) => println!("NONE  {name}  (no frame was ever due)"),
            (n, None) => {
                played += 1;
                let (w, h) = video.size().unwrap_or((0, 0));
                println!(
                    "ok    {n:>3} frames {w}x{h}  first at {:>4.2}s  {times_realtime:>4.1}x realtime  {name}",
                    first_at.unwrap_or(0.0)
                );
            }
        }
    }

    println!();
    println!("{played} of {} played", videos.len());
    if slowest < f64::MAX {
        println!("slowest {slowest:.0}x realtime");
    }
}
