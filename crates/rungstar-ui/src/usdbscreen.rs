//! Browsing USDB from inside the game.
//!
//! The whole point of the phase. usdb_syncer is a separate desktop application you alt-tab
//! away from, drive with a mouse, and come back from — which at a party, on a sofa, on a
//! handheld, means the song nobody has is the song nobody sings. This is the same catalog on
//! the same controller-driven screen as everything else.
//!
//! It draws from a local copy of the catalog, so it opens instantly, searches while offline,
//! and only talks to USDB when asked to sync or to download.

use rungstar_usdb::{CatalogSong, SongId};

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Point, Rect};
use crate::keyboard::{Key, Keyboard};
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// What the screen is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Browsing,
    /// Typing a search.
    Searching,
    /// Typing a username, then a password.
    LoggingIn { password: bool },
}

/// What the screen wants the application to do.
#[derive(Debug, Clone, PartialEq)]
pub enum UsdbOutcome {
    None,
    /// Bring the catalog up to date.
    Sync,
    /// Fetch this song.
    Download(SongId),
    /// Stop whatever is being fetched.
    Cancel,
    /// Log in with these.
    LogIn {
        user: String,
        password: String,
    },
    LogOut,
    /// Re-fetch everything a downloaded song is missing.
    Repair,
    /// The search text changed, so the rows want refreshing.
    Search(String),
}

/// How a song in the catalog stands against the local library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Local {
    /// Not downloaded.
    #[default]
    Absent,
    /// Downloaded and complete.
    Held,
    /// Downloaded, but USDB has edited it since.
    Stale,
    /// Being fetched now.
    Fetching,
}

/// One row of the catalog as this screen shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: SongId,
    pub artist: String,
    pub title: String,
    pub language: String,
    pub year: Option<i32>,
    pub rating: f32,
    pub golden: bool,
    pub local: Local,
}

impl Row {
    pub fn from_catalog(song: &CatalogSong, local: Local) -> Self {
        Self {
            id: song.id,
            artist: song.artist.clone(),
            title: song.title.clone(),
            language: song.language.clone(),
            year: song.year,
            rating: song.rating,
            golden: song.golden_notes,
            local,
        }
    }
}

/// What a download is doing right now, for the strip along the bottom.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Activity {
    /// What is happening, in a few words. Empty when nothing is.
    pub what: String,
    /// How far through, when that is known.
    pub fraction: Option<f32>,
    /// How many songs are waiting behind this one.
    pub queued: usize,
}

impl Activity {
    pub fn busy(&self) -> bool {
        !self.what.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Song(usize),
    Key(usize),
    /// The show-password button.
    Reveal,
}

/// The USDB browser.
pub struct UsdbScreen {
    /// Rows to show, already searched and sorted by the application.
    pub rows: Vec<Row>,
    /// How many songs the catalog holds in total, for the header.
    pub catalog_size: usize,
    /// Who is logged in, if anybody.
    pub user: Option<String>,
    /// What a sync or a download is doing.
    pub activity: Activity,
    /// The last thing that went wrong, shown until something else happens.
    pub problem: String,
    pub gamepad: bool,
    mode: Mode,
    keyboard: Keyboard,
    /// The username typed, kept while the password is being typed.
    user_typed: String,
    /// The search the rows were fetched for.
    searched: String,
    /// Whether the password is shown as itself rather than as dots.
    ///
    /// Off by default and never remembered. On is for the twenty seconds it takes to check a
    /// password with symbols in it, which is the only reason anybody wants this.
    reveal: bool,
    cursor: usize,
    scroll: usize,
    regions: Vec<(Rect, Region)>,
    /// Set when the search text changed and the application should re-query.
    stale: bool,
}

impl Default for UsdbScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl UsdbScreen {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            catalog_size: 0,
            user: None,
            activity: Activity::default(),
            problem: String::new(),
            gamepad: false,
            mode: Mode::Browsing,
            keyboard: Keyboard::new(),
            user_typed: String::new(),
            searched: String::new(),
            reveal: false,
            cursor: 0,
            scroll: 0,
            regions: Vec::new(),
            stale: true,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether a text field has focus, so letter keys are text and nothing else.
    pub fn wants_text(&self) -> bool {
        matches!(self.mode, Mode::Searching | Mode::LoggingIn { .. })
    }

    pub fn search_text(&self) -> &str {
        match self.mode {
            Mode::Searching => self.keyboard.text(),
            _ => &self.searched,
        }
    }

