//! Singing to the backing track.
//!
//! A vocal-removal pass — Demucs and friends — turns a library into a second library: the same
//! songs, the same folder names, one audio file each and nothing else. Everything that makes a
//! song a song is still in the original folder, so this is not a second library to index. It is
//! one substitution, made at the moment the audio file is opened: the notes, the lyrics, the
//! video, the cover and the highscores all come from where they always came from, and only the
//! sound is different.
//!
//! That is the whole design. Indexing the instrumentals as songs of their own would double the
//! library, split every song's play count and highscores across two rows, and make "the same
//! song" a thing the browser has to work out. Repointing one path does none of that.
//!
//! **Matched by the song's own folder name**, which is what a separation tool writing one
//! folder per song produces. Not by artist and title — those live in a header the instrumental
//! folder does not have — and not by audio file name, because the tool renames what it writes.
//!
//! **One directory listing, at the root.** Eight thousand folders is a single `read_dir` and a
//! few milliseconds; a `stat` per song while browsing would be eight thousand of them, on
//! Windows through the filter driver stack, on every scroll.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Extensions worth opening. Ordered by what a separation tool actually writes, so the common
/// case matches on the first try.
const AUDIO: [&str; 7] = ["ogg", "opus", "mp3", "m4a", "flac", "wav", "aac"];

/// The instrumental version of each song, by folder name.
#[derive(Debug, Default)]
pub struct Instrumentals {
    root: Option<PathBuf>,
    /// Folded folder name to the folder itself. Folded because a header, a file system and a
    /// separation tool disagree about case, and on Windows the folder opens anyway.
    folders: HashMap<String, PathBuf>,
}

impl Instrumentals {
    /// Read the folder names under `root`, or nothing when none is configured.
    ///
    /// A missing or unreadable folder is not an error worth reporting from here: it means the
    /// same thing as an empty one — no song has an instrumental — and the screen that offers
    /// the mode is the place that says so.
    pub fn load(root: Option<&str>) -> Self {
        let Some(root) = root.map(PathBuf::from).filter(|path| path.is_dir()) else {
            return Self::default();
        };
        let mut folders = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.filter_map(Result::ok) {
                // Only directories: a stray file at the root is not a song.
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                folders.insert(fold(&name), entry.path());
            }
        }
        Self {
            root: Some(root),
            folders,
        }
    }

    /// The configured folder, whether or not anything was found in it.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// How many songs have one.
    pub fn len(&self) -> usize {
        self.folders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }

    /// Whether this song has an instrumental folder.
    ///
    /// Answers from the listing taken at load, so it costs nothing to ask for every row of a
    /// list. It says a folder exists, not that a file in it decodes — that is
    /// [`Self::audio_for`], and it is asked once, when a song is about to play.
    pub fn has(&self, song: &Path) -> bool {
        self.folder_for(song).is_some()
    }

    /// The instrumental audio file for a song, if there is one.
    ///
    /// `original` is the song's own audio file name, which is preferred when the separation
    /// tool kept it. Otherwise the first audio file in the folder, because a folder holding one
    /// track is what these tools write and guessing at names beyond that is guessing.
    pub fn audio_for(&self, song: &Path, original: Option<&str>) -> Option<PathBuf> {
        let folder = self.folder_for(song)?;
        let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| AUDIO.iter().any(|known| e.eq_ignore_ascii_case(known)))
            })
            .collect();
        // Stable, so a folder that somehow holds two tracks does not alternate between them
        // from one launch to the next.
        files.sort();
        let named: Option<String> = original
            .and_then(|name| Path::new(name).file_stem())
            .and_then(|stem| stem.to_str())
            .map(fold);
        files
            .iter()
            .find(|path| named.is_some() && stem(path) == named)
            .or_else(|| files.first())
            .cloned()
    }

    fn folder_for(&self, song: &Path) -> Option<&PathBuf> {
        let name = song.parent()?.file_name()?.to_string_lossy().into_owned();
        self.folders.get(&fold(&name))
    }
}

fn stem(path: &Path) -> Option<String> {
    Some(fold(path.file_stem()?.to_str()?))
}

