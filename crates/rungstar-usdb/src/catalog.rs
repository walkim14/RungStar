//! The local copy of what USDB has.
//!
//! Kept because the alternative is a browser that cannot search until it has made three
//! hundred requests. With the catalog on disk the USDB browser opens instantly and works
//! offline, and a sync is a handful of requests rather than a crawl.
//!
//! **Incremental by `lastchange`.** USDB can order the list by last edit, newest first, so a
//! sync walks that order and stops at the first page whose songs are all older than the
//! newest edit already stored. A daily sync is one or two requests.

use std::collections::HashMap;

use crate::{CatalogSong, SongId};

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    /// Songs already held at the same edit time, which is how a sync knows to stop.
    pub unchanged: usize,
    /// Pages fetched, so a slow sync can be explained rather than just felt.
    pub pages: usize,
    /// Set when the crawl stopped because it reached songs it already had, rather than
    /// because it ran out of catalog.
    pub stopped_early: bool,
}

/// Why the catalog could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog storage: {0}")]
    Storage(String),
}

/// The stored catalog.
///
/// An in-memory map behind a trait-free struct: thirty thousand rows is a few megabytes, the
/// browser wants all of it sorted and filtered at once, and a second SQLite file beside the
/// library index buys nothing until the catalog outgrows memory. [`Catalog::load`] and
/// [`Catalog::save`] move it to and from one JSON file.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    songs: HashMap<SongId, CatalogSong>,
    /// The newest edit time seen, which is where the next sync stops.
    high_water: i64,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.songs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    pub fn high_water(&self) -> i64 {
        self.high_water
    }

    pub fn get(&self, id: SongId) -> Option<&CatalogSong> {
        self.songs.get(&id)
    }

    /// Every song, in a stable order: artist then title then id.
    pub fn all(&self) -> Vec<&CatalogSong> {
        let mut all: Vec<&CatalogSong> = self.songs.values().collect();
        all.sort_by(|a, b| {
            a.artist
                .to_lowercase()
                .cmp(&b.artist.to_lowercase())
                .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then(a.id.cmp(&b.id))
        });
        all
    }

    /// Songs whose artist or title contains every word typed, case- and accent-insensitively.
    ///
    /// Deliberately not the library's FTS5: the catalog has no lyrics to index and lives in
    /// memory, so a substring walk over thirty thousand rows is a fraction of a frame and
    /// needs no second index to keep in step with the first.
    pub fn search(&self, text: &str) -> Vec<&CatalogSong> {
        let words: Vec<String> = text.split_whitespace().map(fold).collect();
        let mut found: Vec<&CatalogSong> = self
            .all()
            .into_iter()
            .filter(|song| {
                if words.is_empty() {
                    return true;
                }
                let haystack = format!("{} {}", fold(&song.artist), fold(&song.title));
                words.iter().all(|word| haystack.contains(word.as_str()))
            })
            .collect();
        found.sort_by(|a, b| {
            b.rating
                .partial_cmp(&a.rating)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.artist.cmp(&b.artist))
        });
        found
    }

    /// Take in a page of the catalog. Returns whether anything was new.
    pub fn absorb(&mut self, page: &[CatalogSong], report: &mut SyncReport) -> bool {
        let mut fresh = false;
        for song in page {
            self.high_water = self.high_water.max(song.last_change);
            match self.songs.get(&song.id) {
                Some(held) if held.last_change >= song.last_change => report.unchanged += 1,
                Some(_) => {
                    report.updated += 1;
                    fresh = true;
                    self.songs.insert(song.id, song.clone());
                }
                None => {
                    report.added += 1;
                    fresh = true;
                    self.songs.insert(song.id, song.clone());
                }
            }
        }
        report.pages += 1;
        fresh
    }

    /// Whether a sync walking newest-first can stop after this page.
    ///
    /// It can once a whole page is older than what is already held: the order guarantees
    /// everything after it is older still. Requiring a *whole* page rather than one song
    /// matters — USDB's `lastchange` has one-second resolution and several songs edited in the
    /// same second can straddle a page boundary.
    pub fn caught_up(&self, page: &[CatalogSong], before: i64) -> bool {
        !page.is_empty() && page.iter().all(|song| song.last_change <= before)
    }

    /// Read a saved catalog.
    pub fn load(path: &std::path::Path) -> Result<Self, CatalogError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            // A missing file is a first run, not a failure.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => return Err(CatalogError::Storage(error.to_string())),
        };
        let songs: Vec<StoredSong> =
            serde_json::from_str(&text).map_err(|e| CatalogError::Storage(e.to_string()))?;
        let mut catalog = Self::new();
        for song in songs {
            let song = song.into_song();
            catalog.high_water = catalog.high_water.max(song.last_change);
            catalog.songs.insert(song.id, song);
        }
        Ok(catalog)
    }

    /// Write the catalog, atomically.
    ///
    /// Through a temporary file and a rename, because the alternative is a truncated catalog
    /// if the game is closed mid-write, and a truncated catalog reads as an empty one.
    pub fn save(&self, path: &std::path::Path) -> Result<(), CatalogError> {
        let stored: Vec<StoredSong> = self.all().into_iter().map(StoredSong::from_song).collect();
        let text =
            serde_json::to_string(&stored).map_err(|e| CatalogError::Storage(e.to_string()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CatalogError::Storage(e.to_string()))?;
        }
        let temporary = path.with_extension("json.part");
        std::fs::write(&temporary, text).map_err(|e| CatalogError::Storage(e.to_string()))?;
        std::fs::rename(&temporary, path).map_err(|e| CatalogError::Storage(e.to_string()))?;
        Ok(())
    }
}

