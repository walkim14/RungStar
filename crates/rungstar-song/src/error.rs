//! Errors and warnings produced while reading a song file.

use std::fmt;

/// A fatal problem that prevents a song from being loaded at all.
///
/// Anything recoverable is reported as a [`Warning`] instead — real-world song files are
/// full of junk lines and half-broken notes, and refusing to load them would mean refusing
/// a large slice of any existing library.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// One or more of `#TITLE`, `#ARTIST`, `#BPM` was missing.
    #[error("missing required header(s): {0}")]
    MissingRequiredHeaders(MissingHeaders),

    /// The note section was empty, so there is nothing to sing.
    #[error("song contains no singable notes")]
    NoNotes,

    /// `#VERSION` declared a format this build does not understand.
    #[error("unsupported format version {0}")]
    UnsupportedVersion(String),
}

/// Which of the mandatory headers were absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MissingHeaders {
    pub title: bool,
    pub artist: bool,
    pub bpm: bool,
}

impl MissingHeaders {
    pub fn any(self) -> bool {
        self.title || self.artist || self.bpm
    }
}

impl fmt::Display for MissingHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (missing, name) in [
            (self.title, "#TITLE"),
            (self.artist, "#ARTIST"),
            (self.bpm, "#BPM"),
        ] {
            if missing {
                if !first {
                    f.write_str(", ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

/// A recoverable problem: the offending line was skipped or repaired, and parsing continued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning(pub String);

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Collector for [`Warning`]s raised during a parse or a fix pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Warnings(Vec<Warning>);

impl Warnings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.0.push(Warning(message.into()));
    }

    pub fn as_slice(&self) -> &[Warning] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn into_vec(self) -> Vec<Warning> {
        self.0
    }
}

impl<'a> IntoIterator for &'a Warnings {
    type Item = &'a Warning;
    type IntoIter = std::slice::Iter<'a, Warning>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
