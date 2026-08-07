//! Finding songs on disk and getting them into the index.
//!
//! A song is any directory containing a `.txt` file — there is no naming convention to rely
//! on, and twenty years of collections prove it.
//!
//! Rescans are incremental. A file whose size and timestamp match the index is not opened at
//! all, which turns a cold start on a large collection from "watch the progress bar" into
//! something that finishes before the menu has faded in. Parsing runs across every core,
//! since it is pure CPU work over independent files; only the database writes are serial,
//! because SQLite writes are serial anyway.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rungstar_song::SongTxt;
use walkdir::WalkDir;

use crate::db::{Database, DbError, Freshness};

/// What a scan found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    /// Files skipped because they had not changed.
    pub unchanged: usize,
    /// Index entries dropped because the file is gone.
    pub removed: usize,
    /// Files that could not be parsed. They stay out of the index rather than half in it.
    pub failed: usize,
}

impl ScanReport {
    pub fn total_indexed(&self) -> usize {
        self.added + self.updated + self.unchanged
    }
}

/// How to scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Directories to search, each treated as a separate root for folder categories.
    pub roots: Vec<PathBuf>,
    /// Re-read and re-hash every file, even ones that look unchanged.
    ///
    /// Size and timestamp miss a file edited in place within the same second, and cloud sync
    /// can rewrite a timestamp without touching the content. This is the "I do not trust the
    /// index" option.
    pub verify: bool,
}

impl ScanOptions {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
            verify: false,
        }
    }
}

/// A `.txt` found on disk, before it has been read.
#[derive(Debug, Clone)]
struct Candidate {
    path: PathBuf,
    root: PathBuf,
    folder: String,
    mtime: i64,
    size: i64,
}

/// A song that has been read and is ready to be written to the index.
#[derive(Debug, Clone)]
pub struct ParsedSong {
    pub path: String,
    pub root: String,
    pub folder: String,
    pub mtime: i64,
    pub size: i64,
    pub hash: [u8; 32],

    pub artist: String,
    pub title: String,
    pub artist_sort: String,
    pub title_sort: String,
    pub edition: Option<String>,
    pub genre: Option<String>,
    pub language: Option<String>,
    pub creator: Option<String>,
    pub tags: Option<String>,
    pub year: Option<i32>,

    pub bpm: f64,
    pub gap_ms: i64,
    pub duration_secs: f64,
    pub is_duet: bool,

    pub audio_file: Option<String>,
    pub video_file: Option<String>,
    pub cover_file: Option<String>,
    pub background_file: Option<String>,

    pub note_count: i64,
    pub golden_count: i64,
    pub difficulty: f64,

    pub medley_start: Option<i32>,
    pub medley_end: Option<i32>,
    pub preview_start: Option<f64>,
    pub usdb_id: Option<i64>,

    /// All the lyrics as one string, for the full-text index.
    pub lyrics: String,
    /// Whether this song was absent from the index, so the writer can skip the search-index
    /// delete that only matters when replacing an existing row.
    pub is_new: bool,
}

/// Walk the roots and collect every `.txt` with its size and timestamp.
fn collect(options: &ScanOptions) -> Vec<Candidate> {
    let mut found = Vec::new();
    for root in &options.roots {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("txt"))
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);

            found.push(Candidate {
                folder: folder_category(root, path),
                root: root.clone(),
                path: path.to_path_buf(),
                mtime,
                size: metadata.len() as i64,
            });
        }
    }
    found
}

/// The sort category a song falls into: the first directory below its root.
///
/// A song sitting directly in the root takes the root's own name, so it still lands
/// somewhere rather than in a blank category.
fn folder_category(root: &Path, song: &Path) -> String {
    let relative = song.strip_prefix(root).unwrap_or(song);
    let mut parts = relative.components();
    match (parts.next(), parts.next()) {
        // At least two components means the first is a directory.
        (Some(first), Some(_)) => first.as_os_str().to_string_lossy().into_owned(),
        _ => root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned()),
    }
}

