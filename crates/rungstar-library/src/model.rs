//! What the library stores about a song.

use std::path::PathBuf;

/// Everything the browser needs about one song, without touching the disk again.
///
/// Deliberately flat and owned: it comes straight out of a SQL row and goes straight into a
/// list, and the browser scrolls through thousands of these.
#[derive(Debug, Clone, PartialEq)]
pub struct SongEntry {
    pub id: i64,
    /// Absolute path of the `.txt` file, which is the song's identity.
    pub path: PathBuf,
    /// First path segment below the song root — UltraStar's "folder" sort category.
    pub folder: String,

    pub artist: String,
    pub title: String,
    pub edition: Option<String>,
    pub genre: Option<String>,
    pub language: Option<String>,
    pub creator: Option<String>,
    pub tags: Option<String>,
    pub year: Option<i32>,

    pub bpm: f64,
    pub gap_ms: i64,
    /// Length implied by the last note, in seconds. Not the audio file's length.
    pub duration_secs: f64,
    pub is_duet: bool,

    /// Files that sit beside the `.txt`, when the headers name one that exists.
    pub audio_file: Option<String>,
    pub video_file: Option<String>,
    pub cover_file: Option<String>,
    pub background_file: Option<String>,

    pub note_count: i64,
    pub golden_count: i64,
    /// Heuristic difficulty, `0.0..=1.0`. See [`crate::scan::difficulty`].
    pub difficulty: f64,

    pub medley_start: Option<i32>,
    pub medley_end: Option<i32>,
    pub preview_start: Option<f64>,
    /// The USDB id, when the song was downloaded from there.
    pub usdb_id: Option<i64>,

    pub times_played: i64,
    /// Unix seconds.
    pub last_played: Option<i64>,
    /// Integrated loudness in LUFS, once anybody has played the song.
    ///
    /// `None` until then, which means "play it as it is". Measuring costs a full decode, so it
    /// happens the first time the audio is decoded anyway rather than during a scan.
    pub loudness: Option<f32>,
    /// The loudest sample, as a fraction of full scale, measured at the same time.
    ///
    /// Loudness alone does not say how far a song can be turned up: a sparse recording can be
    /// quiet and still touch full scale on one hit, and boosting it clips every one of them.
    pub peak: Option<f32>,
}

impl SongEntry {
    /// The directory the song lives in, where its media files sit.
    pub fn directory(&self) -> Option<&std::path::Path> {
        self.path.parent()
    }

    /// `Artist - Title`.
    pub fn display_name(&self) -> String {
        format!("{} - {}", self.artist, self.title)
    }

    /// Whether the song has everything needed to actually play it.
    pub fn is_playable(&self) -> bool {
        self.audio_file.is_some() && self.note_count > 0
    }

    /// The decade for the decade sort, e.g. `1980`.
    pub fn decade(&self) -> Option<i32> {
        self.year.map(|y| y - y.rem_euclid(10))
    }
}

/// Fields the free-text search can be pointed at.
///
/// Mirrors UltraStar Deluxe's filter list, plus lyrics — which nothing upstream indexes and
/// which is the one people actually reach for when they only remember how a song goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchField {
    #[default]
    All,
    Artist,
    Title,
    Language,
    Edition,
    Genre,
    Year,
    Creator,
    Tags,
    Lyrics,
}

impl SearchField {
    /// The FTS column this field searches, or `None` to search every column.
    pub fn column(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Artist => Some("artist"),
            Self::Title => Some("title"),
            Self::Language => Some("language"),
            Self::Edition => Some("edition"),
            Self::Genre => Some("genre"),
            Self::Year => Some("year"),
            Self::Creator => Some("creator"),
            Self::Tags => Some("tags"),
            Self::Lyrics => Some("lyrics"),
        }
    }
}

/// Where the difficulty scale is cut into bands, measured rather than chosen.
///
/// Fifths of the scale — 0.2, 0.4, 0.6, 0.8 — is the obvious answer and describes nothing.
/// [`crate::scan::difficulty`] is a weighted blend of three clamped scores, and blends do not
/// reach their extremes: across a real 8,159-song library it runs **0.00 to 0.82** with a mean
/// of 0.31, so cut at fifths, 68% of every library is "Easy", "Hard" is half a per cent of it
/// and "Brutal" is a single song in 8,159. A word nearly every song shares is not a word.
///
/// Cut where the library actually lies, the same five words come out 13 / 24 / 32 / 23 / 8 per
/// cent. Not equal fifths of the library either, which was the other candidate: that makes
/// "Brutal" one song in five, and a superlative that common stops being a superlative. A fat
/// middle with narrow ends is how the words are used.
///
/// Only the cut points are calibrated here. The score itself is stored per song and moving it
/// would mean rescanning the library; moving these means nothing, because a band is worked out
/// when it is asked for.
const BAND_EDGES: [f64; 4] = [0.20, 0.28, 0.36, 0.46];

