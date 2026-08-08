//! Reading an existing `Ultrastar.db`, so nobody loses their history by moving.
//!
//! UltraStar keeps scores in two tables: `us_songs` maps an id to an artist and title, and
//! `us_scores` holds one row per score pointing at that id. Both are SQLite, so this is a
//! plain read rather than a parse — and because scores are keyed on the exact artist and
//! title pair, which is how they are keyed here too, the match is direct.
//!
//! The import is **additive and repeatable**. Running it twice must not double anybody's
//! history, and it must never delete a score that was earned here, so a row is skipped when an
//! identical one already exists rather than the table being replaced.

use std::path::Path;

use rusqlite::{params, Connection, OpenFlags};

use crate::{song_key, ProfileError, Profiles};

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Players created because a score named them.
    pub players_added: usize,
    pub scores_added: usize,
    /// Rows that were already here, matched on player, song, difficulty, score and date.
    pub scores_skipped: usize,
    /// Rows pointing at a song id `us_songs` does not have.
    pub orphaned: usize,
}

impl ImportReport {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// A sentence for the status line.
    ///
    /// Orphans are named even when nothing was imported. A file whose every row points at a
    /// missing song would otherwise report "no scores found", which is true and useless — the
    /// interesting part is that they were found and could not be placed.
    pub fn summary(&self) -> String {
        if self.scores_added == 0 && self.scores_skipped == 0 {
            return match self.orphaned {
                0 => "no scores found to import".to_owned(),
                1 => "found 1 score with no song to attach it to".to_owned(),
                n => format!("found {n} scores with no song to attach them to"),
            };
        }
        let mut parts = vec![format!("{} scores", self.scores_added)];
        if self.players_added > 0 {
            parts.push(format!("{} players", self.players_added));
        }
        if self.scores_skipped > 0 {
            parts.push(format!("{} already here", self.scores_skipped));
        }
        if self.orphaned > 0 {
            parts.push(format!("{} with no song", self.orphaned));
        }
        format!("imported {}", parts.join(", "))
    }
}

/// Why an import could not be done.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("opening {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },
    #[error("{0} does not look like an UltraStar database: no us_scores table")]
    NotUltrastar(String),
    #[error("reading scores: {0}")]
    Read(#[from] rusqlite::Error),
    #[error(transparent)]
    Profile(#[from] ProfileError),
}

/// One row as UltraStar stores it.
struct Row {
    player: String,
    artist: String,
    title: String,
    difficulty: u8,
    points: i32,
    /// Unix seconds. UltraStar leaves this null on old rows.
    date: Option<i64>,
}

/// Import every score from an `Ultrastar.db`.
pub fn import_ultrastar(
    profiles: &mut Profiles,
    path: impl AsRef<Path>,
    now: i64,
) -> Result<ImportReport, ImportError> {
    let path = path.as_ref();
    let shown = path.display().to_string();

    // Read-only: this is somebody's other game and we are a guest in it.
    let source =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
            ImportError::Open {
                path: shown.clone(),
                source,
            }
        })?;

    let has_scores: i64 = source
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'us_scores'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_scores == 0 {
        return Err(ImportError::NotUltrastar(shown));
    }

    // A left join rather than an inner one, so a score whose song row is missing is counted
    // and reported instead of silently vanishing.
    let mut statement = source.prepare(
        "SELECT s.Player, g.Artist, g.Title, s.Difficulty, s.Score, s.Date
         FROM us_scores s LEFT JOIN us_songs g ON g.ID = s.SongID",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0).unwrap_or_default(),
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3).unwrap_or(1),
            row.get::<_, i64>(4).unwrap_or(0),
            row.get::<_, Option<i64>>(5).unwrap_or(None),
        ))
    })?;

    let mut report = ImportReport::default();
    let mut pending: Vec<Row> = Vec::new();
    for row in rows {
        let (player, artist, title, difficulty, points, date) = row?;
        let (Some(artist), Some(title)) = (artist, title) else {
            report.orphaned += 1;
            continue;
        };
        if player.trim().is_empty() {
            report.orphaned += 1;
            continue;
        }
        pending.push(Row {
            player,
            artist,
            title,
            // UltraStar numbers difficulty 0, 1, 2 as we do; anything else is a corrupt row.
            difficulty: difficulty.clamp(0, 2) as u8,
            points: points.clamp(0, 100_000) as i32,
            date,
        });
    }

    // One transaction: importing thousands of rows one commit at a time takes minutes, and a
    // half-finished import is worse than none.
    let existing_players = profiles.players()?.len();
    let transaction = profiles.connection_mut().transaction()?;
    {
        let mut find_player =
            transaction.prepare("SELECT id FROM player WHERE name = ?1 COLLATE NOCASE")?;
        let mut add_player = transaction
            .prepare("INSERT INTO player (name, colour, created_at) VALUES (?1, ?2, ?3)")?;
        let mut count_players = transaction.prepare("SELECT COUNT(*) FROM player")?;
        let mut duplicate = transaction.prepare(
            "SELECT COUNT(*) FROM score
             WHERE player_id = ?1 AND song_key = ?2 AND difficulty = ?3 AND points = ?4
               AND sung_at = ?5",
        )?;
        let mut insert = transaction.prepare(
            "INSERT INTO score
                 (player_id, song_key, artist, title, difficulty, points, notes, golden,
                  line_bonus, sung_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 0, ?7)",
        )?;

        for row in &pending {
            let name = row.player.trim();
            let player_id: i64 = match find_player.query_row(params![name], |r| r.get(0)).ok() {
                Some(id) => id,
                None => {
                    let colour: i64 = count_players.query_row([], |r| r.get(0))?;
                    add_player.execute(params![name, colour % 6, now])?;
                    transaction.last_insert_rowid()
                }
            };

            let key = song_key(&row.artist, &row.title);
            // UltraStar's own date, when it has one. Falling back to the import time means a
            // second import of the same file would look like new rows, so the fallback is a
            // stable zero rather than "now".
            let sung_at = row.date.unwrap_or(0);
            let already: i64 = duplicate.query_row(
                params![
                    player_id,
                    key,
                    i64::from(row.difficulty),
                    row.points,
                    sung_at
                ],
                |r| r.get(0),
            )?;
            if already > 0 {
                report.scores_skipped += 1;
                continue;
            }
            insert.execute(params![
                player_id,
                key,
                row.artist,
                row.title,
                i64::from(row.difficulty),
                row.points,
                sung_at,
            ])?;
            report.scores_added += 1;
        }
    }
    transaction.commit()?;

    report.players_added = profiles.players()?.len().saturating_sub(existing_players);
    Ok(report)
}

/// Where UltraStar Deluxe keeps its database, so the import can offer a path rather than ask
/// somebody to find one.
pub fn likely_ultrastar_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            paths.push(Path::new(&appdata).join("ultrastardx").join("Ultrastar.db"));
            paths.push(
                Path::new(&appdata)
                    .join("UltraStar Deluxe")
                    .join("Ultrastar.db"),
            );
        }
    } else if let Ok(home) = std::env::var("HOME") {
        paths.push(Path::new(&home).join(".ultrastardx").join("Ultrastar.db"));
        paths.push(
            Path::new(&home)
                .join(".local/share/ultrastardx")
                .join("Ultrastar.db"),
        );
    }
    paths
}
