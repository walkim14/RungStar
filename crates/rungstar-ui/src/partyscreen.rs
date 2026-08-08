//! Party and tournament: setting one up, and running it round by round.
//!
//! One screen with four stages rather than the reference's ten separate ones (Party Options,
//! Player, Rounds, NewRound, Score, Win, then four more for Tournament). They are the same
//! four questions — who is playing, what is being sung, how did that go, who won — and having
//! them as one state machine is why the party and the tournament can share it.
//!
//! The screen holds no rules. Who sings next, what a joker costs and who has won are the party
//! crate's to decide; this draws the answer and reports which button was pressed.

use rungstar_party::{Bracket, Challenge, Party};

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Rect};
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// What kind of evening this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// Teams, a song offered to each in turn, five jokers apiece.
    #[default]
    Classic,
    /// Teams, but everybody picks their own song.
    Free,
    /// Single elimination, one on one.
    Tournament,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Classic, Kind::Free, Kind::Tournament];

    pub fn name(self) -> &'static str {
        match self {
            Self::Classic => "Party",
            Self::Free => "Party, free choice",
            Self::Tournament => "Tournament",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Classic => {
                "Teams take turns. Each is offered a song and holds five jokers to refuse one."
            }
            Self::Free => "Teams take turns and each picks its own song. No jokers needed.",
            Self::Tournament => {
                "One against one, knocked out until somebody is left. Two, four, eight or sixteen."
            }
        }
    }

    pub fn is_tournament(self) -> bool {
        self == Self::Tournament
    }
}

/// Which stage the screen is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    /// Choosing the kind, the size and the challenge.
    #[default]
    Setup,
    /// Between songs: whose turn it is and what they have been given.
    Round,
    /// The whole thing is over.
    Finished,
}

/// What the screen wants the application to do.
#[derive(Debug, Clone, PartialEq)]
pub enum PartyOutcome {
    None,
    /// Build a party of this shape and start it.
    Begin,
    /// Sing the song that is on offer.
    Sing,
    /// Spend a joker and draw another song.
    Reroll,
    /// Open the browser so this round's song can be chosen by hand.
    Choose,
    /// Throw the party away and go back to the menu.
    Leave,
}

/// One row of the setup stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Kind,
    Size,
    Rounds,
    Challenge,
    Begin,
}

impl Row {
    const ALL: [Row; 5] = [
        Row::Kind,
        Row::Size,
        Row::Rounds,
        Row::Challenge,
        Row::Begin,
    ];
}

/// The party screen.
pub struct PartyScreen {
    pub stage: Stage,
    pub kind: Kind,
    /// Teams for a party, players for a tournament.
    pub size: usize,
    pub rounds: usize,
    /// Index into [`Challenge::ALL`].
    pub challenge: usize,
    /// The party being played, once one has begun.
    pub party: Option<Party>,
    /// The bracket being played, for a tournament.
    pub bracket: Option<Bracket>,
    /// The names available to fill teams and brackets with, from the saved profiles.
    pub pool: Vec<String>,
    /// What is on offer this round, when there is something.
    pub offered: Option<String>,
    pub gamepad: bool,
    cursor: usize,
    regions: Vec<(Rect, usize)>,
}

impl Default for PartyScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl PartyScreen {
    pub fn new() -> Self {
        Self {
            stage: Stage::Setup,
            kind: Kind::default(),
            size: 2,
            rounds: 4,
            challenge: 0,
            party: None,
            bracket: None,
            pool: Vec::new(),
            offered: None,
            gamepad: false,
            cursor: 0,
            regions: Vec::new(),
        }
    }

