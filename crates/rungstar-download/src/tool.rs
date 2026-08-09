//! Getting hold of yt-dlp.
//!
//! It is not a build dependency and it is deliberately not vendored: YouTube changes its
//! extraction often enough that a copy frozen at release time is broken within weeks, which is
//! the whole reason this shells out to a separate tool rather than reimplementing it.
//!
//! So the game fetches it into its own data directory and keeps it up to date there. Three
//! rules make that acceptable rather than alarming:
//!
//! - **A copy already on the PATH wins.** Somebody who installed it with their package manager
//!   is telling us which one to use, and downloading a second is both rude and confusing.
//! - **It is never fetched silently.** The screen asks, because a program that downloads and
//!   runs an executable without saying so is doing something the person in front of it should
//!   have agreed to.
//! - **It is written whole or not at all**, through a temporary file and a rename, so a
//!   cancelled download cannot leave a truncated executable that fails in a way nobody can
//!   diagnose.

use std::path::{Path, PathBuf};

/// Where the releases are.
///
/// The `latest` redirect rather than a pinned version: a pinned yt-dlp is a yt-dlp that stops
/// working, and there is no version of it this code depends on the internals of.
pub const RELEASE_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

/// The asset for this platform.
pub fn asset() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else {
        // The standalone build, which needs no Python on the machine. The `_linux` asset is a
        // self-contained binary; plain `yt-dlp` is a zipapp that needs an interpreter, and a
        // Steam Deck in Game Mode may not have a usable one.
        "yt-dlp_linux"
    }
}

/// What the file is called once it is ours.
pub fn file_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

/// Where a fetched copy lives.
pub fn managed_path(data_dir: &Path) -> PathBuf {
    data_dir.join("tools").join(file_name())
}

/// Where yt-dlp is, if it is anywhere.
///
/// The PATH first. Somebody who installed it themselves gets theirs used, updated by their
/// package manager, and never shadowed by a copy this program fetched.
pub fn find(data_dir: &Path) -> Option<PathBuf> {
    if on_the_path() {
        return Some(PathBuf::from("yt-dlp"));
    }
    let managed = managed_path(data_dir);
    managed.is_file().then_some(managed)
}

/// Whether a working yt-dlp is on the PATH.
///
/// Run rather than looked for: a `yt-dlp` that is a broken symlink, or a wrapper script whose
/// interpreter is missing, is on the PATH and does not work, and finding that out at download
/// time is finding it out too late.
pub fn on_the_path() -> bool {
    std::process::Command::new("yt-dlp")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// What version a copy reports, for the screen to show.
pub fn version(program: &Path) -> Option<String> {
    let output = std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|version| !version.is_empty())
}

/// Why fetching it did not work.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("could not reach GitHub to fetch yt-dlp: {0}")]
    Fetch(String),
    #[error("what GitHub sent back was not a program ({0} bytes)")]
    NotAProgram(usize),
    #[error("could not write yt-dlp to {0}: {1}")]
    Write(String, String),
}

/// Install a fetched copy of yt-dlp into the data directory.
///
/// `bytes` is what the caller downloaded; this crate does no networking of its own, which is
/// what keeps it testable and what keeps the one place that talks to the network in the
/// application.
pub fn install(data_dir: &Path, bytes: &[u8]) -> Result<PathBuf, ToolError> {
    // A redirect to an error page is HTML and a few hundred bytes; yt-dlp is megabytes. Writing
    // an HTML file and marking it executable produces a failure nobody can read.
    if bytes.len() < 500_000 {
        return Err(ToolError::NotAProgram(bytes.len()));
    }
    let path = managed_path(data_dir);
    let folder = path.parent().unwrap_or(data_dir);
    std::fs::create_dir_all(folder)
        .map_err(|e| ToolError::Write(folder.display().to_string(), e.to_string()))?;

    // Whole or not at all: a cancelled write must not leave a truncated executable behind,
    // because that fails at the next download in a way that looks like a broken video.
    let temporary = path.with_extension("part");
    std::fs::write(&temporary, bytes)
        .map_err(|e| ToolError::Write(temporary.display().to_string(), e.to_string()))?;
    make_runnable(&temporary);
    std::fs::rename(&temporary, &path)
        .map_err(|e| ToolError::Write(path.display().to_string(), e.to_string()))?;
    Ok(path)
}

#[cfg(unix)]
fn make_runnable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn make_runnable(_path: &Path) {}

/// The URL to fetch for this platform.
pub fn download_url() -> String {
    format!("{RELEASE_URL}/{}", asset())
}
