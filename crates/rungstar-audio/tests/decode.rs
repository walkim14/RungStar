//! Decoding real audio files.

use std::io::Write;
use std::time::Duration;

use rungstar_audio::AudioClip;

/// Write a canonical 16-bit PCM WAV so the decoder has a real file to open.
fn write_wav(path: &std::path::Path, sample_rate: u32, channels: u16, frames: usize) -> Vec<i16> {
    let samples: Vec<i16> = (0..frames * channels as usize)
        .map(|i| {
            let frame = (i / channels as usize) as f64;
            let phase = frame / f64::from(sample_rate) * 440.0 * std::f64::consts::TAU;
            (phase.sin() * 12_000.0) as i16
        })
        .collect();

    let data_len = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * u32::from(channels) * 2;
    let mut file = std::fs::File::create(path).expect("create wav");
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&(channels * 2).to_le_bytes()).unwrap(); // block align
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_len.to_le_bytes()).unwrap();
    for sample in &samples {
        file.write_all(&sample.to_le_bytes()).unwrap();
    }
    samples
}

#[test]
fn a_wav_decodes_to_the_samples_it_contains() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("tone.wav");
    let expected = write_wav(&path, 44_100, 2, 4_410);

    let clip = AudioClip::open(&path).expect("open wav");
    assert_eq!(clip.sample_rate(), 44_100);
    assert_eq!(clip.channels(), 2);

    assert!(
        clip.wait_for(0.09, Duration::from_secs(10)),
        "decoder should keep up"
    );
    while !clip.is_complete() {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(clip.error(), None);
    assert_eq!(clip.ready_frames(), 4_410);

    let mut out = vec![0i16; 200];
    let frames = clip.read(0, &mut out);
    assert_eq!(frames, 100, "100 stereo frames fill 200 samples");
    assert_eq!(&out[..20], &expected[..20]);
}

#[test]
fn reading_beyond_what_is_decoded_returns_a_short_read() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("tone.wav");
    write_wav(&path, 44_100, 2, 1_000);

    let clip = AudioClip::open(&path).expect("open wav");
    while !clip.is_complete() {
        std::thread::sleep(Duration::from_millis(5));
    }

    // Ask for more than exists: the read stops at the end rather than padding with silence.
    let mut out = vec![0i16; 4_000];
    assert_eq!(clip.read(900, &mut out), 100);
    // Entirely past the end.
    assert_eq!(clip.read(5_000, &mut out), 0);
}

#[test]
fn reading_is_positioned_by_frame_not_by_sample() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("tone.wav");
    let expected = write_wav(&path, 44_100, 2, 500);

    let clip = AudioClip::open(&path).expect("open wav");
    while !clip.is_complete() {
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut out = vec![0i16; 8];
    clip.read(10, &mut out);
    // Frame 10 of a stereo file starts at sample 20.
    assert_eq!(&out[..8], &expected[20..28]);
}

#[test]
fn a_file_that_is_not_audio_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nonsense.wav");
    std::fs::write(&path, b"this is not a wave file").unwrap();
    assert!(AudioClip::open(&path).is_err());
}

#[test]
fn a_missing_file_reports_the_path() {
    let error = AudioClip::open("no/such/song.mp3").unwrap_err();
    assert!(error.to_string().contains("song.mp3"), "got: {error}");
}
