//! Measure how loud a sample of a real library is, and how uneven it is.
//!
//!     cargo run --release --example loudness_check -p rungstar-audio -- <folder> [count]

use rungstar_audio::{loudness, AudioClip};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args.next().expect("a folder");
    let wanted: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(40);

    let mut files = Vec::new();
    walk(std::path::Path::new(&root), &mut files, wanted);
    println!("measuring {} files", files.len());

    let mut measured: Vec<(loudness::Measured, String)> = Vec::new();
    let started = std::time::Instant::now();
    for path in &files {
        let Ok(clip) = AudioClip::open(path) else {
            continue;
        };
        clip.wait_for(1.0e9, std::time::Duration::from_secs(30));
        let frames = clip.ready_frames();
        let channels = clip.channels().max(1);
        let mut samples = vec![0i16; frames * channels];
        let read = clip.read(0, &mut samples);
        samples.truncate(read * channels);
        if let Some(found) = loudness::analyse(&samples, channels, clip.sample_rate()) {
            let name = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            measured.push((found, name));
        }
    }
    measured.sort_by(|a, b| a.0.lufs.partial_cmp(&b.0.lufs).unwrap());

    println!("{} measured in {:.1?}", measured.len(), started.elapsed());
    if measured.is_empty() {
        return;
    }
    for (found, name) in measured.iter().take(3) {
        println!(
            "  quietest {:7.1} LUFS  peak {:.2}  x{:.2}  {name}",
            found.lufs,
            found.peak,
            loudness::playback_gain(*found)
        );
    }
    for (found, name) in measured.iter().rev().take(3) {
        println!(
            "  loudest  {:7.1} LUFS  peak {:.2}  x{:.2}  {name}",
            found.lufs,
            found.peak,
            loudness::playback_gain(*found)
        );
    }
    let spread = measured.last().unwrap().0.lufs - measured[0].0.lufs;
    let mean = measured.iter().map(|m| m.0.lufs).sum::<f32>() / measured.len() as f32;
    println!("  spread   {spread:.1} dB, mean {mean:.1} LUFS");

    // What the spread becomes once every song is played at its own gain: the number the
    // complaint is actually about.
    let after: Vec<f32> = measured
        .iter()
        .map(|(m, _)| m.lufs + 20.0 * loudness::playback_gain(*m).log10())
        .collect();
    let low = after.iter().copied().fold(f32::MAX, f32::min);
    let high = after.iter().copied().fold(f32::MIN, f32::max);
    println!(
        "  after    {:.1} dB spread ({low:.1} to {high:.1} LUFS)",
        high - low
    );
}

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, wanted: usize) {
    if out.len() >= wanted {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= wanted {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out, wanted);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ogg" | "mp3" | "m4a" | "flac" | "opus" | "wav")
        ) {
            out.push(path);
        }
    }
}
