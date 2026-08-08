//! The sidecar that remembers where a song came from.
//!
//! One `.usdb` JSON file per song folder, holding the USDB id, what was fetched, from which
//! URL, and the content hash of each file. Without it a library is a heap of folders and the
//! only way to know whether a song is up to date is to download it again.
//!
//! **Hashes rather than timestamps.** The reference matches on filename plus modification time
//! within two seconds plus source URL, which drifts the moment a folder goes through cloud
//! sync — every file looks changed and the whole library re-downloads. A blake3 of the content
//! says what the file *is*, survives being copied, and catches a truncated download that a
//! timestamp cannot see.

use std::path::{Path, PathBuf};

use rungstar_usdb::SongId;
use serde::{Deserialize, Serialize};

/// The sidecar's file name inside a song folder.
pub const SIDECAR: &str = ".rungstar-sync.json";

/// What kind of file a resource is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The note file. Everything else is optional; this is the song.
    Txt,
    Audio,
    Video,
    Cover,
    Background,
}

impl Kind {
    /// In the order the pipeline fetches them.
    ///
    /// Not arbitrary: the note file and the audio make a song singable, so they come first and
    /// the song appears in the library before its video has started. Waiting for a 60 MB video
    /// before a 4 MB song can be sung is the single worst thing about the reference.
    pub const ALL: [Kind; 5] = [
        Kind::Txt,
        Kind::Audio,
        Kind::Cover,
        Kind::Background,
        Kind::Video,
    ];

    /// Whether the song can be sung without this.
    pub fn optional(self) -> bool {
        !matches!(self, Kind::Txt | Kind::Audio)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Txt => "song",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Cover => "cover",
            Self::Background => "background",
        }
    }
}

/// One file that was fetched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub kind: Kind,
    /// The file's name inside the song folder.
    pub file: String,
    /// Where it came from, so a repair can fetch it again without another detail request.
    pub source: String,
    /// blake3 of the contents, hex.
    pub hash: String,
    pub bytes: u64,
}

/// Everything remembered about one downloaded song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMeta {
    /// Bumped when the shape below changes. An older build reads a newer file as "unknown"
    /// and re-downloads rather than misreading it.
    pub version: u32,
    pub usdb_id: SongId,
    /// USDB's own last-edit time when this was fetched, so a sync can tell it is stale.
    pub usdb_mtime: i64,
    /// Unix seconds when it was fetched.
    pub fetched_at: i64,
    pub resources: Vec<Resource>,
}

/// The version this build writes.
pub const VERSION: u32 = 1;

impl SyncMeta {
    pub fn new(usdb_id: SongId, usdb_mtime: i64, fetched_at: i64) -> Self {
        Self {
            version: VERSION,
            usdb_id,
            usdb_mtime,
            fetched_at,
            resources: Vec::new(),
        }
    }

    pub fn get(&self, kind: Kind) -> Option<&Resource> {
        self.resources.iter().find(|r| r.kind == kind)
    }

    /// Record a file, replacing any earlier one of the same kind.
    pub fn put(&mut self, resource: Resource) {
        self.resources.retain(|held| held.kind != resource.kind);
        self.resources.push(resource);
        self.resources.sort_by_key(|r| r.kind);
    }

    /// Whether the song can be sung with what has been fetched.
    pub fn playable(&self) -> bool {
        self.get(Kind::Txt).is_some() && self.get(Kind::Audio).is_some()
    }

    /// What is named but missing from the folder, or on disk with the wrong contents.
    ///
    /// This is what "repair library" runs on. Checking the hash rather than only existence is
    /// the point: a download interrupted halfway leaves a file that exists, opens, and plays
    /// four seconds of a song.
    pub fn broken(&self, folder: &Path) -> Vec<Kind> {
        self.resources
            .iter()
            .filter(|resource| {
                let path = folder.join(&resource.file);
                match std::fs::read(&path) {
                    Ok(bytes) => hash(&bytes) != resource.hash,
                    Err(_) => true,
                }
            })
            .map(|resource| resource.kind)
            .collect()
    }

    /// Whether USDB has a newer edit of this song than the one on disk.
    pub fn stale(&self, usdb_mtime: i64) -> bool {
        usdb_mtime > self.usdb_mtime
    }

    pub fn path(folder: &Path) -> PathBuf {
        folder.join(SIDECAR)
    }

    /// Read the sidecar from a song folder.
    ///
    /// A missing file is `None` rather than an error: most folders in a real library were put
    /// there by hand and have no sidecar at all.
    pub fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(folder)).ok()?;
        let meta: Self = serde_json::from_str(&text).ok()?;
        // A file from a later build is not readable, and guessing at it would report a song
        // as complete when this build cannot tell.
        (meta.version <= VERSION).then_some(meta)
    }

    /// Write the sidecar, atomically.
    pub fn write(&self, folder: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        let path = Self::path(folder);
        let temporary = path.with_extension("part");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, &path)
    }
}

/// The content hash used throughout: blake3, hex, lower case.
pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