/// Case-folded, so a folder written by one tool matches a folder written by another.
fn fold(name: &str) -> String {
    name.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A root holding `folder/file` for each pair given.
    fn tree(pairs: &[(&str, &[&str])]) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("temp dir");
        for (folder, files) in pairs {
            let directory = temp.path().join(folder);
            std::fs::create_dir_all(&directory).expect("create");
            for file in *files {
                std::fs::write(directory.join(file), b"audio").expect("write");
            }
        }
        temp
    }

    /// The `.txt` of a song in a folder of that name, as the library would have indexed it.
    fn song(folder: &str) -> PathBuf {
        PathBuf::from("C:/songs").join(folder).join("whatever.txt")
    }

    #[test]
    fn nothing_is_held_without_a_folder() {
        let instrumentals = Instrumentals::load(None);
        assert!(instrumentals.is_empty());
        assert!(!instrumentals.has(&song("Aqua - Barbie Girl")));
        assert_eq!(
            instrumentals.audio_for(&song("Aqua - Barbie Girl"), None),
            None
        );
    }

    #[test]
    fn a_folder_that_is_not_there_is_the_same_as_an_empty_one() {
        let missing = std::env::temp_dir().join("rungstar-no-instrumentals-4c1a");
        let instrumentals = Instrumentals::load(missing.to_str());
        assert!(instrumentals.is_empty());
    }

    #[test]
    fn a_song_is_matched_by_its_own_folder_name() {
        let temp = tree(&[("Aqua - Barbie Girl", &["Aqua - Barbie Girl.ogg"])]);
        let instrumentals = Instrumentals::load(temp.path().to_str());
        assert_eq!(instrumentals.len(), 1);
        assert!(instrumentals.has(&song("Aqua - Barbie Girl")));
        // A different song in the same library, with no instrumental of its own.
        assert!(!instrumentals.has(&song("Blur - Song 2")));
        let found = instrumentals
            .audio_for(&song("Aqua - Barbie Girl"), Some("Aqua - Barbie Girl.ogg"))
            .expect("the one file in the folder");
        assert_eq!(found.file_name().unwrap(), "Aqua - Barbie Girl.ogg");
    }

    #[test]
    fn the_case_of_the_folder_does_not_have_to_agree() {
        let temp = tree(&[("AQUA - BARBIE GIRL", &["out.ogg"])]);
        let instrumentals = Instrumentals::load(temp.path().to_str());
        assert!(instrumentals.has(&song("Aqua - Barbie Girl")));
    }

    #[test]
    fn the_separation_tools_own_name_for_the_file_is_taken() {
        // What Demucs and friends actually write: one track, named for the stem it kept.
        let temp = tree(&[("Blur - Song 2", &["no_vocals.ogg"])]);
        let instrumentals = Instrumentals::load(temp.path().to_str());
        let found = instrumentals
            .audio_for(&song("Blur - Song 2"), Some("Blur - Song 2.ogg"))
            .expect("the only audio file");
        assert_eq!(found.file_name().unwrap(), "no_vocals.ogg");
    }

    #[test]
    fn the_songs_own_audio_name_wins_when_the_folder_holds_more_than_one() {
        let temp = tree(&[("Blur - Song 2", &["accompaniment.ogg", "Blur - Song 2.ogg"])]);
        let instrumentals = Instrumentals::load(temp.path().to_str());
        let found = instrumentals
            .audio_for(&song("Blur - Song 2"), Some("Blur - Song 2.mp3"))
            .expect("matched on the stem, not the extension");
        assert_eq!(found.file_name().unwrap(), "Blur - Song 2.ogg");
    }

    #[test]
    fn a_folder_with_nothing_playable_in_it_has_no_audio() {
        // The folder exists, so the browser lists the song; the file it needs does not, so
        // starting it fails rather than quietly playing the version with the singing on it.
        let temp = tree(&[("Blur - Song 2", &["notes.txt"])]);
        let instrumentals = Instrumentals::load(temp.path().to_str());
        assert!(instrumentals.has(&song("Blur - Song 2")));
        assert_eq!(instrumentals.audio_for(&song("Blur - Song 2"), None), None);
    }
}
