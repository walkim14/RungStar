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
            // One level up as well, for `target/release/rungstar` run from the source tree.
            if let Some(up) = beside.parent() {
                paths.push(up.join("assets").join(kind).join(name));
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