    /// Whether the application should re-run the search.
    pub fn needs_rows(&self) -> bool {
        self.stale
    }

    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.cursor = self.cursor.min(rows.len().saturating_sub(1));
        self.rows = rows;
        self.stale = false;
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    pub fn handle(&mut self, input: Input) -> (Transition, UsdbOutcome) {
        if let Input::Hover(point) | Input::Click(point) = input {
            return self.handle_pointer(point, matches!(input, Input::Click(_)));
        }
        match self.mode {
            Mode::Browsing => self.handle_browsing(input),
            Mode::Searching => self.handle_searching(input),
            Mode::LoggingIn { password } => self.handle_login(input, password),
        }
    }

    fn handle_browsing(&mut self, input: Input) -> (Transition, UsdbOutcome) {
        let count = self.rows.len();
        match input {
            Input::Up => self.cursor = self.cursor.saturating_sub(1),
            Input::Down => {
                if self.cursor + 1 < count {
                    self.cursor += 1;
                }
            }
            Input::PageUp => self.cursor = self.cursor.saturating_sub(10),
            Input::PageDown => self.cursor = (self.cursor + 10).min(count.saturating_sub(1)),
            Input::Search => {
                self.keyboard = Keyboard::with_text(self.searched.clone());
                self.mode = Mode::Searching;
            }
            Input::Confirm | Input::Submit => {
                if let Some(row) = self.rows.get(self.cursor) {
                    // A song already held and up to date is not downloaded again. The button
                    // would look like it did nothing, which is worse than not offering it.
                    if row.local != Local::Held && row.local != Local::Fetching {
                        return (Transition::None, UsdbOutcome::Download(row.id));
                    }
                }
            }
            // Sort has no meaning here, so the key that opens the sort picker in the library
            // syncs instead: it is the other thing this screen does.
            Input::Sort => return (Transition::None, UsdbOutcome::Sync),
            Input::CycleFilter => {
                return (
                    Transition::None,
                    match &self.user {
                        Some(_) => UsdbOutcome::LogOut,
                        None => {
                            self.keyboard = Keyboard::new().limit(48);
                            self.mode = Mode::LoggingIn { password: false };
                            UsdbOutcome::None
                        }
                    },
                )
            }
            Input::CycleLayout => return (Transition::None, UsdbOutcome::Repair),
            Input::ContextMenu => {
                if self.activity.busy() {
                    return (Transition::None, UsdbOutcome::Cancel);
                }
            }
            Input::Back => return (Transition::Pop, UsdbOutcome::None),
            _ => {}
        }
        (Transition::None, UsdbOutcome::None)
    }

    fn handle_searching(&mut self, input: Input) -> (Transition, UsdbOutcome) {
        match input {
            Input::Back | Input::Submit | Input::Search => {
                self.mode = Mode::Browsing;
                return (Transition::None, UsdbOutcome::None);
            }
            Input::Type(c) => self.keyboard.push(c),
            Input::Backspace => {
                self.keyboard.apply(Key::Backspace);
            }
            Input::Up => {
                self.keyboard.navigate(0, -1);
                return (Transition::None, UsdbOutcome::None);
            }
            Input::Down => {
                self.keyboard.navigate(0, 1);
                return (Transition::None, UsdbOutcome::None);
            }
            Input::Left => {
                self.keyboard.navigate(-1, 0);
                return (Transition::None, UsdbOutcome::None);
            }
            Input::Right => {
                self.keyboard.navigate(1, 0);
                return (Transition::None, UsdbOutcome::None);
            }
            Input::Confirm => {
                if self.keyboard.press() {
                    self.mode = Mode::Browsing;
                }
            }
            _ => return (Transition::None, UsdbOutcome::None),
        }
        // Live, as in the library browser: the list narrows while you type rather than after.
        self.searched = self.keyboard.text().to_owned();
        self.stale = true;
        self.cursor = 0;
        (Transition::None, UsdbOutcome::Search(self.searched.clone()))
    }

