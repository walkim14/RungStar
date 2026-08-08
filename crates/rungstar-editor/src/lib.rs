//! Editing a song.
//!
//! Everything here is a change to a [`SongTxt`] and nothing here draws, plays or reads a file
//! except to open and save one. That split is what makes an editor testable: a test applies
//! twenty edits, undoes them, and asserts the song is byte-for-byte what it started as.
//!
//! **Undo is by snapshot, not by inverse.** Every operation could carry an inverse — move a
//! note back, restore the text it had — and getting one of them subtly wrong is how an editor
//! silently corrupts somebody's work an hour into a session. A whole song is a few hundred
//! kilobytes; a hundred of them is less memory than one video frame. The obviously correct
//! thing is affordable here, so it is what is done.
//!
//! The other rule: **an edit never writes to disk.** Saving is explicit, goes through the same
//! writer as everything else, and is atomic. An editor that autosaves over the original is one
//! bad keystroke away from ruining a song somebody spent an evening timing.

/// The song model, re-exported so a screen can name a note kind without depending on the
/// parser crate directly.
pub use rungstar_song as song;

pub mod ops;
pub mod waveform;

use std::path::{Path, PathBuf};

use rungstar_song::{Line, Note, SongTxt};

pub use ops::{Op, Which};
pub use waveform::Waveform;

/// How many edits can be taken back.
///
/// Deep enough that an hour of work is recoverable, shallow enough that the memory is not
/// worth thinking about.
pub const UNDO_DEPTH: usize = 200;

/// Why an editor could not open or save.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("{0} could not be read: {1}")]
    Read(String, String),
    #[error("{0} is not a song this build can read: {1}")]
    Parse(String, String),
    #[error("{0} could not be written: {1}")]
    Write(String, String),
    /// The operation asked for something that is not there.
    #[error("nothing is selected")]
    NoSelection,
}

/// Which track and which notes are being worked on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    /// 0 for the first part, 1 for a duet's second.
    pub track: usize,
    /// The line the cursor is on.
    pub line: usize,
    /// The note the cursor is on, within that line.
    pub note: usize,
    /// How many notes are selected, counting from the cursor. Always at least one.
    pub run: usize,
}

impl Selection {
    pub fn new() -> Self {
        Self {
            track: 0,
            line: 0,
            note: 0,
            run: 1,
        }
    }

    /// The note indices selected within the line.
    pub fn notes(&self) -> std::ops::Range<usize> {
        self.note..self.note + self.run.max(1)
    }
}

/// A song open for editing.
#[derive(Debug, Clone)]
pub struct Editor {
    song: SongTxt,
    /// What was last saved, for knowing whether anything has changed.
    saved: SongTxt,
    path: PathBuf,
    undo: Vec<SongTxt>,
    redo: Vec<SongTxt>,
    pub selection: Selection,
    /// The last thing that could not be done, in words, so a screen can say it.
    pub refused: Option<String>,
}

impl Editor {
    /// Open a song file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EditError> {
        let path = path.as_ref().to_path_buf();
        let shown = path.display().to_string();
        let bytes =
            std::fs::read(&path).map_err(|e| EditError::Read(shown.clone(), e.to_string()))?;
        let parsed = SongTxt::parse_bytes(&bytes)
            .map_err(|e| EditError::Parse(shown.clone(), e.to_string()))?;
        Ok(Self::over(parsed.song, path))
    }

    /// Edit a song already in hand. Used by tests and by the sing screen's "fix this".
    pub fn over(song: SongTxt, path: PathBuf) -> Self {
        Self {
            saved: song.clone(),
            song,
            path,
            undo: Vec::new(),
            redo: Vec::new(),
            selection: Selection::new(),
            refused: None,
        }
    }

    pub fn song(&self) -> &SongTxt {
        &self.song
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether there are changes that have not been written.
    pub fn dirty(&self) -> bool {
        self.song != self.saved
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The lines of the track being edited.
    pub fn lines(&self) -> &[Line] {
        match self.selection.track {
            0 => &self.song.tracks.track_1,
            _ => self
                .song
                .tracks
                .track_2
                .as_deref()
                .unwrap_or(&self.song.tracks.track_1),
        }
    }

    /// How many tracks the song has.
    pub fn tracks(&self) -> usize {
        1 + usize::from(self.song.tracks.track_2.is_some())
    }

    /// The note under the cursor.
    pub fn current(&self) -> Option<&Note> {
        self.lines()
            .get(self.selection.line)?
            .notes
            .get(self.selection.note)
    }

    /// Apply an operation, remembering the song as it was.
    ///
    /// Returns whether anything changed. An operation that does nothing does not fill the undo
    /// stack with copies of the same song — otherwise holding a key that has stopped having an
    /// effect quietly throws away the history behind it.
    pub fn apply(&mut self, op: Op) -> bool {
        self.refused = None;
        let before = self.song.clone();
        let changed = ops::apply(self, op);
        if !changed || self.song == before {
            return false;
        }
        self.undo.push(before);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        // A new edit ends the redo branch, as in every editor: keeping it would mean a redo
        // that reaches a song nobody has ever seen.
        self.redo.clear();
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.song, previous));
        self.clamp();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.song, next));
        self.clamp();
        true
    }

    /// Write the song back, atomically.
    ///
    /// Through a temporary file and a rename, because a half-written song file is a song
    /// nobody can sing and the original is gone.
    pub fn save(&mut self) -> Result<(), EditError> {
        let shown = self.path.display().to_string();
        let text = self.song.to_string();
        let temporary = self.path.with_extension("txt.part");
        std::fs::write(&temporary, text.as_bytes())
            .map_err(|e| EditError::Write(shown.clone(), e.to_string()))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|e| EditError::Write(shown, e.to_string()))?;
        self.saved = self.song.clone();
        Ok(())
    }

    /// Put the cursor back inside the song after it changed shape.
    pub fn clamp(&mut self) {
        let tracks = self.tracks();
        self.selection.track = self.selection.track.min(tracks - 1);
        let lines = self.lines().len();
        if lines == 0 {
            self.selection.line = 0;
            self.selection.note = 0;
            self.selection.run = 1;
            return;
        }
        self.selection.line = self.selection.line.min(lines - 1);
        let notes = self.lines()[self.selection.line].notes.len();
        self.selection.note = self.selection.note.min(notes.saturating_sub(1));
        self.selection.run = self
            .selection
            .run
            .clamp(1, notes.saturating_sub(self.selection.note).max(1));
    }

    /// The song's own beat maths, for turning beats into seconds and back.
    pub fn bpm(&self) -> rungstar_song::Bpm {
        self.song.bpm()
    }

    /// When a beat happens, in seconds.
    pub fn seconds_at(&self, beat: f64) -> f64 {
        self.bpm().beat_to_time(beat, self.song.headers.gap as f64)
    }

    /// Which beat a moment in the audio falls on.
    pub fn beat_at(&self, seconds: f64) -> f64 {
        self.bpm()
            .time_to_beat(seconds, self.song.headers.gap as f64)
    }

    /// Direct access for the operations, which are the only things allowed to write.
    pub(crate) fn song_mut(&mut self) -> &mut SongTxt {
        &mut self.song
    }

    pub(crate) fn lines_mut(&mut self) -> &mut Vec<Line> {
        match self.selection.track {
            0 => &mut self.song.tracks.track_1,
            _ => self
                .song
                .tracks
                .track_2
                .as_mut()
                .unwrap_or(&mut self.song.tracks.track_1),
        }
    }
}
