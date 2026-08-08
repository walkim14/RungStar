//! Running a plan.
//!
//! Everything lands in a temporary folder first and moves into the library in one rename. A
//! download killed halfway therefore leaves nothing behind but a temporary folder, rather than
//! a half-song the scanner indexes and somebody tries to sing.
//!
//! The one exception is deliberate and is the whole point of play-now: once the note file and
//! the audio are in, the folder is moved into place *before* the video is fetched, and the
//! video is moved in beside it when it finishes. A song you can sing in ten seconds beats a
//! complete one in three minutes.

use std::path::{Path, PathBuf};

use rungstar_usdb::SongId;

use crate::meta::{hash, Kind, Resource, SyncMeta};
use crate::plan::{Plan, Source, Step};
use crate::ytdlp::{ExtractError, Extractor};
use crate::DownloadError;

/// Whatever fetches a plain URL.
pub trait Fetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// A cancel flag. Checked between steps and passed to the extractor.
pub trait Stop {
    fn stopped(&self) -> bool;
}

impl Stop for std::sync::atomic::AtomicBool {
    fn stopped(&self) -> bool {
        self.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Nothing ever cancels.
pub struct RunToEnd;

impl Stop for RunToEnd {
    fn stopped(&self) -> bool {
        false
    }
}

/// What the pipeline reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Started(Kind),
    Finished(Kind),
    /// The song is now singable and has been moved into the library.
    Playable(PathBuf),
    /// An optional resource could not be fetched. Not fatal.
    Missed(Kind, String),
}

/// How a download ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Everything asked for arrived.
    Complete,
    /// The song is singable; something optional is missing.
    Partial,
    Cancelled,
}

/// What a download did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    pub folder: PathBuf,
    pub fetched: Vec<Kind>,
    pub skipped: Vec<Kind>,
    pub missing: Vec<(Kind, String)>,
    pub meta: SyncMeta,
}

/// Run a plan.
///
/// `into` is the song root the folder is created under. `scratch` is where files are built
/// before they are moved — on the same volume as `into` when possible, because a rename across
/// volumes is a copy and stops being atomic.
#[allow(clippy::too_many_arguments)]
pub fn download(
    plan: &Plan,
    into: &Path,
    scratch: &Path,
    usdb_mtime: i64,
    now: i64,
    fetcher: &dyn Fetcher,
    extractor: &dyn Extractor,
    stop: &dyn Stop,
    mut report_progress: impl FnMut(Progress),
) -> Result<Report, DownloadError> {
    let working = scratch.join(format!("{}.part", plan.folder));
    let _ = std::fs::remove_dir_all(&working);
    std::fs::create_dir_all(&working)
        .map_err(|e| DownloadError::Io(working.display().to_string(), e.to_string()))?;

    let destination = into.join(&plan.folder);
    let mut meta = SyncMeta::read(&destination)
        .filter(|held| held.usdb_id == plan.usdb_id)
        .unwrap_or_else(|| SyncMeta::new(plan.usdb_id, usdb_mtime, now));
    meta.usdb_mtime = usdb_mtime;
    meta.fetched_at = now;

    let mut fetched = Vec::new();
    let mut missing: Vec<(Kind, String)> = Vec::new();
    let mut moved_in = false;

    for step in &plan.steps {
        if stop.stopped() {
            let _ = std::fs::remove_dir_all(&working);
            return Ok(Report {
                outcome: Outcome::Cancelled,
                folder: destination,
                fetched,
                skipped: plan.skipped.clone(),
                missing,
                meta,
            });
        }
        report_progress(Progress::Started(step.kind));
        let where_to = if moved_in { &destination } else { &working };
        match run_step(step, where_to, fetcher, extractor) {
            Ok(resource) => {
                meta.put(resource);
                fetched.push(step.kind);
                report_progress(Progress::Finished(step.kind));
            }
            Err(why) => {
                // An optional resource that will not come is noted and the song still lands.
                // Refusing to deliver a singable song because its background art 404ed is the
                // reference's behaviour and it is wrong.
                if step.kind.optional() {
                    missing.push((step.kind, why.clone()));
                    report_progress(Progress::Missed(step.kind, why));
                } else {
                    let _ = std::fs::remove_dir_all(&working);
                    return Err(DownloadError::Fetch(step.kind.label().to_owned(), why));
                }
            }
        }

        // The moment it is singable, move it in. Everything after this writes straight into
        // the library folder, which is safe because the song is already whole.
        if !moved_in && meta.playable() {
            let _ = meta.write(&working);
            move_in(&working, &destination)?;
            moved_in = true;
            report_progress(Progress::Playable(destination.clone()));
        }
    }

    if !moved_in {
        // Nothing playable came of it. A folder with a cover and no song is not a song.
        let _ = std::fs::remove_dir_all(&working);
        return Err(DownloadError::Fetch(
            "song".to_owned(),
            "nothing playable was fetched".to_owned(),
        ));
    }
    meta.write(&destination)
        .map_err(|e| DownloadError::Io(destination.display().to_string(), e.to_string()))?;

    Ok(Report {
        outcome: if missing.is_empty() {
            Outcome::Complete
        } else {
            Outcome::Partial
        },
        folder: destination,
        fetched,
        skipped: plan.skipped.clone(),
        missing,
        meta,
    })
}

