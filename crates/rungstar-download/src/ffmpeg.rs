//! Finding ffmpeg, which yt-dlp needs and this crate does not run itself.
//!
//! **This is what "ffmpeg is not installed" was.** yt-dlp shells out to ffmpeg for two of the
//! three things asked of it: `-x` to pull the audio out of a container, and merging the
//! separate video and audio streams YouTube serves above 360p. Neither is optional to yt-dlp,
//! so without ffmpeg on the machine every download fails at the last step, after the bytes have
//! already been fetched.
//!
//! The game already links FFmpeg 7.1 for song video, but linking a library is not the same as
//! having a program: yt-dlp runs `ffmpeg` as a child process. A packaged Windows build
//! therefore ships `ffmpeg.exe` from the same build the DLLs come from, and this finds it.
//!
//! When there is none — a Linux build with no system ffmpeg, a source tree with no vendor
//! directory — downloading still works, at lower quality, because
//! [`crate::ytdlp::arguments`] switches to formats that need no post-processing. A worse video
//! is better than an error message.

use std::path::{Path, PathBuf};

/// What the program is called here.
pub fn file_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Where ffmpeg is, if it is anywhere.
///
/// In order:
///
/// 1. **The PATH**, so somebody who installed it themselves gets theirs. On Linux that is the
///    normal case and the only one worth optimising for.
/// 2. **Beside the executable**, which is where a packaged Windows build puts it, next to the
///    DLLs it shares with the game.
/// 3. **`vendor/ffmpeg/bin`** in the source tree, so `cargo run` downloads a song without
///    anything being installed first.
pub fn find() -> Option<PathBuf> {
    if on_the_path() {
        return Some(PathBuf::from("ffmpeg"));
    }
    beside_the_executable().or_else(vendored)
}

/// A copy shipped next to the game.
pub fn beside_the_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?;
    [
        folder.join(file_name()),
        folder.join("tools").join(file_name()),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

/// The copy in the source tree, for a build run out of `target/`.
fn vendored() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // `target/<profile>/rungstar.exe` — the workspace root is two levels up.
    exe.ancestors()
        .nth(3)
        .map(|root| {
            root.join("vendor")
                .join("ffmpeg")
                .join("bin")
                .join(file_name())
        })
        .filter(|candidate| candidate.is_file())
}

/// Whether a working ffmpeg is on the PATH.
///
/// Run rather than looked for, for the same reason as yt-dlp: a broken symlink or a wrapper
/// whose interpreter is gone is on the PATH and does not work, and finding that out when a
/// download fails is finding it out too late.
pub fn on_the_path() -> bool {
    runs(Path::new("ffmpeg"))
}

/// Whether this path is an ffmpeg that starts.
pub fn runs(program: &Path) -> bool {
    std::process::Command::new(program)
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The version line, for the downloads screen to show.
pub fn version(program: &Path) -> Option<String> {
    let output = std::process::Command::new(program)
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // "ffmpeg version n7.1.5-12-g1fdbca85aa-20260808 Copyright (c) ..." — the middle word is
    // the only part worth showing, and the rest is a configure line hundreds of characters long.
    text.lines()
        .next()?
        .split_whitespace()
        .nth(2)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_named_for_the_platform() {
        assert_eq!(
            file_name(),
            if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            }
        );
    }

    #[test]
    fn nothing_is_found_where_nothing_is() {
        assert!(!runs(Path::new("definitely-not-ffmpeg-9d3f")));
        assert!(version(Path::new("definitely-not-ffmpeg-9d3f")).is_none());
    }
}
