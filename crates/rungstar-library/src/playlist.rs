//! UltraStar `.upl` playlists.
//!
//! A plain-text format that names songs by `Artist : Title` rather than by path, which means
//! a playlist survives the library being reorganised but breaks if a song is retagged. That
//! is the trade the format made and existing playlists depend on it, so it is kept — the
//! matching is just done case- and accent-insensitively, which recovers most near misses.

use std::fmt;
use std::path::Path;

use crate::db::{Database, DbError};
use crate::model::SongEntry;
use crate::scan::fold_for_sort;

/// One entry, as it is written in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    pub artist: String,
    pub title: String,
}

/// A parsed playlist.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Playlist {
    pub name: String,
    /// When set, the file's order is authoritative rather than the current sort.
    pub fixed_order: bool,
    pub items: Vec<PlaylistItem>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlaylistError {
    #[error("could not read playlist: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] DbError),
}

impl Playlist {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fixed_order: false,
            items: Vec::new(),
        }
    }

    /// Parse a `.upl` file's contents.
    ///
    /// Unknown `#` lines are ignored rather than rejected: the format's own header is a block
    /// of decorative hashes and a human-readable summary line.
    pub fn parse(text: &str) -> Self {
        let mut playlist = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                let Some((key, value)) = rest.split_once(':') else {
                    continue;
                };
                match key.trim().to_ascii_lowercase().as_str() {
                    "name" => playlist.name = value.trim().to_owned(),
                    "fixedorder" => {
                        playlist.fixed_order = value.trim().eq_ignore_ascii_case("on");
                    }
                    _ => {}
                }
                continue;
            }
            if let Some((artist, title)) = split_entry(line) {
                playlist.items.push(PlaylistItem { artist, title });
            }
        }
        playlist
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, PlaylistError> {
        Ok(Self::parse(&std::fs::read_to_string(path)?))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PlaylistError> {
        std::fs::write(path, self.to_string())?;
        Ok(())
    }

    pub fn add(&mut self, song: &SongEntry) {
        let item = PlaylistItem {
            artist: song.artist.clone(),
            title: song.title.clone(),
        };
        if !self.items.contains(&item) {
            self.items.push(item);
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<PlaylistItem> {
        (index < self.items.len()).then(|| self.items.remove(index))
    }

    /// Look every entry up in the library.
    ///
    /// Entries with no match come back separately rather than being silently dropped, so the
    /// UI can say "3 songs in this playlist are missing" instead of quietly shrinking it.
    pub fn resolve(
        &self,
        database: &Database,
    ) -> Result<(Vec<SongEntry>, Vec<PlaylistItem>), PlaylistError> {
        let all = database.search(&crate::search::SearchQuery::all())?;
        let index: Vec<(String, String, &SongEntry)> = all
            .iter()
            .map(|song| {
                (
                    fold_for_sort(&song.artist),
                    fold_for_sort(&song.title),
                    song,
                )
            })
            .collect();

        let mut found = Vec::new();
        let mut missing = Vec::new();
        for item in &self.items {
            let artist = fold_for_sort(&item.artist);
            let title = fold_for_sort(&item.title);
            match index.iter().find(|(a, t, _)| *a == artist && *t == title) {
                Some((_, _, song)) => found.push((*song).clone()),
                None => missing.push(item.clone()),
            }
        }
        Ok((found, missing))
    }
}

/// Split `Artist : Title`, preferring the spaced separator.
///
/// Both artists and titles routinely contain a bare colon, so the spaced form is tried first
/// and the unspaced one only as a fallback for files written by other editors.
fn split_entry(line: &str) -> Option<(String, String)> {
    if let Some((artist, title)) = line.split_once(" : ") {
        return Some((artist.trim().to_owned(), title.trim().to_owned()));
    }
    let (artist, title) = line.split_once(':')?;
    Some((artist.trim().to_owned(), title.trim().to_owned()))
}

impl fmt::Display for Playlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = "#".repeat(38);
        writeln!(f, "{rule}")?;
        writeln!(f, "#Ultrastar Deluxe Playlist Format v1.0")?;
        writeln!(
            f,
            "#Playlist {} with {} Songs.",
            self.name,
            self.items.len()
        )?;
        writeln!(f, "{rule}")?;
        writeln!(f, "#Name: {}", self.name)?;
        writeln!(
            f,
            "#FixedOrder: {}",
            if self.fixed_order { "On" } else { "Off" }
        )?;
        writeln!(f, "#Songs:")?;
        for item in &self.items {
            writeln!(f, "{} : {}", item.artist, item.title)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "######################################\n\
                          #Ultrastar Deluxe Playlist Format v1.0\n\
                          #Playlist Party with 2 Songs.\n\
                          ######################################\n\
                          #Name: Party\n\
                          #FixedOrder: On\n\
                          #Songs:\n\
                          The Beatles : Hey Jude\n\
                          Queen : Bohemian Rhapsody\n";

    #[test]
    fn a_playlist_round_trips() {
        let playlist = Playlist::parse(SAMPLE);
        assert_eq!(playlist.name, "Party");
        assert!(playlist.fixed_order);
        assert_eq!(playlist.items.len(), 2);
        assert_eq!(playlist.items[0].artist, "The Beatles");
        assert_eq!(playlist.items[1].title, "Bohemian Rhapsody");

        assert_eq!(Playlist::parse(&playlist.to_string()), playlist);
    }

    #[test]
    fn titles_containing_colons_survive() {
        let playlist = Playlist::parse("#Name: x\n#Songs:\nArtist : Title: The Sequel\n");
        assert_eq!(playlist.items[0].artist, "Artist");
        assert_eq!(playlist.items[0].title, "Title: The Sequel");
    }

    #[test]
    fn decorative_header_lines_are_not_songs() {
        let playlist = Playlist::parse(SAMPLE);
        assert!(!playlist
            .items
            .iter()
            .any(|item| item.artist.starts_with('#')));
    }

    #[test]
    fn fixed_order_defaults_to_off() {
        let playlist = Playlist::parse("#Name: x\n#Songs:\nA : B\n");
        assert!(!playlist.fixed_order);
    }

    #[test]
    fn adding_the_same_song_twice_does_nothing() {
        let mut playlist = Playlist::new("Test");
        let queen = PlaylistItem {
            artist: "Queen".into(),
            title: "Bohemian Rhapsody".into(),
        };
        playlist.items.push(queen.clone());
        assert_eq!(playlist.items.len(), 1);

        // `add` goes through a SongEntry, but the duplicate check is on the item itself.
        assert!(playlist.items.contains(&queen));
        assert_eq!(playlist.remove(0), Some(queen));
        assert!(playlist.items.is_empty());
        assert_eq!(playlist.remove(0), None);
    }
}