    pub fn challenge(&self) -> &'static Challenge {
        &Challenge::ALL[self.challenge.min(Challenge::ALL.len() - 1)]
    }

    /// How many teams or players the chosen kind allows.
    ///
    /// A tournament is a power of two because a bracket that is not needs byes, and a bye is
    /// somebody who advances without singing — at a party that reads as a skipped turn.
    pub fn sizes(&self) -> &'static [usize] {
        if self.kind.is_tournament() {
            &[2, 4, 8, 16]
        } else {
            &[2, 3]
        }
    }

    fn clamp_size(&mut self) {
        let sizes = self.sizes();
        if !sizes.contains(&self.size) {
            self.size = sizes[0];
        }
    }

    /// Whether there are enough saved profiles for the shape chosen.
    pub fn short_by(&self) -> usize {
        self.size.saturating_sub(self.pool.len())
    }

    fn step(&mut self, delta: isize) {
        match Row::ALL[self.cursor.min(Row::ALL.len() - 1)] {
            Row::Kind => {
                let index = Kind::ALL.iter().position(|k| *k == self.kind).unwrap_or(0);
                let next = (index as isize + delta).rem_euclid(Kind::ALL.len() as isize);
                self.kind = Kind::ALL[next as usize];
                self.clamp_size();
            }
            Row::Size => {
                let sizes = self.sizes();
                let index = sizes.iter().position(|s| *s == self.size).unwrap_or(0);
                let next = (index as isize + delta).rem_euclid(sizes.len() as isize);
                self.size = sizes[next as usize];
            }
            // Clamped rather than wrapped: a party of twenty rounds by way of one is not a
            // thing anybody meant to ask for.
            Row::Rounds => {
                self.rounds = (self.rounds as isize + delta).clamp(1, 20) as usize;
            }
            Row::Challenge => {
                let count = Challenge::ALL.len() as isize;
                self.challenge = (self.challenge as isize + delta).rem_euclid(count) as usize;
            }
            Row::Begin => {}
        }
    }

    pub fn handle(&mut self, input: Input) -> (Transition, PartyOutcome) {
        match self.stage {
            Stage::Setup => self.handle_setup(input),
            Stage::Round => self.handle_round(input),
            Stage::Finished => match input {
                Input::Confirm | Input::Submit | Input::Back => {
                    (Transition::Pop, PartyOutcome::Leave)
                }
                _ => (Transition::None, PartyOutcome::None),
            },
        }
    }

    fn handle_setup(&mut self, input: Input) -> (Transition, PartyOutcome) {
        let rows = Row::ALL.len();
        match input {
            Input::Up => self.cursor = (self.cursor + rows - 1) % rows,
            Input::Down => self.cursor = (self.cursor + 1) % rows,
            Input::Left => self.step(-1),
            Input::Right => self.step(1),
            Input::Confirm | Input::Submit => {
                if Row::ALL[self.cursor] == Row::Begin {
                    if self.short_by() == 0 {
                        return (Transition::None, PartyOutcome::Begin);
                    }
                } else {
                    self.step(1);
                }
            }
            Input::Back => return (Transition::Pop, PartyOutcome::Leave),
            Input::Hover(point) | Input::Click(point) => {
                if let Some((_, row)) = self.regions.iter().find(|(r, _)| r.contains(point)) {
                    self.cursor = *row;
                    if matches!(input, Input::Click(_)) {
                        return self.handle(Input::Confirm);
                    }
                }
            }
            _ => {}
        }
        (Transition::None, PartyOutcome::None)
    }

    /// The buttons the round stage offers, in order.
    fn round_buttons(&self) -> Vec<(&'static str, PartyOutcome)> {
        let mut buttons: Vec<(&'static str, PartyOutcome)> = Vec::new();
        if self.offered.is_some() {
            buttons.push(("Sing it", PartyOutcome::Sing));
        }
        if self.kind == Kind::Classic {
            if let Some(party) = &self.party {
                if party.can_reject() {
                    buttons.push(("Use a joker", PartyOutcome::Reroll));
                }
            }
        }
        if self.kind != Kind::Classic || self.offered.is_none() {
            buttons.push(("Choose a song", PartyOutcome::Choose));
        }
        buttons.push(("Give up on the party", PartyOutcome::Leave));
        buttons
    }

    fn handle_round(&mut self, input: Input) -> (Transition, PartyOutcome) {
        let buttons = self.round_buttons();
        let count = buttons.len().max(1);
        match input {
            Input::Up | Input::Left => self.cursor = (self.cursor + count - 1) % count,
            Input::Down | Input::Right => self.cursor = (self.cursor + 1) % count,
            Input::Confirm | Input::Submit => {
                let outcome = buttons[self.cursor.min(count - 1)].1.clone();
                if outcome == PartyOutcome::Leave {
                    return (Transition::Pop, outcome);
                }
                return (Transition::None, outcome);
            }
            // No quiet exit from a party: three other people are waiting and Escape landing
            // back on the main menu loses the standings.
            Input::Back => self.cursor = count - 1,
            Input::Hover(point) | Input::Click(point) => {
                if let Some((_, row)) = self.regions.iter().find(|(r, _)| r.contains(point)) {
                    self.cursor = (*row).min(count - 1);
                    if matches!(input, Input::Click(_)) {
                        return self.handle(Input::Confirm);
                    }
                }
            }
            _ => {}
        }
        (Transition::None, PartyOutcome::None)
    }

    /// Move to the round stage, putting the cursor on the first button.
    pub fn to_round(&mut self) {
        self.stage = Stage::Round;
        self.cursor = 0;
    }

    pub fn to_finished(&mut self) {
        self.stage = Stage::Finished;
        self.cursor = 0;
    }

    /// Who is up now: the team and its singer, or the two players of the next match.
    pub fn up_now(&self) -> String {
        if let Some(bracket) = &self.bracket {
            if let Some((round, index)) = bracket.next_match() {
                let fixture = &bracket.rounds[round][index];
                return format!(
                    "{} against {}",
                    bracket.name(fixture.left),
                    bracket.name(fixture.right)
                );
            }
        }
        if let Some(party) = &self.party {
            if let Some(team) = party.teams.get(party.team_up()) {
                return match team.singer() {
                    Some(singer) => format!("{} \u{2014} {singer}", team.name),
                    None => team.name.clone(),
                };
            }
        }
        String::new()
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.regions.clear();
        match self.stage {
            Stage::Setup => self.draw_setup(list, area, style),
            Stage::Round => self.draw_round(list, area, style),
            Stage::Finished => self.draw_finished(list, area, style),
        }
    }

    fn draw_setup(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        let body = widgets.header(list, area, "Party", self.kind.name());
        let hints: &[(&str, &str)] = if self.gamepad {
            &[("LS", "Change"), ("A", "Next"), ("B", "Back")]
        } else {
            &[
                ("\u{2190}\u{2192}", "Change"),
                ("Enter", "Next"),
                ("Esc", "Back"),
            ]
        };
        let body = widgets.footer(list, body, hints);

        // What the row under the cursor means, under the list rather than beside it.
        let (help, body) = body.cut_bottom(style.gap(4.0));
        list.text(
            help.inset_xy(style.gap(2.0), 0.0),
            self.help(),
            TextStyle::new(style.scaled_text(0.85), style.muted).valign(VAlign::Top),
        );

        let inner = body.inset(style.gap(2.0));
        let row_h = style.gap(3.6);
        for (index, row) in Row::ALL.iter().enumerate() {
            let rect = Rect::new(inner.x, inner.y + row_h * index as f32, inner.w, row_h)
                .inset_xy(0.0, style.gap(0.25));
            self.regions.push((rect, index));
            let selected = index == self.cursor;
            if *row == Row::Begin {
                let short = self.short_by();
                let label = if short == 0 {
                    "Start".to_owned()
                } else if short == 1 {
                    "One more singer needed".to_owned()
                } else {
                    format!("{short} more singers needed")
                };
                list.panel(
                    rect,
                    if !selected {
                        style.surface
                    } else if short == 0 {
                        style.accent
                    } else {
                        style.surface_raised
                    },
                    style.metrics.radius,
                );
                list.text(
                    rect,
                    label,
                    TextStyle::new(
                        style.text_size(),
                        match (selected, short) {
                            (true, 0) => style.on_accent,
                            (_, 0) => style.text,
                            _ => style.muted,
                        },
                    )
                    .centered()
                    .bold(),
                );
                continue;
            }
            let (label, value) = match row {
                Row::Kind => ("Mode", self.kind.name().to_owned()),
                Row::Size => (
                    if self.kind.is_tournament() {
                        "Players"
                    } else {
                        "Teams"
                    },
                    self.size.to_string(),
                ),
                Row::Rounds => (
                    "Rounds",
                    if self.kind.is_tournament() {
                        "until somebody wins".to_owned()
                    } else {
                        self.rounds.to_string()
                    },
                ),
                Row::Challenge => ("Sung how", self.challenge().name.to_owned()),
                Row::Begin => unreachable!(),
            };
            widgets.row(list, rect, label, &value, selected);
        }
    }

    /// What the row under the cursor means, shown under the list.
    pub fn help(&self) -> &str {
        match Row::ALL[self.cursor.min(Row::ALL.len() - 1)] {
            Row::Kind => self.kind.blurb(),
            Row::Size => {
                if self.kind.is_tournament() {
                    "Two, four, eight or sixteen. A bracket that is not a power of two would \
                     have somebody advance without singing."
                } else {
                    "Two or three teams. Three teams score three for a win and one for second."
                }
            }
            Row::Rounds => {
                if self.kind.is_tournament() {
                    "A tournament runs until one player is left, so the length is the size."
                } else {
                    "How many songs the party lasts. Everybody sings every round."
                }
            }
            Row::Challenge => self.challenge().blurb,
            Row::Begin => {
                "Singers come from the profiles in Singers. Add more there if you are short."
            }
        }
    }

    fn draw_round(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        let status = match (&self.party, &self.bracket) {
            (Some(party), _) => format!("Round {} of {}", party.round(), party.rounds),
            (_, Some(bracket)) => bracket
                .next_match()
                .map(|(round, _)| bracket.round_name(round).to_owned())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let body = widgets.header(list, area, "Party", &status);
        let hints: &[(&str, &str)] = if self.gamepad {
            &[("LS", "Move"), ("A", "Choose")]
        } else {
            &[("\u{2191}\u{2193}", "Move"), ("Enter", "Choose")]
        };
        let body = widgets.footer(list, body, hints);
        let inner = body.inset(style.gap(2.0));

        // Whose turn, then what they have been given, then the buttons.
        let (who, rest) = inner.cut_top(style.gap(7.0));
        list.text(
            who,
            self.up_now(),
            TextStyle::new(style.scaled_text(1.8), style.text)
                .bold()
                .centered()
                .overflow(Overflow::Ellipsis),
        );

        let (song, rest) = rest.cut_top(style.gap(6.0));
        let card = song.anchored(
            Anchor::Center,
            (song.w * 0.8).min(900.0),
            song.h - style.gap(1.0),
            0.0,
        );
        widgets.card(list, card);
        list.text(
            card,
            self.offered.clone().unwrap_or_else(|| {
                if self.kind == Kind::Classic {
                    "Drawing a song\u{2026}".to_owned()
                } else {
                    "Choose a song".to_owned()
                }
            }),
            TextStyle::new(style.scaled_text(1.1), style.text)
                .centered()
                .overflow(Overflow::Ellipsis),
        );

        // Jokers, which are the only resource a party has and so want to be visible.
        let (jokers, rest) = rest.cut_top(style.gap(3.4));
        if self.kind == Kind::Classic {
            if let Some(party) = &self.party {
                let line = party
                    .teams
                    .iter()
                    .map(|team| format!("{}: {} jokers", team.name, team.jokers))
                    .collect::<Vec<_>>()
                    .join("    ");
                list.text(
                    jokers,
                    line,
                    TextStyle::new(style.scaled_text(0.85), style.muted).centered(),
                );
            }
        }
        if !self.challenge().effects.is_plain() {
            list.text(
                jokers,
                self.challenge().name,
                TextStyle::new(style.scaled_text(0.85), style.accent)
                    .centered()
                    .valign(VAlign::Bottom),
            );
        }

        let buttons = self.round_buttons();
        let row_h = style.gap(3.6);
        for (index, (label, outcome)) in buttons.iter().enumerate() {
            let rect = Rect::new(rest.x, rest.y + row_h * index as f32, rest.w, row_h)
                .inset_xy(rest.w * 0.2, style.gap(0.25));
            self.regions.push((rect, index));
            let selected = index == self.cursor.min(buttons.len() - 1);
            let quitting = *outcome == PartyOutcome::Leave;
            list.panel(
                rect,
                match (selected, quitting) {
                    (true, true) => style.danger,
                    (true, false) => style.accent,
                    _ => style.surface,
                },
                style.metrics.radius,
            );
            list.text(
                rect,
                *label,
                TextStyle::new(
                    style.text_size(),
                    if selected {
                        style.on_accent
                    } else {
                        style.text
                    },
                )
                .centered()
                .bold(),
            );
        }

        self.draw_standings(list, inner, style);
    }

    /// The running standings, down the left, so nobody has to ask who is winning.
    fn draw_standings(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let Some(party) = &self.party else {
            return;
        };
        let column = Rect::new(area.x, area.y, (area.w * 0.24).min(280.0), area.h);
        let row_h = style.gap(3.0);
        for (place, index) in party.standings().iter().enumerate() {
            let team = &party.teams[*index];
            let rect = Rect::new(column.x, column.y + row_h * place as f32, column.w, row_h)
                .inset_xy(0.0, style.gap(0.2));
            list.panel(rect, style.surface.alpha(0.7), style.metrics.radius);
            let inner = rect.inset_xy(style.gap(1.0), 0.0);
            let (name, points) = inner.cut_left(inner.w * 0.62);
            list.text(
                name,
                &team.name,
                TextStyle::new(style.scaled_text(0.9), style.text).overflow(Overflow::Ellipsis),
            );
            list.text(
                points,
                team.points.to_string(),
                TextStyle::new(style.scaled_text(0.9), style.accent)
                    .bold()
                    .align(Align::End),
            );
        }
    }

    fn draw_finished(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        let body = widgets.header(list, area, "Party", "Finished");
        let hints: &[(&str, &str)] = if self.gamepad {
            &[("A", "Done")]
        } else {
            &[("Enter", "Done")]
        };
        let body = widgets.footer(list, body, hints);
        let inner = body.inset(style.gap(3.0));

        let winner = match (&self.party, &self.bracket) {
            (Some(party), _) => party
                .winner()
                .and_then(|index| party.teams.get(index))
                .map(|team| team.name.clone()),
            (_, Some(bracket)) => bracket.champion().map(|p| bracket.name(p).to_owned()),
            _ => None,
        };
        let (crown, rest) = inner.cut_top(style.gap(9.0));
        list.text(
            crown,
            // A drawn party says so rather than picking somebody: the standings are right
            // there and inventing a winner from them would be a lie about the evening.
            winner
                .clone()
                .map(|name| format!("{name} wins"))
                .unwrap_or_else(|| "A dead heat".to_owned()),
            TextStyle::new(style.scaled_text(2.4), style.accent)
                .bold()
                .centered()
                .overflow(Overflow::Ellipsis),
        );

        let row_h = style.gap(4.0);
        let table = rest.anchored(Anchor::Top, (rest.w * 0.7).min(760.0), rest.h, 0.0);
        let order: Vec<(String, String)> = match (&self.party, &self.bracket) {
            (Some(party), _) => party
                .standings()
                .iter()
                .map(|index| {
                    let team = &party.teams[*index];
                    (
                        team.name.clone(),
                        format!("{} \u{b7} {} sung", team.points, team.sung),
                    )
                })
                .collect(),
            (_, Some(bracket)) => bracket
                .placings()
                .iter()
                .map(|player| (bracket.name(*player).to_owned(), String::new()))
                .collect(),
            _ => Vec::new(),
        };
        for (place, (name, detail)) in order.iter().enumerate() {
            let rect = Rect::new(table.x, table.y + row_h * place as f32, table.w, row_h)
                .inset_xy(0.0, style.gap(0.3));
            list.panel(
                rect,
                if place == 0 {
                    style.surface_raised
                } else {
                    style.surface
                },
                style.metrics.radius,
            );
            let inner = rect.inset_xy(style.gap(1.4), 0.0);
            let (number, rest) = inner.cut_left(style.gap(3.0));
            list.text(
                number,
                format!("{}", place + 1),
                TextStyle::new(
                    style.text_size(),
                    if place == 0 {
                        style.accent
                    } else {
                        style.muted
                    },
                )
                .bold(),
            );
            let (name_box, detail_box) = rest.cut_left(rest.w * 0.6);
            list.text(
                name_box,
                name,
                TextStyle::new(style.scaled_text(1.1), style.text)
                    .bold()
                    .overflow(Overflow::Ellipsis),
            );
            list.text(
                detail_box,
                detail,
                TextStyle::new(style.scaled_text(0.85), style.muted).align(Align::End),
            );
        }
    }
}