    fn handle_login(&mut self, input: Input, password: bool) -> (Transition, UsdbOutcome) {
        match input {
            Input::Back => {
                self.mode = Mode::Browsing;
                self.reveal = false;
                self.user_typed.clear();
            }
            Input::Type(c) => self.keyboard.push(c),
            Input::Backspace => {
                self.keyboard.apply(Key::Backspace);
            }
            // Keys that type nothing, so they still work while every letter is text rather
            // than a shortcut.
            Input::Sort | Input::CycleFilter => {
                if password {
                    self.reveal = !self.reveal;
                }
            }
            Input::Up => self.keyboard.navigate(0, -1),
            Input::Down => self.keyboard.navigate(0, 1),
            Input::Left => self.keyboard.navigate(-1, 0),
            Input::Right => self.keyboard.navigate(1, 0),
            Input::Confirm | Input::Submit => {
                let done = matches!(input, Input::Submit) || self.keyboard.press();
                if !done {
                    return (Transition::None, UsdbOutcome::None);
                }
                let typed = self.keyboard.text().to_owned();
                if password {
                    self.mode = Mode::Browsing;
                    self.reveal = false;
                    let user = std::mem::take(&mut self.user_typed);
                    self.keyboard = Keyboard::new();
                    return (
                        Transition::None,
                        UsdbOutcome::LogIn {
                            user,
                            password: typed,
                        },
                    );
                }
                self.user_typed = typed;
                self.keyboard = Keyboard::new().limit(64);
                self.mode = Mode::LoggingIn { password: true };
            }
            _ => {}
        }
        (Transition::None, UsdbOutcome::None)
    }

    fn handle_pointer(&mut self, point: Point, clicked: bool) -> (Transition, UsdbOutcome) {
        let hit = self
            .regions
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, region)| *region);
        match hit {
            Some(Region::Song(index)) => {
                self.cursor = index;
                if clicked {
                    return self.handle_browsing(Input::Confirm);
                }
            }
            Some(Region::Key(index)) => {
                self.keyboard.set_cursor(index);
                if clicked {
                    return self.handle(Input::Confirm);
                }
            }
            Some(Region::Reveal) if clicked => self.reveal = !self.reveal,
            Some(Region::Reveal) => {}
            None => {}
        }
        (Transition::None, UsdbOutcome::None)
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.regions.clear();
        let widgets = Widgets::new(style);
        let status = match (&self.user, self.catalog_size) {
            (Some(user), 0) => format!("{user}  \u{b7}  no catalog yet"),
            (Some(user), n) => format!("{user}  \u{b7}  {n} songs"),
            (None, 0) => "not signed in".to_owned(),
            (None, n) => format!("not signed in  \u{b7}  {n} songs"),
        };
        let body = widgets.header(list, area, "USDB", &status);
        let body = widgets.footer(list, body, &self.hints());

        // What a sync or a download is doing, along the bottom. A background job with no
        // visible sign of life is indistinguishable from one that has died.
        let body = if self.activity.busy() || !self.problem.is_empty() {
            let (strip, rest) = body.cut_bottom(style.gap(4.0));
            self.draw_activity(list, strip, style);
            rest
        } else {
            body
        };

        let inner = body.inset(style.gap(2.0));
        if self.rows.is_empty() {
            let (title, detail) = match (self.catalog_size, self.search_text().is_empty()) {
                (0, _) => (
                    "No catalog yet",
                    "Sync to fetch the list of songs USDB has. It is a few hundred requests \
                     the first time and one or two after that.",
                ),
                (_, false) => (
                    "Nothing matches",
                    "Nothing in the catalog has those words in its artist or title.",
                ),
                _ => ("Nothing here", "The catalog is empty."),
            };
            widgets.empty_state(list, inner, title, detail);
        } else {
            self.draw_rows(list, inner, style);
        }

