//! Querying the index.
//!
//! Free text goes through FTS5, which gives prefix matching and diacritic folding for free,
//! so "bea" narrows as you type and "bjork" finds "Björk". When that returns nothing the
//! query falls back to an edit-distance match, which is what rescues "beatls hey jud".
//!
//! Everything else — the facet filters behind the browser's filter tree, and the sort keys —
//! is ordinary SQL against indexed columns.

use rusqlite::{params_from_iter, Row, ToSql};

use crate::db::{Database, DbError};
use crate::model::{SearchField, SongEntry, SortKey};

/// Columns selected for a [`SongEntry`], in the order [`row_to_entry`] reads them.
const SELECT: &str = "song.id, song.path, song.folder, song.artist, song.title, song.edition, \
     song.genre, song.language, song.creator, song.tags, song.year, song.bpm, song.gap_ms, \
     song.duration_secs, song.is_duet, song.audio_file, song.video_file, song.cover_file, \
     song.background_file, song.note_count, song.golden_count, song.difficulty, \
     song.medley_start, song.medley_end, song.preview_start, song.usdb_id, \
     song.times_played, song.last_played";

/// Narrowing applied on top of the free-text search.
///
/// Every list is an "any of these" match, and an empty list means no constraint — which makes
/// the filter tree's checkbox behaviour fall straight out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filters {
    pub artists: Vec<String>,
    pub editions: Vec<String>,
    pub genres: Vec<String>,
    pub languages: Vec<String>,
    pub creators: Vec<String>,
    pub folders: Vec<String>,
    pub years: Vec<i32>,
    pub decades: Vec<i32>,
    /// Only duets, or only solos.
    pub duet: Option<bool>,
    pub has_video: Option<bool>,
    /// Only songs whose audio file is actually present.
    pub playable: Option<bool>,
    /// Inclusive difficulty band.
    pub difficulty: Option<(f64, f64)>,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A search request.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: String,
    pub field: SearchField,
    pub filters: Filters,
    pub sort: SortKey,
    pub descending: bool,
    pub limit: Option<usize>,
    pub offset: usize,
}

impl SearchQuery {
    /// A query for everything, sorted by artist.
    pub fn all() -> Self {
        Self::default()
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    pub fn field(mut self, field: SearchField) -> Self {
        self.field = field;
        self
    }

    pub fn sort(mut self, sort: SortKey, descending: bool) -> Self {
        self.sort = sort;
        self.descending = descending;
        self
    }

    /// Narrow to a subset — duets only, songs with a video, and so on.
    pub fn filters(mut self, filters: Filters) -> Self {
        self.filters = filters;
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// Turn a typed phrase into an FTS5 expression.
///
/// Every word becomes a prefix term and they are ANDed, so words can be typed in any order
/// and the last one narrows as it is still being typed. Quotes are stripped rather than
/// escaped: a stray quote in a search box is a typo, not an operator.
fn fts_expression(text: &str, field: SearchField) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|word| word.replace(['"', '*', ':', '(', ')', '^'], ""))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\"*"))
        .collect();
    if terms.is_empty() {
        return None;
    }
    let joined = terms.join(" AND ");
    Some(match field.column() {
        Some(column) => format!("{column} : ({joined})"),
        None => joined,
    })
}

/// The same words as a single phrase, when more than one was typed.
///
/// bm25 scores a document on how many of the query's terms it contains and how short it is;
/// it has no notion of the terms being *adjacent*. So "never gonna give you up" ranks a song
/// that happens to contain all five words scattered through it above the song those five
/// words are the chorus of. Searching the phrase separately is what fixes that, and it is the
/// difference between the lyric search finding your song and merely containing it.
///
/// The trailing `*` prefix-matches the final token, so the phrase still narrows while the
/// last word is being typed.
fn phrase_expression(text: &str, field: SearchField) -> Option<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| word.replace(['"', '*', ':', '(', ')', '^'], ""))
        .filter(|word| !word.is_empty())
        .collect();
    if words.len() < 2 {
        return None;
    }
    let phrase = format!("\"{}\"*", words.join(" "));
    Some(match field.column() {
        Some(column) => format!("{column} : ({phrase})"),
        None => phrase,
    })
}

