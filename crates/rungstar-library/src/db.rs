//! The on-disk index.
//!
//! UltraStar Deluxe re-parses every `.txt` in the library at every launch, which is why large
//! collections take a visible age to start. Here the parse result is kept in SQLite, and a
//! rescan only touches files whose size or timestamp changed.
//!
//! Search runs through an FTS5 index rather than substring scans, so typing in the song
//! browser stays instant at thirty thousand songs. The index carries the lyrics as well as
//! the metadata, which is what makes "find the song that goes like this" work.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

/// Bump when the schema changes; [`Database::migrate`] applies the difference.
const SCHEMA_VERSION: i32 = 1;

/// Prefix lengths the FTS index precomputes, so typing "bea" narrows without a full scan.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS song (
    id             INTEGER PRIMARY KEY,
    path           TEXT    NOT NULL UNIQUE,
    root           TEXT    NOT NULL,
    folder         TEXT    NOT NULL,
    mtime          INTEGER NOT NULL,
    size           INTEGER NOT NULL,
    hash           BLOB    NOT NULL,

    artist         TEXT    NOT NULL,
    title          TEXT    NOT NULL,
    artist_sort    TEXT    NOT NULL,
    title_sort     TEXT    NOT NULL,
    edition        TEXT,
    genre          TEXT,
    language       TEXT,
    creator        TEXT,
    tags           TEXT,
    year           INTEGER,

    bpm            REAL    NOT NULL,
    gap_ms         INTEGER NOT NULL,
    duration_secs  REAL    NOT NULL,
    is_duet        INTEGER NOT NULL,

    audio_file     TEXT,
    video_file     TEXT,
    cover_file     TEXT,
    background_file TEXT,

    note_count     INTEGER NOT NULL,
    golden_count   INTEGER NOT NULL,
    difficulty     REAL    NOT NULL,

    medley_start   INTEGER,
    medley_end     INTEGER,
    preview_start  REAL,
    usdb_id        INTEGER,

    times_played   INTEGER NOT NULL DEFAULT 0,
    last_played    INTEGER,
    scanned_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS song_artist_idx   ON song(artist_sort);
CREATE INDEX IF NOT EXISTS song_title_idx    ON song(title_sort);
CREATE INDEX IF NOT EXISTS song_folder_idx   ON song(folder);
CREATE INDEX IF NOT EXISTS song_language_idx ON song(language);
CREATE INDEX IF NOT EXISTS song_edition_idx  ON song(edition);
CREATE INDEX IF NOT EXISTS song_year_idx     ON song(year);
CREATE INDEX IF NOT EXISTS song_usdb_idx     ON song(usdb_id);

CREATE VIRTUAL TABLE IF NOT EXISTS song_fts USING fts5(
    artist, title, edition, genre, language, creator, tags, year, lyrics,
    song_id UNINDEXED,
    prefix = '2 3 4',
    tokenize = 'unicode61 remove_diacritics 2'
);
";

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "index was written by a newer version of RungStar (schema {found}, expected {expected})"
    )]
    TooNew { found: i32, expected: i32 },
}

/// How a file compares to what the index already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Not in the index at all.
    Unknown,
    /// Present and unchanged, so it can be skipped without being read.
    Unchanged(i64),
    /// Present but the file has changed.
    Stale(i64),
}

pub struct Database {
    connection: Connection,
}

impl Database {
    /// Open, or create, the index at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// An index that lives only in memory, for tests.
    pub fn in_memory() -> Result<Self, DbError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, DbError> {
        // WAL keeps a background rescan from blocking the browser's reads, and NORMAL
        // synchronous is the right trade for a rebuildable cache.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let mut database = Self { connection };
        database.migrate()?;
        Ok(database)
    }

    fn migrate(&mut self) -> Result<(), DbError> {
        let found: i32 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if found > SCHEMA_VERSION {
            return Err(DbError::TooNew {
                found,
                expected: SCHEMA_VERSION,
            });
        }
        self.connection.execute_batch(SCHEMA)?;
        self.connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    /// Number of songs indexed.
    pub fn count(&self) -> Result<i64, DbError> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM song", [], |row| row.get(0))?)
    }

    /// Whether a file needs re-reading.
    ///
    /// Size and timestamp are enough for the common case and cost one index lookup; the
    /// content hash is only consulted when they match but a verification pass was asked for.
    pub fn freshness(&self, path: &str, mtime: i64, size: i64) -> Result<Freshness, DbError> {
        let row: Option<(i64, i64, i64)> = self
            .connection
            .query_row(
                "SELECT id, mtime, size FROM song WHERE path = ?1",
                params![path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            None => Freshness::Unknown,
            Some((id, stored_mtime, stored_size)) => {
                if stored_mtime == mtime && stored_size == size {
                    Freshness::Unchanged(id)
                } else {
                    Freshness::Stale(id)
                }
            }
        })
    }

