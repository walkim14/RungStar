//! Who is singing, what they scored, and what that adds up to.
//!
//! Kept in its own database rather than beside the song index, because the two have opposite
//! lifetimes: the index is a cache of what is on disk and can be thrown away and rebuilt at
//! any time, while this is the only copy of something nobody can reproduce. Deleting the
//! index to fix a browsing problem must not cost anybody their scores.
//!
//! **A score is keyed on the artist and title, not on a library row.** Song ids are assigned
//! by the scanner and change when the index is rebuilt, so keying on one would quietly orphan
//! every score the first time somebody rebuilt it. UltraStar keys the same way, which is also
//! what makes importing its database a straight match rather than a guess.

pub mod import;
pub mod stats;

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

pub use import::{import_ultrastar, ImportReport};
pub use stats::{ArtistTally, PlayerTally, ScoreEntry, SongTally};

/// Why the profile store could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("a player must have a name")]
    EmptyName,
    #[error("there is already a player called {0}")]
    DuplicateName(String),
}

/// Someone who sings.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    pub id: i64,
    pub name: String,
    /// A file beside the profile database, or `None` for the generated initial.
    pub avatar: Option<String>,
    /// Colour index into the theme's player colours, so a regular keeps their colour.
    pub colour: u8,
    /// Unix seconds.
    pub created_at: i64,
}

/// One completed song, for one singer.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub player_id: i64,
    pub artist: String,
    pub title: String,
    /// Easy, Medium or Hard as UltraStar numbers them: 0, 1, 2.
    pub difficulty: u8,
    pub points: i32,
    pub notes: i32,
    pub golden: i32,
    pub line_bonus: i32,
    /// Unix seconds.
    pub sung_at: i64,
}

/// How songs are identified across a rebuilt index.
///
/// Case-folded and trimmed, because a library is full of the same song with different
/// capitalisation and stray spaces, and a highscore that does not show up because of a capital
/// letter is worse than no highscore.
pub fn song_key(artist: &str, title: &str) -> String {
    format!(
        "{}\u{1}{}",
        artist.trim().to_lowercase(),
        title.trim().to_lowercase()
    )
}

const SCHEMA_VERSION: i32 = 1;

/// The store.
pub struct Profiles {
    connection: Connection,
}

