//! The `#TAG:value` block at the top of a song file.

use std::fmt;

use crate::bpm::Bpm;
use crate::error::{MissingHeaders, ParseError, Warnings};

/// Headers whose duplicates UltraStar Deluxe concatenates rather than overwrites.
const MULTIVALUE: [&str; 4] = ["genre", "edition", "creator", "language"];

/// Parsed song headers.
///
/// Field order here mirrors the order they are written back out in, which is the order
/// UltraStar tooling has settled on. Unrecognised headers are preserved verbatim in
/// [`Headers::unknown`] so that round-tripping a file never loses data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Headers {
    pub version: Option<String>,
    pub title: String,
    pub artist: String,
    pub language: Option<String>,
    pub edition: Option<String>,
    pub genre: Option<String>,
    /// Kept as text: files carry things like `1985` but also `199x`.
    pub year: Option<String>,
    pub creator: Option<String>,
    pub mp3: Option<String>,
    pub audio: Option<String>,
    pub audiourl: Option<String>,
    pub vocals: Option<String>,
    pub instrumental: Option<String>,
    pub cover: Option<String>,
    pub coverurl: Option<String>,
    pub background: Option<String>,
    pub backgroundurl: Option<String>,
    pub video: Option<String>,
    pub videourl: Option<String>,
    /// Seconds. Absent when zero, since a zero offset is the default.
    pub videogap: Option<f64>,
    pub resolution: Option<String>,
    /// Seconds.
    pub start: Option<f64>,
    /// Milliseconds.
    pub end: Option<i64>,
    pub relative: Option<String>,
    /// Seconds.
    pub previewstart: Option<f64>,
    pub medleystartbeat: Option<i32>,
    pub medleyendbeat: Option<i32>,
    pub bpm: Option<Bpm>,
    /// Milliseconds. Always written, defaulting to zero.
    pub gap: i64,
    pub p1: Option<String>,
    pub p2: Option<String>,
    pub album: Option<String>,
    pub comment: Option<String>,
    pub providedby: Option<String>,
    pub tags: Option<String>,
    /// Not rewritten: it describes the bytes on disk, which we re-decide when saving.
    pub encoding: Option<String>,
    /// Everything else, lower-cased keys, in the order first seen.
    pub unknown: Vec<(String, String)>,
}

/// Order headers are emitted in. `encoding` is deliberately absent.
const WRITE_ORDER: [&str; 35] = [
    "version",
    "title",
    "artist",
    "language",
    "edition",
    "genre",
    "year",
    "creator",
    "mp3",
    "audio",
    "audiourl",
    "vocals",
    "instrumental",
    "cover",
    "coverurl",
    "background",
    "backgroundurl",
    "video",
    "videourl",
    "videogap",
    "resolution",
    "start",
    "end",
    "relative",
    "previewstart",
    "medleystartbeat",
    "medleyendbeat",
    "bpm",
    "gap",
    "p1",
    "p2",
    "album",
    "comment",
    "providedby",
    "tags",
];

impl Headers {
    /// Consume the leading `#...` lines.
    ///
    /// Header order in the file is irrelevant; the block simply ends at the first line that
    /// does not begin with `#`.
    pub fn parse(lines: &mut &[&str], warnings: &mut Warnings) -> Result<Self, ParseError> {
        let mut headers = Headers::default();
        let mut seen = SeenRequired::default();

        while let Some(raw) = lines.first() {
            if !raw.starts_with('#') {
                break;
            }
            *lines = &lines[1..];
            let line = &raw[1..];
            let Some((key, value)) = line.split_once(':') else {
                warnings.warn(format!("header without value: '{line}'"));
                continue;
            };
            if value.is_empty() {
                // A header with nothing after the colon carries no information.
                continue;
            }
            if !headers.set(key, value, &mut seen) {
                warnings.warn(format!("invalid header value: '{line}'"));
            }
        }

        let missing = MissingHeaders {
            title: !seen.title,
            artist: !seen.artist,
            bpm: !seen.bpm,
        };
        if missing.any() {
            return Err(ParseError::MissingRequiredHeaders(missing));
        }
        Ok(headers)
    }

