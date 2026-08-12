//! Fetching songs from USDB.
//!
//! The pipeline is four things, in this order, and the order is the feature:
//!
//! 1. the note file, which names everything else;
//! 2. the audio, at which point **the song is singable and appears in the library**;
//! 3. the artwork;
//! 4. the video, which is ninety per cent of the bytes.
//!
//! The reference downloads all of it before a song is usable, so a 60 MB video stands between
//! you and a 4 MB song you wanted to sing now. Nothing here does: the folder is complete enough
//! to play as soon as step 2 lands, and the rest arrives behind it.
//!
//! Everything else follows from two rules. **Nothing is written where the library can see it
//! until it is whole** — files land in a temporary folder and are moved in one rename, so a
//! download killed halfway leaves no half-song for the scanner to index. And **every file is
//! remembered by its content hash**, not its timestamp, so a folder that has been through
//! cloud sync is not re-downloaded and a truncated file is caught.
//!
//! The parts that need a network are behind traits ([`Fetcher`] and [`Extractor`]), so the
//! whole pipeline is driven by tests with neither.

pub mod ffmpeg;
pub mod meta;
pub mod pipeline;
pub mod plan;
pub mod runtime;
pub mod tool;
pub mod ytdlp;

pub use meta::{Kind, Resource, SyncMeta};
pub use pipeline::{download, needs_repair, Fetcher, Outcome, Progress, Report, Stop};
pub use plan::{plan, Plan, Source, Step};
pub use tool::ToolError;
pub use ytdlp::{Extraction, Extractor, YtDlp};

/// Why a download failed.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("USDB: {0}")]
    Usdb(#[from] rungstar_usdb::UsdbError),
    #[error("{0} could not be fetched: {1}")]
    Fetch(String, String),
    #[error("the song file could not be read back: {0}")]
    BadSong(String),
    #[error("writing to {0}: {1}")]
    Io(String, String),
    /// The caller asked for it to stop. Not a failure, and reported separately.
    #[error("cancelled")]
    Cancelled,
}

/// A transport placeholder, so [`plan`] can name USDB's cover URL without a live session.
///
/// The URL is a pure function of the song id and needs nothing else; the type parameter on the
/// session is what makes this necessary rather than useful.
pub struct NoTransport;

impl rungstar_usdb::Transport for NoTransport {
    fn fetch(
        &self,
        _request: &rungstar_usdb::client::Request,
    ) -> Result<String, rungstar_usdb::UsdbError> {
        Err(rungstar_usdb::UsdbError::Transport(
            "this session cannot fetch".to_owned(),
        ))
    }
}
