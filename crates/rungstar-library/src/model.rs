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