    /// Apply one `#KEY:VALUE`. Returns `false` if the value could not be interpreted.
    fn set(&mut self, key: &str, value: &str, seen: &mut SeenRequired) -> bool {
        // `#AUTHOR` is the old spelling of `#CREATOR`.
        let key = if key.eq_ignore_ascii_case("author") {
            "creator".to_owned()
        } else {
            key.to_ascii_lowercase()
        };

        match key.as_str() {
            "title" => {
                // Duet files often advertise themselves in the title; the flag really lives
                // in the note section, so the decoration is dropped.
                self.title = value.strip_suffix(" [DUET]").unwrap_or(value).to_owned();
                seen.title = true;
            }
            "artist" => {
                self.artist = value.to_owned();
                seen.artist = true;
            }
            "bpm" => match Bpm::parse(value) {
                Some(bpm) => {
                    self.bpm = Some(bpm);
                    seen.bpm = true;
                }
                None => return false,
            },
            // Fractional seconds, decimal comma tolerated. A zero offset is the default, so
            // it is neither stored nor written back.
            "videogap" | "start" | "previewstart" => match parse_decimal(value) {
                Some(v) => {
                    if v != 0.0 {
                        match key.as_str() {
                            "videogap" => self.videogap = Some(v),
                            "start" => self.start = Some(v),
                            _ => self.previewstart = Some(v),
                        }
                    }
                }
                None => return false,
            },
            // Milliseconds, but written with a decimal point often enough to matter.
            "gap" => match parse_decimal(value) {
                Some(v) => self.gap = round_half_to_even(v),
                None => return false,
            },
            "end" => match parse_decimal(value) {
                Some(v) => self.end = Some(round_half_to_even(v)),
                None => return false,
            },
            // Beats are whole numbers; a fractional medley bound is a mistake, not a value
            // to round.
            "medleystartbeat" => match value.trim().parse::<i32>() {
                Ok(v) => self.medleystartbeat = Some(v),
                Err(_) => return false,
            },
            "medleyendbeat" => match value.trim().parse::<i32>() {
                Ok(v) => self.medleyendbeat = Some(v),
                Err(_) => return false,
            },
            _ => {
                if let Some(slot) = self.text_slot(&key) {
                    // UltraStar Deluxe merges repeated multi-value headers rather than
                    // letting the last one win.
                    if MULTIVALUE.contains(&key.as_str()) {
                        if let Some(existing) = slot.take() {
                            *slot = Some(format!("{value},{existing}"));
                            return true;
                        }
                    }
                    *slot = Some(value.to_owned());
                } else {
                    self.set_unknown(key, value);
                }
            }
        }
        true
    }

    fn set_unknown(&mut self, key: String, value: &str) {
        if let Some(entry) = self.unknown.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value.to_owned();
        } else {
            self.unknown.push((key, value.to_owned()));
        }
    }
}

impl Headers {
    fn text_slot(&mut self, key: &str) -> Option<&mut Option<String>> {
        Some(match key {
            "version" => &mut self.version,
            "language" => &mut self.language,
            "edition" => &mut self.edition,
            "genre" => &mut self.genre,
            "year" => &mut self.year,
            "creator" => &mut self.creator,
            "mp3" => &mut self.mp3,
            "audio" => &mut self.audio,
            "audiourl" => &mut self.audiourl,
            "vocals" => &mut self.vocals,
            "instrumental" => &mut self.instrumental,
            "cover" => &mut self.cover,
            "coverurl" => &mut self.coverurl,
            "background" => &mut self.background,
            "backgroundurl" => &mut self.backgroundurl,
            "video" => &mut self.video,
            "videourl" => &mut self.videourl,
            "resolution" => &mut self.resolution,
            "relative" => &mut self.relative,
            "p1" => &mut self.p1,
            "p2" => &mut self.p2,
            "album" => &mut self.album,
            "comment" => &mut self.comment,
            "providedby" => &mut self.providedby,
            "tags" => &mut self.tags,
            "encoding" => &mut self.encoding,
            _ => return None,
        })
    }