/// Case- and accent-folded, so "bjork" finds "Björk".
///
/// The same idea as the library's ASCII shadow fields, done inline: the catalog is searched
/// from memory and has no index to fold at write time.
fn fold(text: &str) -> String {
    // Lower-cased first, as one string, then the accents stripped. The other way round is a
    // real bug and an easy one: "Ö" does not match the "ö" arm, so an upper-case accent
    // survives folding and "BJÖRK" stops finding Björk.
    let mut out = String::with_capacity(text.len());
    for c in text.to_lowercase().chars() {
        match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' | 'ē' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'ō' => out.push('o'),
            'ú' | 'ù' | 'û' | 'ü' | 'ū' => out.push('u'),
            'ñ' => out.push('n'),
            'ç' => out.push('c'),
            'ß' => out.push_str("ss"),
            // Anything else is kept, so a Cyrillic or CJK title is still searchable by
            // copying and pasting it.
            _ => out.push(c),
        }
    }
    out
}

/// The on-disk shape, kept separate so the in-memory struct can change without a migration.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredSong {
    id: i64,
    #[serde(rename = "c")]
    last_change: i64,
    #[serde(rename = "a")]
    artist: String,
    #[serde(rename = "t")]
    title: String,
    #[serde(default, rename = "g", skip_serializing_if = "String::is_empty")]
    genre: String,
    #[serde(default, rename = "y", skip_serializing_if = "Option::is_none")]
    year: Option<i32>,
    #[serde(default, rename = "e", skip_serializing_if = "String::is_empty")]
    edition: String,
    #[serde(default, rename = "l", skip_serializing_if = "String::is_empty")]
    language: String,
    #[serde(default, rename = "u", skip_serializing_if = "String::is_empty")]
    creator: String,
    #[serde(default, rename = "n", skip_serializing_if = "std::ops::Not::not")]
    golden_notes: bool,
    #[serde(default, rename = "r")]
    rating: f32,
    #[serde(default, rename = "v")]
    views: i64,
    #[serde(default, rename = "s", skip_serializing_if = "Option::is_none")]
    sample_url: Option<String>,
}

impl StoredSong {
    fn from_song(song: &CatalogSong) -> Self {
        Self {
            id: song.id.0,
            last_change: song.last_change,
            artist: song.artist.clone(),
            title: song.title.clone(),
            genre: song.genre.clone(),
            year: song.year,
            edition: song.edition.clone(),
            language: song.language.clone(),
            creator: song.creator.clone(),
            golden_notes: song.golden_notes,
            rating: song.rating,
            views: song.views,
            sample_url: song.sample_url.clone(),
        }
    }

    fn into_song(self) -> CatalogSong {
        CatalogSong {
            id: SongId(self.id),
            last_change: self.last_change,
            artist: self.artist,
            title: self.title,
            genre: self.genre,
            year: self.year,
            edition: self.edition,
            language: self.language,
            creator: self.creator,
            golden_notes: self.golden_notes,
            rating: self.rating,
            views: self.views,
            sample_url: self.sample_url,
        }
    }
}