fn run_step(
    step: &Step,
    into: &Path,
    fetcher: &dyn Fetcher,
    extractor: &dyn Extractor,
) -> Result<Resource, String> {
    match &step.source {
        Source::Text(text) => {
            let file = format!("{}.txt", step.stem);
            let bytes = text.as_bytes();
            std::fs::write(into.join(&file), bytes).map_err(|e| e.to_string())?;
            Ok(Resource {
                kind: step.kind,
                file,
                source: "usdb".to_owned(),
                hash: hash(bytes),
                bytes: bytes.len() as u64,
            })
        }
        Source::Url(url) => {
            let bytes = fetcher.get(url)?;
            if bytes.is_empty() {
                return Err("the server sent an empty file".to_owned());
            }
            let file = format!("{}{}", step.stem, extension_for(step.kind, url, &bytes));
            std::fs::write(into.join(&file), &bytes).map_err(|e| e.to_string())?;
            Ok(Resource {
                kind: step.kind,
                file,
                source: url.clone(),
                hash: hash(&bytes),
                bytes: bytes.len() as u64,
            })
        }
        Source::Extract { page, audio_only } => {
            let stem = match step.kind {
                // The audio and the video come from the same page and would otherwise be
                // written over each other.
                Kind::Audio => step.stem.clone(),
                _ => format!("{} [video]", step.stem),
            };
            let extraction =
                extractor
                    .extract(page, *audio_only, into, &stem)
                    .map_err(|e| match e {
                        ExtractError::Missing => "yt-dlp is not installed".to_owned(),
                        other => other.to_string(),
                    })?;
            let bytes = std::fs::read(&extraction.path).map_err(|e| e.to_string())?;
            let file = extraction
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .ok_or_else(|| "the extracted file has no name".to_owned())?;
            Ok(Resource {
                kind: step.kind,
                file,
                source: page.clone(),
                hash: hash(&bytes),
                bytes: bytes.len() as u64,
            })
        }
    }
}

/// The extension for a downloaded file.
///
/// From the bytes first, because a URL's extension is a suggestion — half the covers on
/// fanart.tv are served from a path ending in `.jpg` and are PNGs.
fn extension_for(kind: Kind, url: &str, bytes: &[u8]) -> String {
    if let Some(sniffed) = sniff(bytes) {
        return format!(".{sniffed}");
    }
    let from_url = url
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.split(['?', '#']).next().unwrap_or(ext))
        .filter(|ext| ext.len() <= 4 && ext.chars().all(char::is_alphanumeric));
    match from_url {
        Some(ext) => format!(".{}", ext.to_lowercase()),
        None if kind == Kind::Txt => ".txt".to_owned(),
        None => ".bin".to_owned(),
    }
}

/// The image formats worth recognising, by their magic bytes.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if bytes.starts_with(b"GIF8") {
        return Some("gif");
    }
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    None
}

/// Move a finished folder into the library.
///
/// One rename when the destination is new. When it is not — a repair, or a re-download of a
/// song that changed — the files are moved in one at a time over what is there, because
/// replacing the folder wholesale would delete anything the player had put in it.
fn move_in(working: &Path, destination: &Path) -> Result<(), DownloadError> {
    let io = |path: &Path, e: std::io::Error| {
        DownloadError::Io(path.display().to_string(), e.to_string())
    };
    if !destination.exists() {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        match std::fs::rename(working, destination) {
            Ok(()) => return Ok(()),
            // Across volumes a rename is not possible; fall through to the per-file path.
            Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {}
            Err(error) if error.raw_os_error() == Some(17) => {}
            Err(error) => return Err(io(destination, error)),
        }
    }
    std::fs::create_dir_all(destination).map_err(|e| io(destination, e))?;
    for entry in std::fs::read_dir(working)
        .map_err(|e| io(working, e))?
        .flatten()
    {
        let to = destination.join(entry.file_name());
        if std::fs::rename(entry.path(), &to).is_err() {
            std::fs::copy(entry.path(), &to).map_err(|e| io(&to, e))?;
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = std::fs::remove_dir_all(working);
    Ok(())
}

/// Songs in a library whose sidecar says something is missing or broken.
///
/// This is "repair library": every folder that was downloaded, checked against what its
/// sidecar says should be there. Folders with no sidecar are somebody's own songs and are left
/// alone — repairing a song nobody downloaded means deciding what it should have been.
pub fn needs_repair(root: &Path) -> Vec<(PathBuf, SongId, Vec<Kind>)> {
    let mut broken = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return broken;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        let Some(meta) = SyncMeta::read(&folder) else {
            continue;
        };
        let wrong = meta.broken(&folder);
        if !wrong.is_empty() {
            broken.push((folder, meta.usdb_id, wrong));
        }
    }
    broken.sort_by(|a, b| a.0.cmp(&b.0));
    broken
}