/// Put the phrase matches in front, keeping each group's own order and dropping duplicates.
fn phrase_first(
    phrase: Vec<SongEntry>,
    rest: Vec<SongEntry>,
    limit: Option<usize>,
) -> Vec<SongEntry> {
    let mut seen: std::collections::HashSet<i64> = phrase.iter().map(|s| s.id).collect();
    let mut merged = phrase;
    for song in rest {
        if seen.insert(song.id) {
            merged.push(song);
        }
    }
    if let Some(limit) = limit {
        merged.truncate(limit);
    }
    merged
}

fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<SongEntry> {
    Ok(SongEntry {
        id: row.get(0)?,
        path: std::path::PathBuf::from(row.get::<_, String>(1)?),
        folder: row.get(2)?,
        artist: row.get(3)?,
        title: row.get(4)?,
        edition: row.get(5)?,
        genre: row.get(6)?,
        language: row.get(7)?,
        creator: row.get(8)?,
        tags: row.get(9)?,
        year: row.get(10)?,
        bpm: row.get(11)?,
        gap_ms: row.get(12)?,
        duration_secs: row.get(13)?,
        is_duet: row.get(14)?,
        audio_file: row.get(15)?,
        video_file: row.get(16)?,
        cover_file: row.get(17)?,
        background_file: row.get(18)?,
        note_count: row.get(19)?,
        golden_count: row.get(20)?,
        difficulty: row.get(21)?,
        medley_start: row.get(22)?,
        medley_end: row.get(23)?,
        preview_start: row.get(24)?,
        usdb_id: row.get(25)?,
        times_played: row.get(26)?,
        last_played: row.get(27)?,
    })
}

/// Build the `WHERE` fragments and their bound values for a set of filters.
fn filter_clauses(filters: &Filters) -> (Vec<String>, Vec<Box<dyn ToSql>>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();

    let mut push_text = |column: &str, list: &Vec<String>| {
        if list.is_empty() {
            return;
        }
        let slots = vec!["?"; list.len()].join(", ");
        clauses.push(format!("song.{column} IN ({slots})"));
        for value in list {
            values.push(Box::new(value.clone()));
        }
    };
    push_text("artist", &filters.artists);
    push_text("edition", &filters.editions);
    push_text("genre", &filters.genres);
    push_text("language", &filters.languages);
    push_text("creator", &filters.creators);
    push_text("folder", &filters.folders);

    if !filters.years.is_empty() {
        let slots = vec!["?"; filters.years.len()].join(", ");
        clauses.push(format!("song.year IN ({slots})"));
        for year in &filters.years {
            values.push(Box::new(*year));
        }
    }
    if !filters.decades.is_empty() {
        // A decade is every year that floors to it.
        let slots = vec!["?"; filters.decades.len()].join(", ");
        clauses.push(format!("(song.year - (song.year % 10)) IN ({slots})"));
        for decade in &filters.decades {
            values.push(Box::new(*decade));
        }
    }
    if let Some(duet) = filters.duet {
        clauses.push("song.is_duet = ?".to_owned());
        values.push(Box::new(duet));
    }
    if let Some(has_video) = filters.has_video {
        clauses.push(
            if has_video {
                "song.video_file IS NOT NULL"
            } else {
                "song.video_file IS NULL"
            }
            .to_owned(),
        );
    }
    if let Some(playable) = filters.playable {
        clauses.push(
            if playable {
                "song.audio_file IS NOT NULL AND song.note_count > 0"
            } else {
                "song.audio_file IS NULL OR song.note_count = 0"
            }
            .to_owned(),
        );
    }
    if let Some((low, high)) = filters.difficulty {
        clauses.push("song.difficulty BETWEEN ? AND ?".to_owned());
        values.push(Box::new(low));
        values.push(Box::new(high));
    }
    (clauses, values)
}