    /// Rendered value of a header by its lower-case name, if set.
    fn value_of(&self, key: &str) -> Option<String> {
        Some(match key {
            "title" => self.title.clone(),
            "artist" => self.artist.clone(),
            "gap" => self.gap.to_string(),
            "bpm" => self.bpm?.to_string(),
            "videogap" => format_decimal(self.videogap?),
            "start" => format_decimal(self.start?),
            "previewstart" => format_decimal(self.previewstart?),
            "end" => self.end?.to_string(),
            "medleystartbeat" => self.medleystartbeat?.to_string(),
            "medleyendbeat" => self.medleyendbeat?.to_string(),
            "version" => self.version.clone()?,
            "language" => self.language.clone()?,
            "edition" => self.edition.clone()?,
            "genre" => self.genre.clone()?,
            "year" => self.year.clone()?,
            "creator" => self.creator.clone()?,
            "mp3" => self.mp3.clone()?,
            "audio" => self.audio.clone()?,
            "audiourl" => self.audiourl.clone()?,
            "vocals" => self.vocals.clone()?,
            "instrumental" => self.instrumental.clone()?,
            "cover" => self.cover.clone()?,
            "coverurl" => self.coverurl.clone()?,
            "background" => self.background.clone()?,
            "backgroundurl" => self.backgroundurl.clone()?,
            "video" => self.video.clone()?,
            "videourl" => self.videourl.clone()?,
            "resolution" => self.resolution.clone()?,
            "relative" => self.relative.clone()?,
            "p1" => self.p1.clone()?,
            "p2" => self.p2.clone()?,
            "album" => self.album.clone()?,
            "comment" => self.comment.clone()?,
            "providedby" => self.providedby.clone()?,
            "tags" => self.tags.clone()?,
            _ => return None,
        })
    }

    /// `Artist - Title`, the conventional display and folder name.
    pub fn artist_title(&self) -> String {
        format!("{} - {}", self.artist, self.title)
    }

    /// Whether relative timing is in effect.
    pub fn is_relative(&self) -> bool {
        self.relative
            .as_deref()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"))
    }

    /// Whether a medley section should be auto-detected.
    ///
    /// `#CALCMEDLEY:OFF` opts out; anything else, including absence, leaves it on.
    pub fn calc_medley(&self) -> bool {
        !self
            .unknown
            .iter()
            .any(|(k, v)| k == "calcmedley" && v.trim().eq_ignore_ascii_case("off"))
    }

    /// The audio file to play: `#AUDIO` if present, else the legacy `#MP3`.
    pub fn audio_file(&self) -> Option<&str> {
        self.audio.as_deref().or(self.mp3.as_deref())
    }

    /// Primary language, with any romanisation marker removed.
    pub fn main_language(&self) -> &str {
        self.language
            .as_deref()
            .map(|l| l.split(',').next().unwrap_or(l).trim())
            .map(|l| l.strip_suffix(" (romanized)").unwrap_or(l))
            .unwrap_or("")
    }

    /// Clear every header naming a local file. Used before re-resolving downloads.
    pub fn reset_file_location_headers(&mut self) {
        self.mp3 = None;
        self.audio = None;
        self.instrumental = None;
        self.vocals = None;
        self.video = None;
        self.cover = None;
        self.background = None;
    }
}

impl fmt::Display for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        // An empty value is skipped rather than written: the parser ignores such headers, so
        // emitting one would produce a file that does not read back as what was written.
        let named = WRITE_ORDER.iter().filter_map(|key| {
            self.value_of(key)
                .filter(|value| !value.is_empty())
                .map(|value| (key.to_ascii_uppercase(), value))
        });
        let unknown = self
            .unknown
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.to_ascii_uppercase(), v.clone()));
        for (key, value) in named.chain(unknown) {
            if !first {
                f.write_str("\n")?;
            }
            write!(f, "#{key}:{value}")?;
            first = false;
        }
        Ok(())
    }
}

#[derive(Default)]
struct SeenRequired {
    title: bool,
    artist: bool,
    bpm: bool,
}

/// Parse a number that may use a decimal comma.
fn parse_decimal(value: &str) -> Option<f64> {
    let v: f64 = value.trim().replace(',', ".").parse().ok()?;
    v.is_finite().then_some(v)
}

/// Render a float without a trailing `.0`, matching what other UltraStar tools emit.
fn format_decimal(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let mut s = format!("{v}");
    if s.contains('e') || s.contains('E') {
        s = format!("{v:.6}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Round half to even, matching Python's `round()`.
///
/// Worth the trouble because `#GAP` values land on exact halves often enough that a
/// different rule would show up as a one-millisecond diff against the reference tooling.
fn round_half_to_even(x: f64) -> i64 {
    let floor = x.floor();
    let diff = x - floor;
    // Exactly halfway goes to whichever neighbour is even; everything else rounds normally.
    let round_up = diff > 0.5 || (diff == 0.5 && (floor as i64) % 2 != 0);
    (if round_up { floor + 1.0 } else { floor }) as i64
}
