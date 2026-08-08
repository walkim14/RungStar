//! Party Classic and Party Free: teams, jokers and rounds.
//!
//! Two to three teams of one to four players sing a set number of rounds, and the team with
//! the most round points at the end wins. A team holds five jokers and spends one to reject
//! the song it has been given.
//!
//! Round points, not song points, decide it. UltraStar's scoring — two teams, winner takes
//! one; three teams, three for first and one for second — means a team that is beaten badly
//! all evening loses by the same margin as one beaten narrowly, and that is the point of a
//! party mode. Keeping the raw song scores as well is what makes the standings readable.

use serde::{Deserialize, Serialize};

/// How many teams a party may have.
pub const TEAM_RANGE: std::ops::RangeInclusive<usize> = 2..=3;
/// How many players a team may have.
pub const TEAM_SIZE: std::ops::RangeInclusive<usize> = 1..=4;
/// Jokers per team, as the reference gives.
pub const JOKERS: u8 = 5;

/// One team in a party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub name: String,
    /// The players who take turns singing for it, in order.
    pub players: Vec<String>,
    /// Rejections left.
    pub jokers: u8,
    /// Round points won so far. This is what decides the party.
    pub points: u32,
    /// Song points totalled across the rounds, for the standings.
    pub sung: u32,
    /// Which player sings next, so everybody gets a turn before anybody gets a second.
    next_player: usize,
}

impl Team {
    pub fn new(name: impl Into<String>, players: Vec<String>) -> Self {
        Self {
            name: name.into(),
            players,
            jokers: JOKERS,
            points: 0,
            sung: 0,
            next_player: 0,
        }
    }

    /// Who sings this round. `None` for a team with no players, which cannot happen through
    /// the setup screen but can through a hand-edited save.
    pub fn singer(&self) -> Option<&str> {
        self.players.get(self.next_player).map(String::as_str)
    }
}

/// What the party is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Between rounds: the next team and song are being offered.
    Choosing,
    /// A song is being sung.
    Singing,
    /// Every round has been played.
    Finished,
}

/// One finished round, kept so the final screen can show how it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Played {
    pub song: String,
    /// Song score per team, in team order.
    pub scores: Vec<i32>,
    /// Round points awarded, in team order.
    pub awarded: Vec<u32>,
}

/// A party in progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Party {
    pub teams: Vec<Team>,
    /// How many rounds the party lasts.
    pub rounds: usize,
    /// Rounds already sung.
    pub played: Vec<Played>,
    /// The song offered for the round about to be sung.
    pub offered: Option<String>,
    phase: Phase,
}

