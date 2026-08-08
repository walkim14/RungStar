//! Point the linker at the vendored FFmpeg and put its DLLs next to the executable.
//!
//! The same arrangement as `rungstar-platform`'s SDL3 build script, and for the same reason: a
//! clean machine should be able to build and run the game without installing anything first.
//! On Linux nothing is vendored and FFmpeg comes from the system package.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if !cfg!(target_os = "windows") {
        return;
    }

    let root = workspace_root();
    let vendor = root.join("vendor").join("ffmpeg");
    if !vendor.is_dir() {
        println!("cargo:warning=vendor/ffmpeg is missing; expecting FFmpeg from the system");
        return;
    }

    // `FFMPEG_DIR` is set in `.cargo/config.toml`, not here: a build script cannot hand an
    // environment variable to another crate's build script, and `ffmpeg-sys` is the one that
    // needs it.
    println!(
        "cargo:rustc-link-search=native={}",
        vendor.join("lib").display()
    );

    // The DLLs have to sit beside the executable, because Windows looks there and not in a
    // vendor directory. Copied on every build so an updated vendor tree takes effect.
    if let Some(out) = executable_directory() {
        if let Ok(entries) = std::fs::read_dir(vendor.join("bin")) {
            for entry in entries.filter_map(Result::ok) {
                let from = entry.path();
                if from.extension().and_then(|e| e.to_str()) == Some("dll") {
                    let to = out.join(entry.file_name());
                    let _ = std::fs::copy(&from, &to);
                }
            }
        }
    }
}

/// The workspace root, walking up from this crate.
fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    manifest
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").is_file() || dir.join("vendor").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or(manifest)
}

/// Where cargo is putting the binaries this build feeds into.
///
/// `OUT_DIR` is `target/<profile>/build/<crate>-<hash>/out`, so the profile directory is four
/// levels up. Fragile in principle, and the alternative — an env var cargo does not provide —
/// does not exist.
fn executable_directory() -> Option<PathBuf> {
    let out = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    out.ancestors().nth(3).map(Path::to_path_buf)
}