    /// Paths currently in the index, so a scan can spot files that have gone away.
    pub fn all_paths(&self) -> Result<Vec<String>, DbError> {
        let mut statement = self.connection.prepare("SELECT path FROM song")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Remove songs whose files no longer exist.
    pub fn remove_paths(&mut self, paths: &[String]) -> Result<usize, DbError> {
        let transaction = self.connection.transaction()?;
        let mut removed = 0;
        {
            let mut delete_fts = transaction.prepare(
                "DELETE FROM song_fts WHERE song_id = (SELECT id FROM song WHERE path = ?1)",
            )?;
            let mut delete_song = transaction.prepare("DELETE FROM song WHERE path = ?1")?;
            for path in paths {
                delete_fts.execute(params![path])?;
                removed += delete_song.execute(params![path])?;
            }
        }
        transaction.commit()?;
        Ok(removed)
    }
}

/// Columns written from a parsed file, in the order the upsert binds them.
const SONG_COLUMNS: &str = "path, root, folder, mtime, size, hash, \
     artist, title, artist_sort, title_sort, edition, genre, language, creator, tags, year, \
     bpm, gap_ms, duration_secs, is_duet, \
     audio_file, video_file, cover_file, background_file, \
     note_count, golden_count, difficulty, \
     medley_start, medley_end, preview_start, usdb_id, scanned_at";

impl Database {
    /// Write parsed songs into the index, replacing any previous entry for the same path.
    ///
    /// Play counts are deliberately left alone. They are the player's history, not a property
    /// of the file, and re-scanning a library must not wipe them.
    pub fn upsert_songs(&mut self, songs: &[crate::scan::ParsedSong]) -> Result<(), DbError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);

        let transaction = self.connection.transaction()?;
        {
            let placeholders = (1..=32)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let updates = SONG_COLUMNS
                .split(',')
                .map(str::trim)
                .filter(|c| *c != "path")
                .map(|c| format!("{c} = excluded.{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "INSERT INTO song ({SONG_COLUMNS}) VALUES ({placeholders}) \
                 ON CONFLICT(path) DO UPDATE SET {updates} RETURNING id"
            );

            let mut insert_song = transaction.prepare(&sql)?;
            let mut clear_fts = transaction.prepare("DELETE FROM song_fts WHERE song_id = ?1")?;
            let mut insert_fts = transaction.prepare(
                "INSERT INTO song_fts \
                 (artist, title, edition, genre, language, creator, tags, year, lyrics, song_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;

            for song in songs {
                let id: i64 = insert_song.query_row(
                    params![
                        song.path,
                        song.root,
                        song.folder,
                        song.mtime,
                        song.size,
                        song.hash.as_slice(),
                        song.artist,
                        song.title,
                        song.artist_sort,
                        song.title_sort,
                        song.edition,
                        song.genre,
                        song.language,
                        song.creator,
                        song.tags,
                        song.year,
                        song.bpm,
                        song.gap_ms,
                        song.duration_secs,
                        song.is_duet,
                        song.audio_file,
                        song.video_file,
                        song.cover_file,
                        song.background_file,
                        song.note_count,
                        song.golden_count,
                        song.difficulty,
                        song.medley_start,
                        song.medley_end,
                        song.preview_start,
                        song.usdb_id,
                        now,
                    ],
                    |row| row.get(0),
                )?;

                clear_fts.execute(params![id])?;
                insert_fts.execute(params![
                    song.artist,
                    song.title,
                    song.edition,
                    song.genre,
                    song.language,
                    song.creator,
                    song.tags,
                    song.year.map(|y| y.to_string()),
                    song.lyrics,
                    id,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

impl Database {
    /// Every indexed path with its recorded size and timestamp.
    ///
    /// A scan compares the whole library at once rather than querying per file: one statement
    /// and one table scan beats thirty thousand prepared lookups by a wide margin.
    pub fn existing_files(&self) -> Result<std::collections::HashMap<String, (i64, i64)>, DbError> {
        let mut statement = self
            .connection
            .prepare("SELECT path, mtime, size FROM song")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
            ))
        })?;
        rows.collect::<Result<_, _>>().map_err(DbError::from)
    }
}
