//! The usdb.animux.de protocol.
//!
//! USDB is a PHP site from 2008 with no API. Everything is `index.php` dispatched by a `link=`
//! query parameter, errors arrive as HTML bodies with a 200 status, and the page language
//! follows the account. Three consequences shape this crate:
//!
//! - **All of the markup knowledge is in [`parse`].** When the site changes, that is the file
//!   that breaks and the only one that has to be fixed. Everything above it works on structs.
//! - **Every response is checked before it is read.** A request for a song that does not exist
//!   comes back as a perfectly ordinary 200 whose body says so, and a client that trusts status
//!   codes reports an empty catalog instead of an error.
//! - **The transport is a trait.** [`Transport`] is what the network is behind, so the whole
//!   protocol is tested against saved pages with no network and no account.
//!
//! What is deliberately different from usdb_syncer, which is the reference: a full catalog
//! crawl there is about a thousand sequential POSTs with no rate limiting and no backoff. Here
//! every request goes through a [`rate::Limiter`], and a failed one is retried with an
//! exponential delay and jitter rather than hammering the site.

pub mod catalog;
pub mod client;
pub mod parse;
pub mod rate;
pub mod secret;
pub mod session_file;
pub mod strings;

pub use catalog::{Catalog, CatalogError, SyncReport};
pub use client::{Credentials, Endpoint, Session, Transport, Usdb};
pub use parse::{CatalogSong, SongDetails};
pub use strings::{Labels, Language};

/// A USDB song id.
///
/// A newtype because it is not a library id and the two are easy to confuse: a song can be in
/// the catalog with no local copy, on disk with no USDB id, or both.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct SongId(pub i64);

impl std::fmt::Display for SongId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for SongId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim().parse().map(SongId)
    }
}

/// Why a request failed.
#[derive(Debug, thiserror::Error)]
pub enum UsdbError {
    /// The page asked for a login. Not an HTTP 401 — USDB says so in the body of a 200.
    #[error("USDB wants a login for that")]
    NotLoggedIn,
    /// The song id does not exist. Also a 200, also said in the body.
    #[error("USDB has no song with that id")]
    NotFound,
    #[error("USDB rejected the username or password")]
    BadCredentials,
    #[error("could not reach USDB: {0}")]
    Transport(String),
    /// The markup was not what this build expects, which means the site has changed.
    ///
    /// Loud on purpose. The failure mode to avoid is a scraper that keeps working and returns
    /// nothing, because that is indistinguishable from an empty catalog.
    #[error("USDB returned something this build does not understand: {0}")]
    Unexpected(&'static str),
}
