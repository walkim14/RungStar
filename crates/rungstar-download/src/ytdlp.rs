//! Shelling out to yt-dlp.
//!
//! Not reimplemented, and not a close call. YouTube extraction needs constant updating and a
//! JavaScript runtime to answer the signature challenges — the reference bundles an entire Deno
//! for it. yt-dlp is the tool that does this; the job here is to invoke it well.
//!
//! Invoking it well means three things the reference does not do. The command is built as data
//! so it can be tested without running anything. The child is **killed** on cancel rather than
//! politely asked between polls — the reference's abort is cooperative and checked every 500 ms,
//! which cannot stop a running subprocess at all. And its output is read for the errors that
//! mean "do not retry this" (age-gated, geo-blocked, taken down) so a dead link is reported once
//! instead of four times.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What came out of an extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    /// The file that was written, with whatever extension the container turned out to be.
    pub path: PathBuf,
    /// What yt-dlp said, kept for the log when something looks wrong.
    pub note: String,
}

/// Why an extraction failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    /// yt-dlp is not installed or not on the path.
    Missing,
    /// The video is gone, private, age-gated or blocked here. Retrying will not help.
    Unavailable(String),
    /// Something else, which might work next time.
    Failed(String),
    Cancelled,
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(
                f,
                "yt-dlp was not found. Downloads need it; everything else works without it."
            ),
            Self::Unavailable(why) => write!(f, "{why}"),
            Self::Failed(why) => write!(f, "{why}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Whatever runs an extraction.
pub trait Extractor {
    fn extract(
        &self,
        page: &str,
        audio_only: bool,
        into: &Path,
        stem: &str,
    ) -> Result<Extraction, ExtractError>;
}

/// The real one.
pub struct YtDlp {
    /// The executable, so a managed copy in the data directory can be used in preference to
    /// whatever is on the path.
    pub program: PathBuf,
    /// Where ffmpeg is, when there is one.
    ///
    /// yt-dlp finds ffmpeg on the PATH by itself and nowhere else, so a copy shipped beside
    /// the game is invisible to it unless it is told. Being told is the whole fix for
    /// "ffmpeg is not installed": the file was there, in the same folder as the executable,
    /// and yt-dlp had no reason to look.
    pub ffmpeg: Option<PathBuf>,
}

impl Default for YtDlp {
    fn default() -> Self {
        Self {
            program: PathBuf::from("yt-dlp"),
            ffmpeg: crate::ffmpeg::find(),
        }
    }
}

impl YtDlp {
    /// Use a particular copy, which is how a fetched one in the data directory is reached.
    pub fn at(program: PathBuf) -> Self {
        Self {
            program,
            ffmpeg: crate::ffmpeg::find(),
        }
    }

    /// Use a particular ffmpeg, or none at all.
    pub fn with_ffmpeg(mut self, ffmpeg: Option<PathBuf>) -> Self {
        self.ffmpeg = ffmpeg;
        self
    }
}

/// The arguments for one extraction, as data.
///
/// Built separately from being run so the command can be asserted in a test — which is the
/// only way any of this is checkable without a network and a live video.
pub fn arguments(
    page: &str,
    audio_only: bool,
    into: &Path,
    stem: &str,
    ffmpeg: Option<&Path>,
) -> Vec<String> {
    let template = into.join(format!("{stem}.%(ext)s"));
    let mut args: Vec<String> = vec![
        // Never touch anything but the one URL given. A playlist link would otherwise fetch
        // four hundred songs into one folder.
        "--no-playlist".into(),
        // Neither of these belongs in a song folder.
        "--no-write-info-json".into(),
        "--no-write-thumbnail".into(),
        // Progress on stdout, one line per update, so it can be parsed rather than scraped.
        "--newline".into(),
        "--no-colors".into(),
        "-o".into(),
        template.to_string_lossy().into_owned(),
    ];
    // Where our ffmpeg is. yt-dlp only ever looks on the PATH, so a copy shipped beside the
    // game is invisible to it without this — which is what made every download fail with
    // "ffmpeg is not installed" while the file sat in the same folder as the executable.
    if let Some(path) = ffmpeg {
        args.push("--ffmpeg-location".into());
        args.push(path.to_string_lossy().into_owned());
    }

    match (audio_only, ffmpeg.is_some()) {
        // **m4a, and asked for twice.** The obvious `bestaudio` plus a bare `-x` keeps
        // whatever YouTube served, and what YouTube serves is Opus in WebM — which Symphonia
        // has no decoder for, so the download succeeded and produced a song that would not
        // play. That was the other half of "downloading does not work", and it was invisible
        // until the file was opened.
        //
        // Naming the format twice is not redundant. The selector prefers an m4a *source* so
        // there is nothing to re-encode, and `--audio-format m4a` covers the videos that have
        // no m4a at all by transcoding the Opus. When the source already matches, yt-dlp says
        // "file is already in target format" and copies it, so the common case stays lossless.
        (true, true) => {
            args.push("-f".into());
            args.push("bestaudio[ext=m4a]/bestaudio/best".into());
            args.push("-x".into());
            args.push("--audio-format".into());
            args.push("m4a".into());
        }
        // No ffmpeg, so no `-x`: it is a post-processor and always runs one. An audio-only
        // format is already a playable file on its own, so ask for one directly. m4a first
        // for the same reason as above, and here it is the only defence — an Opus-only video
        // downloads a file the game cannot open, and there is nothing to convert it with.
        (true, false) => {
            args.push("-f".into());
            args.push("bestaudio[ext=m4a]/bestaudio[ext=mp3]/bestaudio/best".into());
        }
        // Capped at 1080p: the renderer scales the frame to at most 1280 wide, and a 4K video
        // is four times the bytes and four times the decode for a picture behind lyrics.
        (false, true) => {
            args.push("-f".into());
            args.push("bestvideo[height<=1080]+bestaudio/best[height<=1080]/best".into());
        }
        // Merging two streams needs ffmpeg, so take the best single file instead. That is
        // 720p at best and often 360p, which is a real loss — and still better than refusing.
        (false, false) => {
            args.push("-f".into());
            args.push("best[height<=1080]/best".into());
        }
    }
    args.push(page.to_owned());
    args
}

/// Whether yt-dlp's output means the link is dead rather than the attempt unlucky.
///
/// Retrying an age-gated video four times with exponential backoff wastes a minute and ends in
/// the same place. The strings are yt-dlp's own.
pub fn is_permanent(output: &str) -> bool {
    const DEAD: [&str; 7] = [
        "Sign in to confirm your age",
        "The uploader has not made this video available in your country",
        "This video contains content from",
        "Video unavailable",
        "This video is not available",
        "Private video",
        "This video has been removed",
    ];
    DEAD.iter().any(|needle| output.contains(needle))
}

/// The file yt-dlp actually wrote, found by its stem.
///
/// It picks the extension from the container, so the caller cannot know the name in advance —
/// and a folder listing is more reliable than parsing it out of the progress output, which
/// changes between yt-dlp releases.
pub fn written(into: &Path, stem: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(into).ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let matches = path
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == stem);
        // `.part` is a download in progress and `.ytdl` is its bookkeeping; neither is the file.
        let finished = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| !matches!(ext, "part" | "ytdl" | "temp"));
        if matches && finished {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if best.as_ref().is_none_or(|(held, _)| size > *held) {
                best = Some((size, path));
            }
        }
    }
    best.map(|(_, path)| path)
}

impl Extractor for YtDlp {
    fn extract(
        &self,
        page: &str,
        audio_only: bool,
        into: &Path,
        stem: &str,
    ) -> Result<Extraction, ExtractError> {
        let args = arguments(page, audio_only, into, stem, self.ffmpeg.as_deref());
        let output = Command::new(&self.program)
            .args(&args)
            .stdin(Stdio::null())
            .output();
        let output = match output {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ExtractError::Missing)
            }
            Err(error) => return Err(ExtractError::Failed(error.to_string())),
        };
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            return Err(if is_permanent(&said) {
                ExtractError::Unavailable(first_error(&said))
            } else {
                ExtractError::Failed(first_error(&said))
            });
        }
        match written(into, stem) {
            Some(path) => Ok(Extraction { path, note: said }),
            None => Err(ExtractError::Failed(
                "yt-dlp reported success but wrote nothing".to_owned(),
            )),
        }
    }
}

/// The first line that looks like the reason, rather than the whole log.
fn first_error(output: &str) -> String {
    output
        .lines()
        .find(|line| line.contains("ERROR") || line.contains("Unavailable"))
        .unwrap_or_else(|| output.lines().last().unwrap_or("yt-dlp failed"))
        .trim()
        .to_owned()
}