/// Read and interpret one song file.
fn parse(candidate: &Candidate) -> Option<ParsedSong> {
    let bytes = std::fs::read(&candidate.path).ok()?;
    let hash = blake3::hash(&bytes);
    let parsed = SongTxt::parse_bytes(&bytes).ok()?;
    let song = parsed.song;

    let headers = &song.headers;
    let bpm = song.bpm().value();
    let duration_secs = song.minimum_length_secs().max(0.0);

    let notes: Vec<_> = song.tracks.all_notes().collect();
    let note_count = notes.len() as i64;
    let golden_count = notes.iter().filter(|n| n.kind.is_golden()).count() as i64;

    // Lyrics as one blob for the full-text index. Syllable markers are dropped so a search
    // for "together" matches a word split across four notes.
    let lyrics: String = song
        .tracks
        .all_lines()
        .map(|line| line.text().trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    // One directory listing instead of a stat per media file. On Windows each stat goes
    // through the filter driver stack, and at four per song across a large library that
    // dominates the whole scan.
    let directory = candidate.path.parent().unwrap_or(&candidate.root);
    let present: Vec<String> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    let beside = |name: Option<&str>| -> Option<String> {
        let name = name?;
        // Case-insensitive: headers routinely disagree with the file's actual casing, and on
        // Windows the file opens anyway.
        present
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(name))
            .then(|| name.to_owned())
    };

    Some(ParsedSong {
        path: candidate.path.to_string_lossy().into_owned(),
        root: candidate.root.to_string_lossy().into_owned(),
        folder: candidate.folder.clone(),
        mtime: candidate.mtime,
        size: candidate.size,
        hash: *hash.as_bytes(),

        artist_sort: fold_for_sort(&headers.artist),
        title_sort: fold_for_sort(&headers.title),
        artist: headers.artist.clone(),
        title: headers.title.clone(),
        edition: headers.edition.clone(),
        genre: headers.genre.clone(),
        language: headers.language.clone(),
        creator: headers.creator.clone(),
        tags: headers.tags.clone(),
        year: headers.year.as_deref().and_then(|y| y.trim().parse().ok()),

        bpm,
        gap_ms: headers.gap,
        duration_secs,
        is_duet: song.is_duet(),

        audio_file: beside(headers.audio_file()),
        video_file: beside(headers.video.as_deref()),
        cover_file: beside(headers.cover.as_deref()),
        background_file: beside(headers.background.as_deref()),

        note_count,
        golden_count,
        difficulty: difficulty(&notes, duration_secs),

        medley_start: headers.medleystartbeat,
        medley_end: headers.medleyendbeat,
        preview_start: headers.previewstart,
        usdb_id: usdb_id(headers),

        lyrics,
        // Filled in by the caller, which is what knows.
        is_new: false,
    })
}

