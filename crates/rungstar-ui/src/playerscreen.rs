//! Who is singing tonight.
//!
//! A profile here is a name and a colour, nothing more. UltraStar asks for a name before every
//! song and forgets it afterwards, which is why its statistics are full of "Player 1" — a
//! saved profile is what makes a highscore table mean anything over time.
//!
//! Names are typed with the same on-screen keyboard as the search, because a party is played
//! from the sofa and reaching for a keyboard to enter a name is exactly the moment somebody
//! gives up and stays "Player 1" forever.

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Rect};
use crate::keyboard::{Key, Keyboard};
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// A saved singer, as the screen shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub name: String,
    pub colour: u8,
    /// Songs sung, so a profile is visibly somebody's history rather than just a name.
    pub songs: i64,
    pub best: i32,
}

/// What the screen wants the application to do.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerOutcome {
    None,
    /// Create a profile with this name.
    Add(String),
    /// Rename a profile.
    Rename(i64, String),
    /// Change a profile's colour.
    Recolour(i64, u8),
    /// Delete a profile and everything it did.
    Remove(i64),
    /// Who is singing, in order. Empty means nobody has been chosen.
    Singers(Vec<i64>),
    /// Start the song this screen was opened for.
    Start,
}

/// What the screen is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Browsing,
    /// Typing a new name, or renaming the profile at `cursor`.
    Naming { renaming: bool },
    /// Confirming a deletion, because it takes their scores with it.
    Confirming,
}

/// The player screen.
pub struct PlayerScreen {
    pub players: Vec<Entry>,
    /// Who is singing, by profile id, in singer order.
    pub singers: Vec<i64>,
    /// How many microphones are assigned, which is how many can sing.
    pub microphones: usize,
    /// The song this screen was opened to choose singers for.
    ///
    /// When set, the screen is a step on the way into a song rather than a place to manage
    /// profiles, and it grows a Start row. Asking who is singing *before* the song is the
    /// only moment the answer is worth anything — afterwards the score has nowhere to go.
    pub for_song: Option<String>,
    /// Whether that song is a duet, so the two parts can be named against the singers.
    pub duet: Option<(String, String)>,
    pub gamepad: bool,
    cursor: usize,
    mode: Mode,
    keyboard: Keyboard,
    regions: Vec<(Rect, usize)>,
}

impl Default for PlayerScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerScreen {
    pub fn new() -> Self {
        Self {
            players: Vec::new(),
            singers: Vec::new(),
            microphones: 1,
            for_song: None,
            duet: None,
            gamepad: false,
            cursor: 0,
            mode: Mode::default(),
            keyboard: Keyboard::new().limit(24),
            regions: Vec::new(),
        }
    }

    /// Whether a text field has focus, so letter keys stay text.
    pub fn wants_text(&self) -> bool {
        matches!(self.mode, Mode::Naming { .. })
    }

    /// The row after the profiles, which adds one.
    fn add_row(&self) -> usize {
        self.players.len()
    }

    /// The Start row, when this screen is a step into a song.
    fn start_row(&self) -> Option<usize> {
        self.for_song.as_ref().map(|_| self.players.len() + 1)
    }

    fn rows(&self) -> usize {
        self.players.len() + 1 + usize::from(self.for_song.is_some())
    }

    /// Which singer number a profile is, if they are singing.
    pub fn singer_number(&self, id: i64) -> Option<usize> {
        self.singers.iter().position(|s| *s == id).map(|i| i + 1)
    }