impl Profiles {
    /// Open or create the store.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        if let Some(parent) = path.as_ref().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self::from_connection(Connection::open(path)?)
    }

    /// An in-memory store, for tests.
    pub fn in_memory() -> Result<Self, ProfileError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, ProfileError> {
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), ProfileError> {
        let version: i32 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS player (
                 id         INTEGER PRIMARY KEY,
                 name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
                 avatar     TEXT,
                 colour     INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS score (
                 id         INTEGER PRIMARY KEY,
                 player_id  INTEGER NOT NULL REFERENCES player(id) ON DELETE CASCADE,
                 song_key   TEXT NOT NULL,
                 artist     TEXT NOT NULL,
                 title      TEXT NOT NULL,
                 difficulty INTEGER NOT NULL,
                 points     INTEGER NOT NULL,
                 notes      INTEGER NOT NULL DEFAULT 0,
                 golden     INTEGER NOT NULL DEFAULT 0,
                 line_bonus INTEGER NOT NULL DEFAULT 0,
                 sung_at    INTEGER NOT NULL
             );
             -- The two questions asked constantly: this song's table, and this player's
             -- history. Both would be a full scan otherwise, on the one table that only grows.
             CREATE INDEX IF NOT EXISTS score_by_song ON score(song_key, difficulty, points DESC);
             CREATE INDEX IF NOT EXISTS score_by_player ON score(player_id, sung_at DESC);
             CREATE TABLE IF NOT EXISTS favourite (
                 player_id INTEGER NOT NULL REFERENCES player(id) ON DELETE CASCADE,
                 song_key  TEXT NOT NULL,
                 PRIMARY KEY (player_id, song_key)
             );",
        )?;
        self.connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    /// Every player, oldest first so the order does not shuffle as names change.
    pub fn players(&self) -> Result<Vec<Player>, ProfileError> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, avatar, colour, created_at FROM player ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Player {
                id: row.get(0)?,
                name: row.get(1)?,
                avatar: row.get(2)?,
                colour: row.get::<_, i64>(3)? as u8,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn player(&self, id: i64) -> Result<Option<Player>, ProfileError> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, avatar, colour, created_at FROM player WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Player {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        avatar: row.get(2)?,
                        colour: row.get::<_, i64>(3)? as u8,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Find a player by name, case-insensitively.
    ///
    /// Used by the importer and by anything that has a name rather than an id — UltraStar
    /// records the singer's name in the score row and nothing else.
    pub fn player_by_name(&self, name: &str) -> Result<Option<Player>, ProfileError> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, avatar, colour, created_at FROM player \
                 WHERE name = ?1 COLLATE NOCASE",
                params![name.trim()],
                |row| {
                    Ok(Player {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        avatar: row.get(2)?,
                        colour: row.get::<_, i64>(3)? as u8,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Add a player, or return the one already called that.
    ///
    /// Idempotent because the importer meets the same name over and over, and because two
    /// people called the same thing is a mistake rather than a feature.
    pub fn ensure_player(&mut self, name: &str, now: i64) -> Result<Player, ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if let Some(existing) = self.player_by_name(name)? {
            return Ok(existing);
        }
        // The next colour in rotation, so two players never start with the same one.
        let colour: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM player", [], |row| row.get(0))?;
        self.connection.execute(
            "INSERT INTO player (name, colour, created_at) VALUES (?1, ?2, ?3)",
            params![name, colour % 6, now],
        )?;
        let id = self.connection.last_insert_rowid();
        Ok(Player {
            id,
            name: name.to_owned(),
            avatar: None,
            colour: (colour % 6) as u8,
            created_at: now,
        })
    }

    pub fn rename_player(&mut self, id: i64, name: &str) -> Result<(), ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if let Some(other) = self.player_by_name(name)? {
            if other.id != id {
                return Err(ProfileError::DuplicateName(name.to_owned()));
            }
        }
        self.connection.execute(
            "UPDATE player SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    pub fn set_colour(&mut self, id: i64, colour: u8) -> Result<(), ProfileError> {
        self.connection.execute(
            "UPDATE player SET colour = ?1 WHERE id = ?2",
            params![i64::from(colour), id],
        )?;
        Ok(())
    }

    pub fn set_avatar(&mut self, id: i64, avatar: Option<&str>) -> Result<(), ProfileError> {
        self.connection.execute(
            "UPDATE player SET avatar = ?1 WHERE id = ?2",
            params![avatar, id],
        )?;
        Ok(())
    }

    /// Remove a player and everything they did.
    ///
    /// Their scores go with them, which is what somebody deleting a profile means. The
    /// statistics are recomputed from what remains rather than kept as totals, so nothing is
    /// left claiming they sang.
    pub fn remove_player(&mut self, id: i64) -> Result<(), ProfileError> {
        self.connection
            .execute("DELETE FROM player WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Record a finished song.
    pub fn record(&mut self, score: &Score) -> Result<(), ProfileError> {
        self.connection.execute(
            "INSERT INTO score
                 (player_id, song_key, artist, title, difficulty, points, notes, golden,
                  line_bonus, sung_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                score.player_id,
                song_key(&score.artist, &score.title),
                score.artist,
                score.title,
                i64::from(score.difficulty),
                score.points,
                score.notes,
                score.golden,
                score.line_bonus,
                score.sung_at,
            ],
        )?;
        Ok(())
    }

    /// The best scores for a song, highest first.
    ///
    /// Difficulties are kept apart, because a score on Easy is not comparable with one on
    /// Hard and putting them in one table would make Easy always win.
    pub fn best_for(
        &self,
        artist: &str,
        title: &str,
        difficulty: Option<u8>,
        limit: usize,
    ) -> Result<Vec<ScoreEntry>, ProfileError> {
        let key = song_key(artist, title);
        let mut sql = String::from(
            "SELECT player.name, score.points, score.difficulty, score.sung_at
             FROM score JOIN player ON player.id = score.player_id
             WHERE score.song_key = ?1",
        );
        if difficulty.is_some() {
            sql.push_str(" AND score.difficulty = ?2");
        }
        sql.push_str(" ORDER BY score.points DESC, score.sung_at ASC");
        sql.push_str(&format!(" LIMIT {limit}"));

        let mut statement = self.connection.prepare(&sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(ScoreEntry {
                player: row.get(0)?,
                points: row.get(1)?,
                difficulty: row.get::<_, i64>(2)? as u8,
                sung_at: row.get(3)?,
                artist: artist.to_owned(),
                title: title.to_owned(),
            })
        };
        let rows = match difficulty {
            Some(level) => statement.query_map(params![key, i64::from(level)], map)?,
            None => statement.query_map(params![key], map)?,
        };
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether a player has marked a song.
    pub fn is_favourite(
        &self,
        player_id: i64,
        artist: &str,
        title: &str,
    ) -> Result<bool, ProfileError> {
        let key = song_key(artist, title);
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM favourite WHERE player_id = ?1 AND song_key = ?2",
            params![player_id, key],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Mark or unmark a song. Returns whether it is now a favourite.
    pub fn toggle_favourite(
        &mut self,
        player_id: i64,
        artist: &str,
        title: &str,
    ) -> Result<bool, ProfileError> {
        let key = song_key(artist, title);
        if self.is_favourite(player_id, artist, title)? {
            self.connection.execute(
                "DELETE FROM favourite WHERE player_id = ?1 AND song_key = ?2",
                params![player_id, key],
            )?;
            Ok(false)
        } else {
            self.connection.execute(
                "INSERT INTO favourite (player_id, song_key) VALUES (?1, ?2)",
                params![player_id, key],
            )?;
            Ok(true)
        }
    }

    /// Every song key any player has marked, for filtering the browser.
    pub fn favourite_keys(&self, player_id: Option<i64>) -> Result<Vec<String>, ProfileError> {
        let mut statement = match player_id {
            Some(_) => self
                .connection
                .prepare("SELECT song_key FROM favourite WHERE player_id = ?1")?,
            None => self
                .connection
                .prepare("SELECT DISTINCT song_key FROM favourite")?,
        };
        // Collected inside each arm: the two `query_map` calls have different closure types,
        // so they cannot be the arms of one expression.
        let keys: Vec<String> = match player_id {
            Some(id) => statement
                .query_map(params![id], |row| row.get(0))?
                .collect::<Result<_, _>>()?,
            None => statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?,
        };
        Ok(keys)
    }

    /// How many scores are stored, for the importer to report against.
    pub fn score_count(&self) -> Result<usize, ProfileError> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM score", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}