impl Database {
    /// Run a search.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SongEntry>, DbError> {
        let expression = fts_expression(&query.text, query.field);
        let results = self.search_indexed(query, expression.as_deref())?;

        // Nothing matched, but something was typed: the words are probably misspelled rather
        // than absent, so try again by similarity.
        if results.is_empty() && expression.is_some() {
            return self.search_fuzzy(query);
        }

        // Ranked by relevance, songs containing the words as a phrase come first. Only when
        // ranking: under an artist or title sort the player has asked for a specific order
        // and reordering it would be wrong.
        if query.sort == SortKey::Relevance && !results.is_empty() {
            if let Some(phrase) = phrase_expression(&query.text, query.field) {
                let exact = self.search_indexed(query, Some(&phrase))?;
                if !exact.is_empty() {
                    return Ok(phrase_first(exact, results, query.limit));
                }
            }
        }
        Ok(results)
    }

    fn search_indexed(
        &self,
        query: &SearchQuery,
        expression: Option<&str>,
    ) -> Result<Vec<SongEntry>, DbError> {
        let (mut clauses, mut values) = filter_clauses(&query.filters);
        let mut sql = format!("SELECT {SELECT}");

        match expression {
            Some(expression) => {
                // bm25 is negative-better, so it is negated to make "higher is more relevant".
                sql.push_str(
                    ", -bm25(song_fts) AS relevance FROM song \
                     JOIN song_fts ON song_fts.song_id = song.id",
                );
                clauses.insert(0, "song_fts MATCH ?".to_owned());
                values.insert(0, Box::new(expression.to_owned()));
            }
            None => sql.push_str(", 0 AS relevance FROM song"),
        }

        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        // Relevance means nothing without a search term; fall back to artist order.
        let sort = if query.sort == SortKey::Relevance && expression.is_none() {
            SortKey::Artist
        } else {
            query.sort
        };
        sql.push_str(" ORDER BY ");
        sql.push_str(&sort.order_by(query.descending || sort == SortKey::Relevance));
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {limit} OFFSET {}", query.offset));
        }

        let mut statement = self.connection().prepare(&sql)?;
        let bound = params_from_iter(values.iter().map(std::convert::AsRef::as_ref));
        let rows = statement.query_map(bound, row_to_entry)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// One song by id.
    pub fn song(&self, id: i64) -> Result<Option<SongEntry>, DbError> {
        let sql = format!("SELECT {SELECT}, 0 AS relevance FROM song WHERE song.id = ?1");
        let mut statement = self.connection().prepare(&sql)?;
        let mut rows = statement.query_map([id], row_to_entry)?;
        Ok(rows.next().transpose()?)
    }

    /// Note that a song was sung, for the play-count sorts.
    pub fn record_play(&self, id: i64) -> Result<(), DbError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.connection().execute(
            "UPDATE song SET times_played = times_played + 1, last_played = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        Ok(())
    }

    /// Distinct values of a column with their counts, for the filter tree.
    ///
    /// Only columns the tree offers are accepted, so this cannot become an injection point.
    pub fn facet(&self, column: &str) -> Result<Vec<(String, i64)>, DbError> {
        // Decades are computed rather than stored, and are ordered newest first rather than
        // by count: a decade list is a timeline and shuffling it by popularity makes it
        // unreadable.
        if column == "decade" {
            let mut statement = self.connection().prepare(
                "SELECT CAST(year - (year % 10) AS TEXT), COUNT(*) FROM song
                 WHERE year IS NOT NULL AND year > 0
                 GROUP BY year - (year % 10) ORDER BY year - (year % 10) DESC",
            )?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            return Ok(rows.collect::<Result<Vec<_>, _>>()?);
        }
        let column = match column {
            "artist" | "edition" | "genre" | "language" | "creator" | "folder" => column,
            _ => return Ok(Vec::new()),
        };
        let sql = format!(
            "SELECT {column}, COUNT(*) FROM song WHERE {column} IS NOT NULL AND {column} <> '' \
             GROUP BY {column} ORDER BY COUNT(*) DESC, {column} ASC"
        );
        let mut statement = self.connection().prepare(&sql)?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

impl Database {
    /// Similarity search, used when the index found nothing.
    ///
    /// Bounded to artist and title: those are what people mistype. Thirty thousand rows of
    /// edit distance is a couple of milliseconds, and it only runs once the fast path has
    /// already come back empty.
    fn search_fuzzy(&self, query: &SearchQuery) -> Result<Vec<SongEntry>, DbError> {
        let needles: Vec<String> = query
            .text
            .split_whitespace()
            .map(crate::scan::fold_for_sort)
            .filter(|word| !word.is_empty())
            .collect();
        if needles.is_empty() {
            return Ok(Vec::new());
        }

        let (clauses, values) = filter_clauses(&query.filters);
        let mut sql = format!("SELECT {SELECT}, 0 AS relevance FROM song");
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }

        let mut statement = self.connection().prepare(&sql)?;
        let bound = params_from_iter(values.iter().map(std::convert::AsRef::as_ref));
        let rows = statement.query_map(bound, |row| {
            let entry = row_to_entry(row)?;
            let haystack = format!(
                "{} {}",
                crate::scan::fold_for_sort(&entry.artist),
                crate::scan::fold_for_sort(&entry.title)
            );
            Ok((entry, haystack))
        })?;

        let mut scored: Vec<(u32, SongEntry)> = Vec::new();
        for row in rows {
            let (entry, haystack) = row?;
            if let Some(score) = similarity(&needles, &haystack) {
                scored.push((score, entry));
            }
        }
        // Closest first, then the usual stable order.
        scored.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.artist.cmp(&b.1.artist))
                .then_with(|| a.1.title.cmp(&b.1.title))
        });
        let limit = query.limit.unwrap_or(usize::MAX);
        Ok(scored
            .into_iter()
            .take(limit)
            .map(|(_, entry)| entry)
            .collect())
    }
}