        if self.mode != Mode::Browsing {
            let mut overlay = Vec::new();
            self.draw_typing(list, area, style, &mut overlay);
            self.regions.extend(overlay);
        }
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        let pad = self.gamepad;
        let confirm = if pad { "A" } else { "Enter" };
        let back = if pad { "B" } else { "Esc" };
        match self.mode {
            Mode::Browsing => {
                let mut hints = vec![
                    (confirm, "Download"),
                    (if pad { "X" } else { "F" }, "Search"),
                    (if pad { "Y" } else { "F3" }, "Sync"),
                    (
                        if pad { "LT" } else { "D" },
                        if self.user.is_some() {
                            "Sign out"
                        } else {
                            "Sign in"
                        },
                    ),
                ];
                if self.activity.busy() {
                    hints.push((if pad { "RB" } else { "M" }, "Stop"));
                }
                hints.push((back, "Back"));
                hints
            }
            Mode::Searching => vec![(confirm, "Press key"), (back, "Done")],
            Mode::LoggingIn { password } => {
                let mut hints = vec![(confirm, "Press key")];
                if password {
                    hints.push((
                        if pad { "Y" } else { "F3" },
                        if self.reveal { "Hide it" } else { "Show it" },
                    ));
                }
                hints.push((back, "Cancel"));
                hints
            }
        }
    }

    fn draw_rows(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        let row_h = style.gap(3.6);
        let visible = ((area.h / row_h).floor() as usize).max(1);
        self.scroll = self
            .cursor
            .saturating_sub(visible.saturating_sub(2))
            .min(self.rows.len().saturating_sub(visible.min(self.rows.len())));
        let first = self.scroll;

        let mut regions = Vec::new();
        let rows = &self.rows;
        let cursor = self.cursor;
        list.clipped(area, |list| {
            for (offset, row) in rows.iter().skip(first).take(visible).enumerate() {
                let index = first + offset;
                let rect = Rect::new(area.x, area.y + row_h * offset as f32, area.w, row_h)
                    .inset_xy(0.0, style.gap(0.25));
                regions.push((rect, Region::Song(index)));
                let selected = index == cursor;
                list.panel(
                    rect,
                    if selected {
                        style.accent
                    } else {
                        style.surface
                    },
                    style.metrics.radius,
                );
                let text = if selected {
                    style.on_accent
                } else {
                    style.text
                };
                let muted = if selected {
                    style.on_accent
                } else {
                    style.muted
                };

                let inner = rect.inset_xy(style.gap(1.2), 0.0);
                // The state column first, so the eye can run down it looking for what is not
                // held yet — which is the only reason anybody is on this screen.
                let (state, rest) = inner.cut_left(style.gap(7.0));
                let (label, colour) = match row.local {
                    Local::Absent => ("", muted),
                    Local::Held => ("in library", style.success),
                    Local::Stale => ("updated", style.warning),
                    Local::Fetching => ("fetching", style.accent),
                };
                if !label.is_empty() {
                    list.text(
                        state,
                        label,
                        TextStyle::new(style.scaled_text(0.72), colour).bold(),
                    );
                }

                let (name, side) = rest.cut_left(rest.w * 0.62);
                let (top, bottom) = name.cut_top(name.h * 0.56);
                list.text(
                    top,
                    &row.title,
                    TextStyle::new(style.text_size(), text)
                        .valign(VAlign::Bottom)
                        .overflow(Overflow::Ellipsis),
                );
                list.text(
                    bottom,
                    &row.artist,
                    TextStyle::new(style.scaled_text(0.8), muted)
                        .valign(VAlign::Top)
                        .overflow(Overflow::Ellipsis),
                );

                let mut detail: Vec<String> = Vec::new();
                if !row.language.is_empty() {
                    detail.push(row.language.clone());
                }
                if let Some(year) = row.year {
                    detail.push(year.to_string());
                }
                if row.golden {
                    detail.push("golden".to_owned());
                }
                list.text(
                    side,
                    detail.join("  \u{b7}  "),
                    TextStyle::new(style.scaled_text(0.78), muted)
                        .align(Align::End)
                        .valign(VAlign::Bottom)
                        .overflow(Overflow::Ellipsis),
                );
                // Stars, drawn rather than written: five glyphs read faster than "4.5".
                list.text(
                    side,
                    stars(row.rating),
                    TextStyle::new(
                        style.scaled_text(0.78),
                        if selected {
                            style.on_accent
                        } else {
                            style.warning
                        },
                    )
                    .align(Align::End)
                    .valign(VAlign::Top),
                );
            }
        });
        self.regions.extend(regions);

        if self.rows.len() > visible {
            list.text(
                Rect::new(
                    area.x,
                    area.bottom() - style.gap(2.0),
                    area.w,
                    style.gap(2.0),
                ),
                format!("{} of {}", self.cursor + 1, self.rows.len()),
                TextStyle::new(style.scaled_text(0.75), style.muted).align(Align::End),
            );
        }
    }

    fn draw_activity(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let inner = area.inset_xy(style.gap(2.0), style.gap(0.4));
        list.panel(inner, style.surface, style.metrics.radius);
        let text = inner.inset_xy(style.gap(1.2), 0.0);
        let (line, bar) = text.cut_top(text.h * 0.62);
        let (what, colour) = if self.problem.is_empty() {
            (self.activity.what.clone(), style.text)
        } else {
            (self.problem.clone(), style.danger)
        };
        list.text(
            line,
            what,
            TextStyle::new(style.scaled_text(0.85), colour).overflow(Overflow::Ellipsis),
        );
        if self.activity.queued > 0 {
            list.text(
                line,
                format!("{} waiting", self.activity.queued),
                TextStyle::new(style.scaled_text(0.78), style.muted).align(Align::End),
            );
        }
        if let Some(fraction) = self.activity.fraction {
            let track = Rect::new(bar.x, bar.y + bar.h * 0.3, bar.w, style.gap(0.4));
            list.panel(track, style.surface_sunken, track.h / 2.0);
            list.panel(
                Rect::new(
                    track.x,
                    track.y,
                    track.w * fraction.clamp(0.0, 1.0),
                    track.h,
                ),
                style.accent,
                track.h / 2.0,
            );
        }
    }

    fn draw_typing(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.7).min(1000.0),
            (area.h * 0.62).min(620.0),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(2.0));
        let (heading, rest) = inner.cut_top(style.gap(3.4));
        let title = match self.mode {
            Mode::Searching => "Search USDB",
            Mode::LoggingIn { password: false } => "USDB username",
            Mode::LoggingIn { password: true } => "USDB password",
            Mode::Browsing => "",
        };
        list.text(
            heading,
            title,
            TextStyle::new(style.scaled_text(1.2), style.text).bold(),
        );

        let (field, keys) = rest.cut_top(style.gap(4.0));
        // A password is dots by default. Not for shoulder-surfing on a sofa — for the
        // screenshot somebody takes of the party and puts online.
        //
        // But it can be shown, because a password with symbols in it cannot be checked any
        // other way, and a sign-in that fails with no way to see what was typed is one nobody
        // can debug. Off again the moment the field is left.
        let typing_password = matches!(self.mode, Mode::LoggingIn { password: true });
        list.panel(
            field.inset_xy(0.0, style.gap(0.4)),
            style.surface_sunken,
            style.metrics.radius,
        );
        let (eye, field) = if typing_password {
            field.cut_right(style.gap(7.0))
        } else {
            (Rect::default(), field)
        };
        let shown = if typing_password && !self.reveal {
            "\u{2022}".repeat(self.keyboard.text().chars().count())
        } else {
            self.keyboard.text().to_owned()
        };
        list.text(
            field.inset_xy(style.gap(1.4), 0.0),
            shown,
            TextStyle::new(style.text_size(), style.text).overflow(Overflow::Ellipsis),
        );
        if typing_password {
            let button = eye.inset_xy(style.gap(0.4), style.gap(0.8));
            regions.push((button, Region::Reveal));
            list.panel(
                button,
                if self.reveal {
                    style.accent
                } else {
                    style.surface_raised
                },
                style.metrics.radius,
            );
            list.text(
                button,
                if self.reveal { "Hide" } else { "Show" },
                TextStyle::new(
                    style.scaled_text(0.8),
                    if self.reveal {
                        style.on_accent
                    } else {
                        style.text
                    },
                )
                .centered(),
            );
        }

        self.draw_keys(list, keys, style, regions);
    }
}