impl Party {
    /// Start a party. `rounds` is clamped to at least one, because a party of no rounds is a
    /// menu that closes itself.
    pub fn new(teams: Vec<Team>, rounds: usize) -> Self {
        Self {
            teams,
            rounds: rounds.max(1),
            played: Vec::new(),
            offered: None,
            phase: Phase::Choosing,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Which round is being played, counting from one.
    pub fn round(&self) -> usize {
        (self.played.len() + 1).min(self.rounds)
    }

    /// Whose turn it is to be offered a song: the teams take rounds in order.
    pub fn team_up(&self) -> usize {
        if self.teams.is_empty() {
            return 0;
        }
        self.played.len() % self.teams.len()
    }

    /// Offer a song for this round.
    pub fn offer(&mut self, song: impl Into<String>) {
        if self.phase == Phase::Finished {
            return;
        }
        self.offered = Some(song.into());
        self.phase = Phase::Choosing;
    }

    /// Whether the team up now can still reject what it has been offered.
    pub fn can_reject(&self) -> bool {
        self.phase == Phase::Choosing
            && self.offered.is_some()
            && self.teams.get(self.team_up()).is_some_and(|t| t.jokers > 0)
    }

    /// Spend a joker on the song offered, so another can be drawn.
    ///
    /// Returns whether one was spent. Refusing when there are none left rather than going into
    /// debt is the whole reason a joker count is worth having.
    pub fn reject(&mut self) -> bool {
        if !self.can_reject() {
            return false;
        }
        let team = self.team_up();
        self.teams[team].jokers -= 1;
        self.offered = None;
        true
    }

    /// Accept the offered song and start singing it.
    pub fn accept(&mut self) -> Option<&str> {
        if self.phase != Phase::Choosing || self.offered.is_none() {
            return None;
        }
        self.phase = Phase::Singing;
        self.offered.as_deref()
    }

    /// Record a finished round. `scores` is one song score per team, in team order.
    ///
    /// Everybody's turn advances, not only the team that sang: in Party Classic every team
    /// sings every round, one player each, and the singer rotates within the team.
    pub fn finish_round(&mut self, scores: &[i32]) {
        if self.phase == Phase::Finished {
            return;
        }
        let awarded = award(scores, self.teams.len());
        for (index, team) in self.teams.iter_mut().enumerate() {
            team.points += awarded.get(index).copied().unwrap_or(0);
            team.sung += scores.get(index).copied().unwrap_or(0).max(0) as u32;
            if !team.players.is_empty() {
                team.next_player = (team.next_player + 1) % team.players.len();
            }
        }
        self.played.push(Played {
            song: self.offered.take().unwrap_or_default(),
            scores: scores.to_vec(),
            awarded,
        });
        self.phase = if self.played.len() >= self.rounds {
            Phase::Finished
        } else {
            Phase::Choosing
        };
    }

    /// Team indices in finishing order, best first.
    ///
    /// Round points decide it; the song total breaks a tie, because two teams on the same
    /// round points have genuinely sung differently and the party wants a winner.
    pub fn standings(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.teams.len()).collect();
        order.sort_by(|a, b| {
            let (a, b) = (&self.teams[*a], &self.teams[*b]);
            b.points.cmp(&a.points).then(b.sung.cmp(&a.sung))
        });
        order
    }

    /// The winning team, or `None` while the party is still going or if it is a dead heat on
    /// both points and total.
    pub fn winner(&self) -> Option<usize> {
        if self.phase != Phase::Finished {
            return None;
        }
        let order = self.standings();
        let (first, second) = (order.first().copied()?, order.get(1).copied());
        match second {
            Some(second)
                if self.teams[first].points == self.teams[second].points
                    && self.teams[first].sung == self.teams[second].sung =>
            {
                None
            }
            _ => Some(first),
        }
    }
}

/// Round points for one round's song scores.
///
/// Two teams: one point to the winner. Three: three to the first and one to the second. Ties
/// share the placing — both get the higher award and nobody gets the lower, which is what
/// happens on a podium and what the reference does not define.
fn award(scores: &[i32], teams: usize) -> Vec<u32> {
    let mut awarded = vec![0; teams];
    if teams < 2 {
        return awarded;
    }
    let best = scores.iter().copied().max().unwrap_or(0);
    let winners: Vec<usize> = (0..teams)
        .filter(|i| scores.get(*i).copied().unwrap_or(0) == best)
        .collect();
    let first = if teams >= 3 { 3 } else { 1 };
    for index in &winners {
        awarded[*index] = first;
    }
    if teams < 3 || winners.len() > 1 {
        return awarded;
    }
    // Second place, only when there is an undisputed first to be second to.
    let runner_up = (0..teams)
        .filter(|i| !winners.contains(i))
        .map(|i| (i, scores.get(i).copied().unwrap_or(0)))
        .max_by_key(|(_, score)| *score);
    if let Some((_, second_score)) = runner_up {
        for (index, award) in awarded.iter_mut().enumerate() {
            if !winners.contains(&index) && scores.get(index).copied().unwrap_or(0) == second_score
            {
                *award = 1;
            }
        }
    }
    awarded
}
