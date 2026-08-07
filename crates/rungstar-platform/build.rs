//! Points the linker at the vendored SDL3 and puts its DLL where the executable can find it.
//!
//! Windows only. Elsewhere SDL3 comes from the system, and the crate links against it the
//! usual way.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let vendor = manifest
        .parent()
        .and_then(Path::parent)
        .expect("crate lives two levels below the workspace root")
        .join("vendor/sdl3/lib/x64");

    if !vendor.join("SDL3.lib").exists() {
        println!(
            "cargo:warning=vendored SDL3 not found at {}",
            vendor.display()
        );
        return;
    }
    println!("cargo:rustc-link-search=native={}", vendor.display());

    // A dynamically linked SDL3 has to sit beside the executable at run time. Copying it
    // during the build means `cargo run` and `cargo test` both just work.
    if let Some(target_dir) = executable_dir() {
        let destination = target_dir.join("SDL3.dll");
        let source = vendor.join("SDL3.dll");
        if source.exists() {
            let _ = std::fs::create_dir_all(&target_dir);
            if let Err(error) = std::fs::copy(&source, &destination) {
                println!("cargo:warning=could not copy SDL3.dll: {error}");
            }
        }
    }
}

/// Walk up from `OUT_DIR` to the profile directory, which is where binaries land.
fn executable_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    // .../target/<profile>/build/<pkg>-<hash>/out
    out_dir.ancestors().nth(3).map(Path::to_path_buf)
}