impl UsdbScreen {
    /// The on-screen keyboard's grid.
    ///
    /// Drawn here rather than shared with the library browser because that one carries a
    /// "searching artist" caption and a result count that mean nothing on this screen, and a
    /// shared widget with two thirds of it switched off is worse than two grids.
    fn draw_keys(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let keys = self.keyboard.keys();
        let rows = self.keyboard.rows().max(1);
        let size = (area.w / crate::keyboard::COLUMNS as f32).min(area.h / rows as f32);
        let gap = size * 0.12;
        let origin = Rect::new(
            area.center().x - size * crate::keyboard::COLUMNS as f32 / 2.0,
            area.y,
            size * crate::keyboard::COLUMNS as f32,
            size * rows as f32,
        );
        for (index, key) in keys.iter().enumerate() {
            let (row, column) = Keyboard::position(index);
            let cell = Rect::new(
                origin.x + column as f32 * size,
                origin.y + row as f32 * size,
                size,
                size,
            )
            .inset(gap);
            regions.push((cell, Region::Key(index)));
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
            list.text(
                cell,
                key.label(),
                TextStyle::new(
                    if key.wide() { size * 0.24 } else { size * 0.45 },
                    if selected {
                        style.on_accent
                    } else {
                        style.text
                    },
                )
                .centered(),
            );
        }
    }
}

/// Five glyphs for a rating, halves included.
fn stars(rating: f32) -> String {
    let whole = rating.floor().clamp(0.0, 5.0) as usize;
    let half = usize::from(rating - rating.floor() >= 0.5 && whole < 5);
    let mut out = "\u{2605}".repeat(whole);
    if half == 1 {
        out.push('\u{00BD}');
    }
    out.push_str(&"\u{2606}".repeat(5 - whole - half));
    out
}