/// How hard a song is to sing, in the words the browser and the song panel both use.
///
/// The index holds difficulty as one number in `0.0..=1.0` (see [`crate::scan::difficulty`]),
/// computed from how fast the syllables come, how wide the melody ranges and how far it jumps.
/// Nobody browses by a number, so this is the same number said out loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifficultyBand {
    Gentle,
    Easy,
    Moderate,
    Hard,
    Brutal,
}

impl DifficultyBand {
    /// Gentlest first. A difficulty scale is a scale, so it is never ordered by popularity the
    /// way a genre list is — Easy above Gentle above Hard reads as a mistake.
    pub const ALL: [DifficultyBand; 5] = [
        Self::Gentle,
        Self::Easy,
        Self::Moderate,
        Self::Hard,
        Self::Brutal,
    ];

    /// What the index and the filter tree pass around, unaffected by how it is displayed.
    pub fn key(self) -> &'static str {
        match self {
            Self::Gentle => "gentle",
            Self::Easy => "easy",
            Self::Moderate => "moderate",
            Self::Hard => "hard",
            Self::Brutal => "brutal",
        }
    }

    /// What a person is shown, on the song panel and in the filter tree alike.
    pub fn label(self) -> &'static str {
        match self {
            Self::Gentle => "Gentle",
            Self::Easy => "Easy",
            Self::Moderate => "Moderate",
            Self::Hard => "Hard",
            Self::Brutal => "Brutal",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|band| band.key() == key)
    }

    /// The half-open span of difficulty this band covers.
    ///
    /// Half-open so the bands tile the scale exactly: a song cannot fall in two of them, and
    /// a song cannot fall in none.
    pub fn range(self) -> (f64, f64) {
        let index = Self::ALL
            .iter()
            .position(|band| *band == self)
            .unwrap_or_default();
        let edge = |at: usize| BAND_EDGES.get(at).copied();
        (
            index
                .checked_sub(1)
                .and_then(edge)
                .unwrap_or(f64::NEG_INFINITY),
            edge(index).unwrap_or(f64::INFINITY),
        )
    }

    /// Which band a difficulty falls in.
    pub fn of(difficulty: f64) -> Self {
        Self::ALL
            .into_iter()
            .find(|band| difficulty < band.range().1)
            .unwrap_or(Self::Brutal)
    }
}

/// How to order results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Artist,
    Title,
    Edition,
    Genre,
    Language,
    Folder,
    Year,
    Decade,
    Creator,
    /// How often this song has been sung.
    TimesPlayed,
    LastPlayed,
    Duration,
    Difficulty,
    /// Best match first. Only meaningful with a search term.
    Relevance,
}

impl SortKey {
    /// The SQL fragment to order by, with a stable tiebreak.
    ///
    /// Every sort falls back to artist then title, so equal keys keep a fixed order rather
    /// than shuffling between queries — a list that reorders itself under the cursor is
    /// unusable.
    pub fn order_by(self, descending: bool) -> String {
        let direction = if descending { "DESC" } else { "ASC" };
        let primary = match self {
            Self::Artist => "song.artist_sort",
            Self::Title => "song.title_sort",
            Self::Edition => "song.edition",
            Self::Genre => "song.genre",
            Self::Language => "song.language",
            Self::Folder => "song.folder",
            Self::Year | Self::Decade => "song.year",
            Self::Creator => "song.creator",
            Self::TimesPlayed => "song.times_played",
            Self::LastPlayed => "song.last_played",
            Self::Duration => "song.duration_secs",
            Self::Difficulty => "song.difficulty",
            Self::Relevance => "relevance",
        };
        // NULLs last regardless of direction: a song with no year belongs at the end of a
        // year sort, not at the top of it.
        format!(
            "{primary} IS NULL, {primary} {direction}, song.artist_sort ASC, song.title_sort ASC"
        )
    }
}
