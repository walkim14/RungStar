//! Write the menu music to a WAV, so it can be listened to and tweaked.
//!
//!     cargo run --release --example menu_music -p rungstar-platform -- [out.wav]
//!
//! The loop is synthesised at startup rather than shipped, which means the only way to hear a
//! change to `chiptune.rs` would otherwise be to start the game and open a menu. This writes it
//! out instead, twice through, so the loop point can be heard.

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "menu.wav".into());
    let once = rungstar_platform::chiptune::render();
    // Twice, because the join is the part worth checking and it only exists at the seam.
    let twice: Vec<i16> = once.iter().chain(once.iter()).copied().collect();

    let rate = 44_100u32;
    let bytes = twice.len() * 2;
    let mut wav: Vec<u8> = Vec::with_capacity(44 + bytes);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + bytes) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(bytes as u32).to_le_bytes());
    for sample in &twice {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&path, &wav).expect("writing the wav");

    let peak = once.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    println!(
        "wrote {path}: {:.1}s loop, twice through, peak {:.0}% of full scale",
        rungstar_platform::chiptune::length_secs(),
        f32::from(peak) / 327.67
    );
}
