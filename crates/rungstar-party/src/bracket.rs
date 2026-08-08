//! A single-elimination tournament for 2, 4, 8 or 16 players.
//!
//! Powers of two only, as the reference has it. A bracket that is not a power of two needs
//! byes, and a bye is a player who advances without singing — at a party that reads as the
//! game skipping their turn.

use serde::{Deserialize, Serialize};

/// Sizes a bracket can be.
pub const SIZES: [usize; 4] = [2, 4, 8, 16];

/// One match: two players, and who won when it has been sung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Match {
    /// Indices into the bracket's players.
    pub left: usize,
    pub right: usize,
    /// `0` for the left player, `1` for the right. `None` until it has been played.
    pub won_by: Option<u8>,
    /// The song scores, so the bracket can be read back afterwards.
    pub scores: Option<(i32, i32)>,
}

impl Match {
    pub fn winner(&self) -> Option<usize> {
        match self.won_by {
            Some(0) => Some(self.left),
            Some(1) => Some(self.right),
            _ => None,
        }
    }

    pub fn loser(&self) -> Option<usize> {
        match self.won_by {
            Some(0) => Some(self.right),
            Some(1) => Some(self.left),
            _ => None,
        }
    }
}

/// A single-elimination tournament.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bracket {
    pub players: Vec<String>,
    /// One list of matches per round, first round first.
    pub rounds: Vec<Vec<Match>>,
}

/// Why a bracket could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BracketError {
    /// The player count is not 2, 4, 8 or 16.
    NotAPowerOfTwo(usize),
}

impl std::fmt::Display for BracketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAPowerOfTwo(n) => write!(
                f,
                "a tournament needs 2, 4, 8 or 16 players, not {n} \u{2014} nobody should have \
                 to sit out a round"
            ),
        }
    }
}

impl std::error::Error for BracketError {}

impl Bracket {
    /// Build a bracket over these players, in the order given.
    ///
    /// The order is the seeding and is left alone. Shuffling belongs to whoever calls this:
    /// a party wants it random, and a test wants it not to be.
    pub fn new(players: Vec<String>) -> Result<Self, BracketError> {
        let count = players.len();
        if !SIZES.contains(&count) {
            return Err(BracketError::NotAPowerOfTwo(count));
        }
        let first: Vec<Match> = (0..count / 2)
            .map(|i| Match {
                left: i * 2,
                right: i * 2 + 1,
                won_by: None,
                scores: None,
            })
            .collect();
        Ok(Self {
            players,
            rounds: vec![first],
        })
    }

    /// How many rounds the whole tournament takes.
    pub fn total_rounds(&self) -> usize {
        self.players.len().trailing_zeros() as usize
    }

    /// What this round is called, so a screen does not have to count.
    pub fn round_name(&self, round: usize) -> &'static str {
        match self.total_rounds().saturating_sub(round) {
            0 | 1 => "Final",
            2 => "Semi-final",
            3 => "Quarter-final",
            _ => "First round",
        }
    }

    /// The next match waiting to be sung, as `(round, index)`.
    pub fn next_match(&self) -> Option<(usize, usize)> {
        for (round, matches) in self.rounds.iter().enumerate() {
            if let Some(index) = matches.iter().position(|m| m.won_by.is_none()) {
                return Some((round, index));
            }
        }
        None
    }

    /// Record who won a match, and build the next round once this one is complete.
    ///
    /// A draw is not allowed to stand: somebody has to go through, so the higher score wins
    /// and an exact tie goes to the left. Asking two people to sing it again is the right
    /// answer at a real tournament and the wrong one at half past eleven at a party.
    pub fn report(&mut self, round: usize, index: usize, scores: (i32, i32)) {
        let Some(fixture) = self.rounds.get_mut(round).and_then(|r| r.get_mut(index)) else {
            return;
        };
        fixture.scores = Some(scores);
        fixture.won_by = Some(u8::from(scores.1 > scores.0));
        self.grow();
    }

    /// Add the next round if the last one is finished and there is another to play.
    fn grow(&mut self) {
        let Some(last) = self.rounds.last() else {
            return;
        };
        if last.len() < 2 || last.iter().any(|m| m.won_by.is_none()) {
            return;
        }
        let winners: Vec<usize> = last.iter().filter_map(Match::winner).collect();
        let next: Vec<Match> = winners
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| Match {
                left: pair[0],
                right: pair[1],
                won_by: None,
                scores: None,
            })
            .collect();
        if !next.is_empty() {
            self.rounds.push(next);
        }
    }

    /// The champion, once the final has been sung.
    pub fn champion(&self) -> Option<usize> {
        let last = self.rounds.last()?;
        if last.len() != 1 {
            return None;
        }
        last[0].winner()
    }

    pub fn is_finished(&self) -> bool {
        self.champion().is_some()
    }

    /// Everybody in finishing order, champion first.
    ///
    /// Losing later is placing higher, which is all a single-elimination bracket actually
    /// knows: two players knocked out in the same round are not ranked against each other.
    pub fn placings(&self) -> Vec<usize> {
        let mut order: Vec<usize> = Vec::new();
        if let Some(champion) = self.champion() {
            order.push(champion);
        }
        for matches in self.rounds.iter().rev() {
            for fixture in matches {
                if let Some(loser) = fixture.loser() {
                    if !order.contains(&loser) {
                        order.push(loser);
                    }
                }
            }
        }
        order
    }

    pub fn name(&self, player: usize) -> &str {
        self.players.get(player).map_or("", String::as_str)
    }
}