    pub fn handle(&mut self, input: Input) -> (Transition, PlayerOutcome) {
        match self.mode {
            Mode::Naming { renaming } => {
                return (Transition::None, self.handle_naming(input, renaming))
            }
            Mode::Confirming => return (Transition::None, self.handle_confirming(input)),
            Mode::Browsing => {}
        }

        let rows = self.rows();
        match input {
            Input::Up => self.cursor = (self.cursor + rows - 1) % rows,
            Input::Down => self.cursor = (self.cursor + 1) % rows,
            Input::Left | Input::Right => {
                if let Some(player) = self.players.get(self.cursor) {
                    // Six colours, cycling. Somebody's colour is how they are recognised on
                    // the sing screen, so it is worth being able to choose.
                    let step: i32 = if matches!(input, Input::Right) { 1 } else { -1 };
                    let colour = (player.colour as i32 + step).rem_euclid(6) as u8;
                    let id = player.id;
                    self.players[self.cursor].colour = colour;
                    return (Transition::None, PlayerOutcome::Recolour(id, colour));
                }
            }
            Input::Confirm | Input::Submit => {
                if Some(self.cursor) == self.start_row() {
                    // Starting with nobody chosen is allowed: somebody may just want to sing
                    // without a profile, and refusing would be the game arguing.
                    return (Transition::None, PlayerOutcome::Start);
                }
                if self.cursor == self.add_row() {
                    self.keyboard = Keyboard::new().limit(24);
                    self.mode = Mode::Naming { renaming: false };
                } else if let Some(player) = self.players.get(self.cursor) {
                    // Toggle them in or out of tonight's line-up.
                    let id = player.id;
                    match self.singers.iter().position(|s| *s == id) {
                        Some(index) => {
                            self.singers.remove(index);
                        }
                        None if self.singers.len() < self.microphones.max(1) => {
                            self.singers.push(id);
                        }
                        None => {
                            // Full. Replacing the last one is friendlier than doing nothing,
                            // because "nothing happened" reads as a broken button.
                            self.singers.pop();
                            self.singers.push(id);
                        }
                    }
                    return (
                        Transition::None,
                        PlayerOutcome::Singers(self.singers.clone()),
                    );
                }
            }
            Input::Search => {
                if self.players.get(self.cursor).is_some() {
                    let current = self.players[self.cursor].name.clone();
                    self.keyboard = Keyboard::with_text(current).limit(24);
                    self.mode = Mode::Naming { renaming: true };
                }
            }
            Input::ContextMenu => {
                if self.players.get(self.cursor).is_some() {
                    self.mode = Mode::Confirming;
                }
            }
            Input::Back => return (Transition::Pop, PlayerOutcome::None),
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
        (Transition::None, PlayerOutcome::None)
    }

    fn handle_naming(&mut self, input: Input, renaming: bool) -> PlayerOutcome {
        let finish = match input {
            Input::Up => {
                self.keyboard.navigate(0, -1);
                false
            }
            Input::Down => {
                self.keyboard.navigate(0, 1);
                false
            }
            Input::Left => {
                self.keyboard.navigate(-1, 0);
                false
            }
            Input::Right => {
                self.keyboard.navigate(1, 0);
                false
            }
            Input::Confirm => self.keyboard.press(),
            Input::Submit => true,
            Input::Type(c) => {
                self.keyboard.push(c);
                false
            }
            Input::Backspace => {
                self.keyboard.backspace();
                false
            }
            Input::Back => {
                self.mode = Mode::Browsing;
                return PlayerOutcome::None;
            }
            _ => false,
        };
        if !finish {
            return PlayerOutcome::None;
        }

        let name = self.keyboard.text().trim().to_owned();
        self.mode = Mode::Browsing;
        if name.is_empty() {
            // An empty name is a cancelled edit, not a profile called nothing.
            return PlayerOutcome::None;
        }
        if renaming {
            match self.players.get(self.cursor) {
                Some(player) => PlayerOutcome::Rename(player.id, name),
                None => PlayerOutcome::None,
            }
        } else {
            PlayerOutcome::Add(name)
        }
    }

    fn handle_confirming(&mut self, input: Input) -> PlayerOutcome {
        match input {
            Input::Confirm | Input::Submit => {
                self.mode = Mode::Browsing;
                match self.players.get(self.cursor) {
                    Some(player) => PlayerOutcome::Remove(player.id),
                    None => PlayerOutcome::None,
                }
            }
            Input::Back | Input::ContextMenu => {
                self.mode = Mode::Browsing;
                PlayerOutcome::None
            }
            _ => PlayerOutcome::None,
        }
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.regions.clear();
        let widgets = Widgets::new(style);
        let status = match self.singers.len() {
            0 => format!("{} microphones", self.microphones),
            n => format!("{n} of {} singing", self.microphones.max(1)),
        };
        let title = match &self.for_song {
            Some(song) => song.as_str(),
            None => "Singers",
        };
        let body = widgets.header(list, area, title, &status);
        let hints: &[(&str, &str)] = match (self.mode, self.gamepad) {
            (Mode::Naming { .. }, true) => &[("A", "Press key"), ("B", "Cancel")],
            (Mode::Naming { .. }, false) => &[("Enter", "Done"), ("Esc", "Cancel")],
            (Mode::Confirming, true) => &[("A", "Delete"), ("B", "Keep")],
            (Mode::Confirming, false) => &[("Enter", "Delete"), ("Esc", "Keep")],
            (Mode::Browsing, true) => &[
                ("A", "Sing"),
                ("X", "Rename"),
                ("LS", "Colour"),
                ("Back", "Delete"),
            ],
            (Mode::Browsing, false) => &[
                ("Enter", "Sing"),
                ("F", "Rename"),
                ("\u{2190}\u{2192}", "Colour"),
                ("M", "Delete"),
            ],
        };
        let body = widgets.footer(list, body, hints);

        let inner = body.inset(style.gap(2.0));
        // From the name and the history line under it: a constant row put one through the
        // other as soon as the Text size setting went above 1.0.
        let name_size = style.text_size();
        let history_size = style.scaled_text(0.78);
        let row_h = style.row_height(&[name_size, history_size]) + style.gap(1.0);

        for (index, player) in self.players.iter().enumerate() {
            let rect = Rect::new(inner.x, inner.y + row_h * index as f32, inner.w, row_h)
                .inset_xy(0.0, style.gap(0.3));
            self.regions.push((rect, index));
            self.draw_player(list, rect, style, player, index == self.cursor);
        }

        let add = Rect::new(
            inner.x,
            inner.y + row_h * self.players.len() as f32,
            inner.w,
            row_h,
        )
        .inset_xy(0.0, style.gap(0.3));
        self.regions.push((add, self.add_row()));
        widgets.row(list, add, "Add a singer", "", self.cursor == self.add_row());

        if let Some(row) = self.start_row() {
            let start = Rect::new(inner.x, inner.y + row_h * (row as f32), inner.w, row_h)
                .inset_xy(0.0, style.gap(0.3));
            self.regions.push((start, row));
            let selected = self.cursor == row;
            list.panel(
                start,
                if selected {
                    style.accent
                } else {
                    style.surface
                },
                style.metrics.radius,
            );
            let label = match (&self.duet, self.singers.len()) {
                (Some((one, two)), n) if n >= 2 => format!("Sing \u{2014} {one} and {two}"),
                (Some(_), _) => "Sing \u{2014} a duet wants two singers".to_owned(),
                (None, 0) => "Sing without a profile".to_owned(),
                (None, _) => "Sing".to_owned(),
            };
            list.text(
                start,
                label,
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

        if self.players.is_empty() && self.mode == Mode::Browsing {
            let hint = Rect::new(
                inner.x,
                add.bottom() + style.gap(2.0),
                inner.w,
                style.gap(6.0),
            );
            list.text(
                hint,
                "Without a profile, scores are not kept and the statistics stay empty. \
                 UltraStar asks for a name before every song and forgets it afterwards, \
                 which is why its tables are full of Player 1.",
                TextStyle::new(style.scaled_text(0.85), style.muted),
            );
        }

        match self.mode {
            Mode::Naming { renaming } => self.draw_keyboard(list, area, style, renaming),
            Mode::Confirming => self.draw_confirm(list, area, style),
            Mode::Browsing => {}
        }
    }

    fn draw_player(
        &self,
        list: &mut DrawList,
        rect: Rect,
        style: &Style,
        player: &Entry,
        selected: bool,
    ) {
        let colour = style.player(player.colour as usize);
        list.panel(
            rect,
            if selected {
                style.surface_raised
            } else {
                style.surface
            },
            style.metrics.radius,
        );
        if selected {
            list.outline(
                rect,
                style.accent,
                style.metrics.outline,
                style.metrics.radius,
            );
        }

        let inner = rect.inset_xy(style.gap(1.2), style.gap(0.5));
        // A disc in their colour with their initial, which is what an avatar would be if one
        // were set and is better than an empty square when one is not.
        let (badge, rest) = inner.cut_left(inner.h);
        let disc = badge.fit_aspect(1.0);
        list.panel(disc, colour.alpha(0.28), disc.h / 2.0);
        list.outline(disc, colour, 2.0, disc.h / 2.0);
        list.text(
            disc,
            player
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default(),
            TextStyle::new(disc.h * 0.5, colour).centered().bold(),
        );

        // The same two sizes the row was measured from, recomputed from the style.
        let name_size = style.text_size();
        let history_size = style.scaled_text(0.78);
        let text = rest.inset_xy(style.gap(1.0), 0.0);
        let lines = style.stack(text, &[name_size, history_size]);
        let (top, bottom) = (lines[0], lines[1]);
        list.text(
            top,
            &player.name,
            TextStyle::new(name_size, style.text)
                .bold()
                .valign(VAlign::Bottom)
                .overflow(Overflow::Ellipsis),
        );
        let history = match player.songs {
            0 => "no songs yet".to_owned(),
            1 => format!("1 song, best {}", player.best),
            n => format!("{n} songs, best {}", player.best),
        };
        list.text(
            bottom,
            history,
            TextStyle::new(history_size, style.muted).valign(VAlign::Top),
        );

        // Whether they are singing, and as which player — the number decides which microphone
        // and which colour they get on the sing screen.
        match self.singer_number(player.id) {
            Some(number) => {
                let pill = text.anchored(Anchor::Right, style.gap(7.0), style.gap(2.2), 0.0);
                list.panel(pill, colour, pill.h / 2.0);
                list.text(
                    pill,
                    format!("Player {number}"),
                    TextStyle::new(style.scaled_text(0.78), colour.contrasting())
                        .centered()
                        .bold(),
                );
            }
            None => {
                list.text(
                    text,
                    "not singing",
                    TextStyle::new(style.scaled_text(0.78), style.muted).align(Align::End),
                );
            }
        }
    }

    fn draw_keyboard(&self, list: &mut DrawList, area: Rect, style: &Style, renaming: bool) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);

        let keys = self.keyboard.keys();
        let rows = self.keyboard.rows();
        let key_size = (area.w / 16.0).min(area.h / (rows as f32 + 7.0));
        let gap = key_size * 0.12;
        let grid_w = key_size * crate::keyboard::COLUMNS as f32;
        let grid_h = key_size * rows as f32;

        let card = area.anchored(
            Anchor::Center,
            grid_w + style.gap(4.0),
            grid_h + style.gap(10.0),
            0.0,
        );
        widgets.card(list, card);

        let title = Rect::new(
            card.x + style.gap(2.0),
            card.y + style.gap(1.4),
            card.w - style.gap(4.0),
            style.gap(2.4),
        );
        list.text(
            title,
            if renaming {
                "Rename"
            } else {
                "Who is singing?"
            },
            TextStyle::new(style.scaled_text(1.0), style.text).bold(),
        );

        let field = Rect::new(
            title.x,
            title.bottom() + style.gap(0.4),
            title.w,
            style.gap(3.2),
        );
        list.panel(field, style.surface_sunken, style.metrics.radius);
        let shown = if self.keyboard.is_empty() {
            "A name\u{2026}".to_owned()
        } else {
            format!("{}\u{2502}", self.keyboard.text())
        };
        list.text(
            field.inset_xy(style.gap(1.0), 0.0),
            shown,
            TextStyle::new(
                style.text_size(),
                if self.keyboard.is_empty() {
                    style.muted
                } else {
                    style.text
                },
            )
            .overflow(Overflow::Ellipsis),
        );

        let origin = Rect::new(
            card.center().x - grid_w / 2.0,
            field.bottom() + style.gap(1.0),
            grid_w,
            grid_h,
        );
        for (index, key) in keys.iter().enumerate() {
            let (row, column) = Keyboard::position(index);
            let cell = Rect::new(
                origin.x + column as f32 * key_size,
                origin.y + row as f32 * key_size,
                key_size,
                key_size,
            )
            .inset(gap);
            let selected = index == self.keyboard.cursor();
            list.panel(
                cell,
                if selected {
                    style.accent
                } else {
                    style.surface_raised
                },
                style.metrics.radius * 0.7,
            );
            let size = if key.wide() {
                key_size * 0.24
            } else {
                key_size * 0.45
            };
            list.text(
                cell,
                key.label(),
                TextStyle::new(
                    size,
                    if selected {
                        style.on_accent
                    } else {
                        style.text
                    },
                )
                .centered(),
            );
        }
        let _ = Key::Done;
    }

    fn draw_confirm(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.4).min(700.0),
            style.gap(14.0),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(2.5));

        let name = self
            .players
            .get(self.cursor)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let (title, rest) = inner.cut_top(style.gap(4.0));
        list.text(
            title,
            format!("Delete {name}?"),
            TextStyle::new(style.scaled_text(1.2), style.text)
                .bold()
                .centered(),
        );
        list.text(
            rest,
            "Their scores go too, and they will drop out of the statistics. There is no \
             undo for this.",
            TextStyle::new(style.text_size(), style.muted).centered(),
        );
    }
}
