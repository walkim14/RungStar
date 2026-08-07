//! The UltraStar `.txt` song format.
//!
//! This crate reads, writes and normalises the plain-text files that describe an UltraStar
//! song: the `#TAG:value` header block, followed by a stream of timed syllables.
//!
//! Two properties drive the design:
//!
//! * **Real files are messy.** Two decades of editors, hand-editing and half-finished
//!   transcriptions mean stray lines, malformed notes and inconsistent encodings are normal.
//!   Anything recoverable is repaired or skipped with a [`Warning`], and only a genuinely
//!   unusable file produces a [`ParseError`].
//! * **Timing must be exact.** Scoring compares sung pitch against note beats, so the beat
//!   arithmetic in [`Bpm`] is the reference for the whole engine. Note that `#BPM` is a
//!   quarter of the real grid rate — see [`Bpm::grid_rate`].
//!
//! ```
//! use rungstar_song::SongTxt;
//!
//! let song = SongTxt::parse_str(
//!     "#TITLE:Example\n#ARTIST:Nobody\n#BPM:120\n#GAP:500\n: 0 4 0 Hel\n: 4 4 2 lo\nE\n",
//! )
//! .unwrap();
//!
//! assert_eq!(song.headers.title, "Example");
//! assert_eq!(song.tracks.track_1.len(), 1);
//! // The grid runs at 4x the file's BPM, so beat 4 is half a second in, plus the 500ms gap.
//! assert!((song.beat_to_time(4.0) - 1.0).abs() < 1e-9);
//! ```

pub mod bpm;
pub mod encoding;
pub mod error;
pub mod fix;
pub mod headers;
pub mod lang;
pub mod meta_tags;
pub mod note;

pub use bpm::Bpm;
pub use encoding::Encoding;
pub use error::{ParseError, Warning, Warnings};
pub use fix::{FixOptions, LinebreakStyle, SpaceStyle};
pub use headers::Headers;
pub use meta_tags::MetaTags;
pub use note::{Line, LineBreak, Note, NoteKind, Tracks};

use encoding::split_lines;
use note::LineCursor;

/// A parsed song file.
#[derive(Debug, Clone, PartialEq)]
pub struct SongTxt {
    pub headers: Headers,
    pub tracks: Tracks,
    /// Resource hints decoded from the `#VIDEO` header.
    pub meta_tags: MetaTags,
    /// The encoding the file was read as, so it can be written back the same way.
    pub encoding: Encoding,
}

/// A parse result together with everything that had to be repaired or skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub song: SongTxt,
    pub warnings: Warnings,
}

impl SongTxt {
    /// Parse already-decoded text, discarding warnings.
    pub fn parse_str(text: &str) -> Result<Self, ParseError> {
        Self::parse_str_verbose(text).map(|p| p.song)
    }

    /// Parse already-decoded text, keeping the warnings.
    pub fn parse_str_verbose(text: &str) -> Result<Parsed, ParseError> {
        let mut warnings = Warnings::new();
        let song = Self::parse_lines(text, Encoding::Utf8, &mut warnings)?;
        Ok(Parsed { song, warnings })
    }

    /// Parse raw file bytes, working out the encoding first.
    ///
    /// A byte-order mark wins outright. Otherwise the text is sniffed, and if the resulting
    /// headers declare a different `#ENCODING`, the bytes are decoded again with it — the
    /// header cannot be read before the text is decoded, so one speculative pass is
    /// unavoidable.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Parsed, ParseError> {
        let first = encoding::decode(bytes, Encoding::Auto);
        let declared = scan_declared_encoding(&first.text);

        let decoded = match declared {
            // A BOM already settled it, or the file agrees with the sniff.
            Some(enc) if first.encoding != Encoding::Utf8Bom && enc != first.encoding => {
                encoding::decode(bytes, enc)
            }
            _ => first,
        };

        let mut warnings = Warnings::new();
        let song = Self::parse_lines(&decoded.text, decoded.encoding, &mut warnings)?;
        Ok(Parsed { song, warnings })
    }

    fn parse_lines(
        text: &str,
        encoding: Encoding,
        warnings: &mut Warnings,
    ) -> Result<Self, ParseError> {
        // Blank lines carry no meaning anywhere in the format, and dropping them up front
        // means neither the header nor the note reader has to special-case them. A line of
        // only spaces is *not* blank: it could be a lyric.
        let lines: Vec<&str> = split_lines(text)
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect();

        let mut rest: &[&str] = &lines;
        let headers = Headers::parse(&mut rest, warnings)?;
        let meta_tags = MetaTags::parse(headers.video.as_deref().unwrap_or(""), warnings);
        let mut cursor = LineCursor::new(rest);
        let tracks = Tracks::parse(&mut cursor, warnings).ok_or(ParseError::NoNotes)?;
        if !cursor.is_empty() {
            let remaining: Vec<&str> = cursor.collect();
            warnings.warn(format!("trailing text in song txt: {remaining:?}"));
        }

        Ok(Self {
            headers,
            tracks,
            meta_tags,
            encoding,
        })
    }

    /// Tempo, falling back to a sane default if `#BPM` was unusable.
    pub fn bpm(&self) -> Bpm {
        self.headers.bpm.unwrap_or(Bpm::new(1.0))
    }

    /// Playback time of a beat, in seconds, including `#GAP`.
    pub fn beat_to_time(&self, beat: f64) -> f64 {
        self.bpm().beat_to_time(beat, self.headers.gap as f64)
    }

    /// Which beat is playing at `secs`.
    pub fn time_to_beat(&self, secs: f64) -> f64 {
        self.bpm().time_to_beat(secs, self.headers.gap as f64)
    }

    /// Shortest audio length that could contain this song, in seconds.
    ///
    /// Used when judging whether a downloaded audio file is the right one.
    pub fn minimum_length_secs(&self) -> f64 {
        self.beat_to_time(f64::from(self.tracks.end()))
    }

    pub fn is_duet(&self) -> bool {
        self.tracks.is_duet()
    }

    /// Serialise back to text with the given line ending.
    pub fn to_string_with(&self, newline: Newline) -> String {
        let body = format!("{}\n{}\n", self.headers, self.tracks);
        match newline {
            Newline::Lf => body,
            Newline::Crlf => body.replace('\n', "\r\n"),
        }
    }

    /// Serialise to bytes, applying the encoding and line ending.
    pub fn to_bytes(&self, encoding: Encoding, newline: Newline) -> Vec<u8> {
        encoding::encode(&self.to_string_with(newline), encoding)
    }
}

impl std::fmt::Display for SongTxt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n{}", self.headers, self.tracks)
    }
}

/// Line ending to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Newline {
    #[default]
    Lf,
    Crlf,
}

impl Newline {
    /// The convention of the host platform.
    pub fn platform_default() -> Self {
        if cfg!(windows) {
            Self::Crlf
        } else {
            Self::Lf
        }
    }
}

/// Look for an `#ENCODING` header without committing to a full parse.
fn scan_declared_encoding(text: &str) -> Option<Encoding> {
    for line in split_lines(text) {
        if !line.starts_with('#') {
            break;
        }
        let (key, value) = line[1..].split_once(':')?;
        if key.eq_ignore_ascii_case("encoding") {
            let enc = Encoding::parse(value);
            return (enc != Encoding::Auto).then_some(enc);
        }
    }
    None
}