/// Total edit distance of every needle against its best word in `haystack`.
///
/// `None` when any needle is too far from anything, which keeps unrelated songs out rather
/// than returning the whole library in similarity order.
fn similarity(needles: &[String], haystack: &str) -> Option<u32> {
    let words: Vec<&str> = haystack.split_whitespace().collect();
    let mut total = 0;
    for needle in needles {
        // Roughly one edit per three characters, so short words stay strict and longer ones
        // forgive a slip or two.
        let budget = (needle.chars().count() / 3).max(1) as u32;
        let best = words
            .iter()
            .map(|word| edit_distance(needle, word, budget))
            .min()
            .unwrap_or(u32::MAX);
        if best > budget {
            return None;
        }
        total += best;
    }
    Some(total)
}

/// Levenshtein distance, abandoned once it exceeds `budget`.
///
/// The cutoff matters: this runs over every row in the library, and most rows are nothing
/// like the query and can be rejected after a couple of rows of the matrix.
fn edit_distance(a: &str, b: &str, budget: u32) -> u32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) as u32 > budget {
        return u32::MAX;
    }
    let mut previous: Vec<u32> = (0..=b.len() as u32).collect();
    let mut current = vec![0u32; b.len() + 1];

    for (i, &ca) in a.iter().enumerate() {
        current[0] = i as u32 + 1;
        let mut row_best = current[0];
        for (j, &cb) in b.iter().enumerate() {
            let cost = u32::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
            row_best = row_best.min(current[j + 1]);
        }
        if row_best > budget {
            return u32::MAX;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}