/// The USDB id, if the file records one.
///
/// Songs downloaded by usdb_syncer keep their id in a sidecar rather than the text, but
/// several other tools write a `#USDB_ID` or a `#COMMENT` pointing at the page, and knowing
/// the id lets the in-game browser show what is already owned.
fn usdb_id(headers: &rungstar_song::Headers) -> Option<i64> {
    if let Some((_, value)) = headers.unknown.iter().find(|(key, _)| key == "usdb_id") {
        return value.trim().parse().ok();
    }
    let comment = headers.comment.as_deref()?;
    let marker = comment.find("usdb.animux.de")?;
    comment[marker..]
        .split("id=")
        .nth(1)?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// A heuristic difficulty, `0.0..=1.0`.
///
/// Three things make a song hard to sing, and they are close to independent: how fast the
/// syllables come, how wide the melody ranges, and how far it jumps between consecutive
/// notes. A slow ballad across two octaves and a fast rap on one note are both hard, in
/// different ways, and neither shows up if you only count notes.
///
/// This is a browsing aid, not a claim about vocal technique.
pub fn difficulty(notes: &[&rungstar_song::Note], duration_secs: f64) -> f64 {
    let pitched: Vec<i32> = notes
        .iter()
        .filter(|n| n.kind.has_pitch())
        .map(|n| n.pitch)
        .collect();
    if pitched.len() < 2 || duration_secs <= 0.0 {
        return 0.0;
    }

    // Syllables per second. One is conversational, five is a patter song.
    let density = pitched.len() as f64 / duration_secs;
    let density_score = ((density - 1.0) / 4.0).clamp(0.0, 1.0);

    // Total range in semitones. An octave is comfortable, three is not.
    let (low, high) = pitched
        .iter()
        .fold((i32::MAX, i32::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let range_score = (f64::from(high - low - 8) / 24.0).clamp(0.0, 1.0);

    // Mean interval between consecutive notes: how jumpy the melody is.
    let jumps: i64 = pitched
        .windows(2)
        .map(|w| i64::from((w[1] - w[0]).abs()))
        .sum();
    let volatility = jumps as f64 / (pitched.len() - 1) as f64;
    let volatility_score = ((volatility - 0.5) / 4.0).clamp(0.0, 1.0);

    (0.4 * density_score + 0.3 * range_score + 0.3 * volatility_score).clamp(0.0, 1.0)
}

/// Fold text into a sort key: lower case, common accents reduced to their base letter.
///
/// Sorting on the raw string puts "Ärzte" after "Zappa", which is not where anyone looks for
/// it. This is a sort key only — the display name keeps its accents.
pub fn fold_for_sort(text: &str) -> String {
    text.trim()
        .chars()
        .flat_map(|c| {
            let base = match c {
                'à'..='å' | 'À'..='Å' => 'a',
                'è'..='ë' | 'È'..='Ë' => 'e',
                'ì'..='ï' | 'Ì'..='Ï' => 'i',
                'ò'..='ö' | 'Ò'..='Ö' => 'o',
                'ù'..='ü' | 'Ù'..='Ü' => 'u',
                'ý' | 'ÿ' | 'Ý' => 'y',
                'ñ' | 'Ñ' => 'n',
                'ç' | 'Ç' => 'c',
                'ß' => 's',
                other => other,
            };
            base.to_lowercase()
        })
        .collect()
}

/// Bring the index in line with what is on disk.
pub fn scan(database: &mut Database, options: &ScanOptions) -> Result<ScanReport, DbError> {
    let candidates = collect(options);
    let mut report = ScanReport::default();

    // Decide what needs reading before touching any file, so the parallel pass below is pure
    // CPU work with no database contention.
    // One query for the whole index rather than one per file: a lookup per song costs a
    // statement compilation each time, and there are tens of thousands of them.
    let known = database.existing_files()?;
    let mut to_parse = Vec::new();
    let mut seen = std::collections::HashSet::with_capacity(candidates.len());
    for candidate in candidates {
        let path = candidate.path.to_string_lossy().into_owned();
        let freshness = match known.get(&path) {
            None => Freshness::Unknown,
            Some(&(mtime, size)) if mtime == candidate.mtime && size == candidate.size => {
                Freshness::Unchanged(0)
            }
            Some(_) => Freshness::Stale(0),
        };
        seen.insert(path);
        match freshness {
            Freshness::Unchanged(_) if !options.verify => report.unchanged += 1,
            Freshness::Unchanged(_) | Freshness::Stale(_) => to_parse.push((candidate, false)),
            Freshness::Unknown => to_parse.push((candidate, true)),
        }
    }

    let parsed: Vec<(Option<ParsedSong>, bool)> = to_parse
        .par_iter()
        .map(|(candidate, is_new)| (parse(candidate), *is_new))
        .collect();

    let mut songs = Vec::with_capacity(parsed.len());
    for (song, is_new) in parsed {
        match song {
            Some(mut song) => {
                song.is_new = is_new;
                if is_new {
                    report.added += 1;
                } else {
                    report.updated += 1;
                }
                songs.push(song);
            }
            None => report.failed += 1,
        }
    }
    database.upsert_songs(&songs)?;

    // Anything indexed but no longer on disk has been deleted or moved away.
    let gone: Vec<String> = database
        .all_paths()?
        .into_iter()
        .filter(|path| !seen.contains(path))
        .collect();
    report.removed = database.remove_paths(&gone)?;

    Ok(report)
}
