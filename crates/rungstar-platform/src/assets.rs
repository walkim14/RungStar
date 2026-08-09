//! Where the shipped assets are.
//!
//! Fonts and sounds are both committed under `assets/` and both have to be found from three
//! quite different working directories — a packaged build, a `target/release` binary run from
//! the source tree, and `cargo test`. One search shared by both, because the failure of having
//! two is that a release ships the fonts and silently loses the sounds.

use std::path::PathBuf;

/// Every place `assets/<kind>/<name>` might be, best first.
///
/// Returned rather than resolved so the caller decides what a miss means: a missing font falls
/// back to a system face, a missing sound just does not play.
pub fn asset_paths(kind: &str, name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(beside) = exe.parent() {
            paths.push(beside.join("assets").join(kind).join(name));
            // One level up as well, for `target/release/rungstar` run from the source tree,
            // and for a Windows zip whose executable sits beside an `assets` folder.
            if let Some(up) = beside.parent() {
                paths.push(up.join("assets").join(kind).join(name));
                // `<prefix>/share/rungstar/assets`, which is where a Linux package puts them
                // and where the Flatpak does. Without this the Flatpak started, borrowed
                // DejaVu and played nothing — it looked like a working build until `--check`
                // printed `0/6 sounds`, which is exactly what that line is for.
                paths.push(
                    up.join("share")
                        .join("rungstar")
                        .join("assets")
                        .join(kind)
                        .join(name),
                );
            }
        }
    }
    // The source tree, for a test and for `cargo run`.
    paths.push(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets")
            .join(kind)
            .join(name),
    );
    paths
}

/// The first of [`asset_paths`] that exists.
pub fn asset(kind: &str, name: &str) -> Option<PathBuf> {
    asset_paths(kind, name).into_iter().find(|p| p.exists())
}
