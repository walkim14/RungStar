//! The four things people want to know at the end of a party.
//!
//! The same four UltraStar Deluxe has, because they are the right four and a returning player
//! should find what they are used to: the best scores, the best singers, the most-sung songs,
//! and the most-sung artists.
//!
//! Every one is computed from the score rows rather than kept as a running total. Totals drift
//! — a deleted player leaves their contribution behind — and at the scale of one household's
//! singing there is nothing to gain by keeping them.

use rusqlite::params;

use crate::{ProfileError, Profiles};

/// One score, with who and what.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreEntry {
    pub player: String,
    pub artist: String,
    pub title: String,
    pub difficulty: u8,
    pub points: i32,
    /// Unix seconds.
    pub sung_at: i64,
}

/// A singer, and how they have done.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerTally {
    pub name: String,
    pub songs: i64,
    pub average: i32,
    pub best: i32,
}

/// A song, and how often it has been sung.
#[derive(Debug, Clone, PartialEq)]
pub struct SongTally {
    pub artist: String,
    pub title: String,
    pub times: i64,
    pub best: i32,
}

/// An artist, and how often anything of theirs has been sung.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtistTally {
    pub artist: String,
    pub times: i64,
}

/// How to order a statistics view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// Best or most first, which is what everybody actually wants to see.
    #[default]
    Best,
    /// Worst or least first. UltraStar offers this and it is genuinely funny at a party.
    Worst,
}

impl Order {
    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Best first",
            Self::Worst => "Worst first",
        }
    }

    pub fn flip(self) -> Self {
        match self {
            Self::Best => Self::Worst,
            Self::Worst => Self::Best,
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Self::Best => "DESC",
            Self::Worst => "ASC",
        }
    }
}

impl Profiles {
    /// The highest scores anybody has managed, on any song.
    pub fn best_scores(&self, order: Order, limit: usize) -> Result<Vec<ScoreEntry>, ProfileError> {
        let sql = format!(
            "SELECT player.name, score.artist, score.title, score.difficulty, score.points,
                    score.sung_at
             FROM score JOIN player ON player.id = score.player_id
             ORDER BY score.points {}, score.sung_at ASC
             LIMIT ?1",
            order.sql()
        );
        let mut statement = self.connection().prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(ScoreEntry {
                player: row.get(0)?,
                artist: row.get(1)?,
                title: row.get(2)?,
                difficulty: row.get::<_, i64>(3)? as u8,
                points: row.get(4)?,
                sung_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Singers ranked by their average, which is the fair way round.
    ///
    /// Ranking by total would just rank by who sang most, and one lucky song would not put
    /// somebody top of a list they have no business being on.
    pub fn best_singers(
        &self,
        order: Order,
        limit: usize,
    ) -> Result<Vec<PlayerTally>, ProfileError> {
        let sql = format!(
            "SELECT player.name, COUNT(*), CAST(AVG(score.points) AS INTEGER),
                    MAX(score.points)
             FROM score JOIN player ON player.id = score.player_id
             GROUP BY player.id
             ORDER BY AVG(score.points) {}
             LIMIT ?1",
            order.sql()
        );
        let mut statement = self.connection().prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(PlayerTally {
                name: row.get(0)?,
                songs: row.get(1)?,
                average: row.get(2)?,
                best: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The songs that get picked.
    pub fn most_sung_songs(
        &self,
        order: Order,
        limit: usize,
    ) -> Result<Vec<SongTally>, ProfileError> {
        // Grouped on the folded key so the same song written two ways counts once, but shown
        // with the spelling most recently used — which is the one on disk now.
        let sql = format!(
            "SELECT artist, title, COUNT(*) AS times, MAX(points)
             FROM score
             GROUP BY song_key
             ORDER BY times {}, artist ASC
             LIMIT ?1",
            order.sql()
        );
        let mut statement = self.connection().prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(SongTally {
                artist: row.get(0)?,
                title: row.get(1)?,
                times: row.get(2)?,
                best: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The artists that get picked, across all their songs.
    pub fn most_sung_artists(
        &self,
        order: Order,
        limit: usize,
    ) -> Result<Vec<ArtistTally>, ProfileError> {
        let sql = format!(
            "SELECT artist, COUNT(*) AS times
             FROM score
             GROUP BY LOWER(TRIM(artist))
             ORDER BY times {}, artist ASC
             LIMIT ?1",
            order.sql()
        );
        let mut statement = self.connection().prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(ArtistTally {
                artist: row.get(0)?,
                times: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One player's own songs, most recent first.
    pub fn history(&self, player_id: i64, limit: usize) -> Result<Vec<ScoreEntry>, ProfileError> {
        let mut statement = self.connection().prepare(
            "SELECT player.name, score.artist, score.title, score.difficulty, score.points,
                    score.sung_at
             FROM score JOIN player ON player.id = score.player_id
             WHERE score.player_id = ?1
             ORDER BY score.sung_at DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![player_id, limit as i64], |row| {
            Ok(ScoreEntry {
                player: row.get(0)?,
                artist: row.get(1)?,
                title: row.get(2)?,
                difficulty: row.get::<_, i64>(3)? as u8,
                points: row.get(4)?,
                sung_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// Which statistics view is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Scores,
    Singers,
    Songs,
    Artists,
}

impl View {
    pub const ALL: [View; 4] = [View::Scores, View::Singers, View::Songs, View::Artists];

    pub fn title(self) -> &'static str {
        match self {
            Self::Scores => "Scores",
            Self::Singers => "Singers",
            Self::Songs => "Songs",
            Self::Artists => "Artists",
        }
    }

    /// What the two columns mean, so a table of numbers is readable without guessing.
    pub fn columns(self) -> (&'static str, &'static str) {
        match self {
            Self::Scores => ("Song", "Score"),
            Self::Singers => ("Singer", "Average"),
            Self::Songs => ("Song", "Times sung"),
            Self::Artists => ("Artist", "Times sung"),
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        self.next().next().next()
    }
}
