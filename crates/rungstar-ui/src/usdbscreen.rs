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
    /// The filter panel.
    Filtering,
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
    /// Fetch yt-dlp, without which nothing can be downloaded.
    GetTool,
    /// The search text changed, so the rows want refreshing.
    Search(String),
}

/// One thing the list can be narrowed by, beyond the catalogue's own fields.
///
/// Thirty thousand songs is not a list anybody scrolls, and the question that matters most is
/// **what have they got that I have not**.
///
/// These are toggles rather than one choice of five, because they are not one question. "Four
/// stars and up" and "not in my library" are independent things to want, and the first version
/// of this made them exclusive — so asking for well-rated songs silently stopped asking for
/// new ones. Only the two library states rule each other out, and they do it themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Narrow {
    /// Not in the library yet.
    New,
    /// Already downloaded, for checking or for fetching again.
    Held,
    /// Four stars and up.
    WellRated,
    /// Has golden notes, which usually means somebody took care over it.
    Golden,
}

impl Narrow {
    pub const ALL: [Narrow; 4] = [Narrow::New, Narrow::Held, Narrow::WellRated, Narrow::Golden];

    pub fn label(self) -> &'static str {
        match self {
            Self::New => "Not in my library",
            Self::Held => "Already have it",
            Self::WellRated => "4 stars and up",
            Self::Golden => "With golden notes",
        }
    }

    /// The one this cannot be on at the same time as.
    ///
    /// Having it and not having it is an empty list, and the useful reading of asking for
    /// both is that the second one is what was meant.
    pub fn contradicts(self) -> Option<Narrow> {
        match self {
            Self::New => Some(Self::Held),
            Self::Held => Some(Self::New),
            _ => None,
        }
    }

    /// Whether a row survives it.
    pub fn keeps(self, row: &Row) -> bool {
        match self {
            Self::New => row.local == Local::Absent,
            Self::Held => row.local != Local::Absent,
            Self::WellRated => row.rating >= 4.0,
            Self::Golden => row.golden,
        }
    }
}

/// A category the catalog can be narrowed by.
///
/// The same panel as the song browser's, deliberately: two screens that both filter lists
/// should not have two different ways of doing it, and somebody who has used one already
/// knows this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facet {
    /// Whether it is already downloaded, and how well rated. One at a time.
    Kind,
    Language,
    Genre,
    Decade,
    Edition,
}

impl Facet {
    pub const ALL: [Facet; 5] = [
        Facet::Kind,
        Facet::Language,
        Facet::Genre,
        Facet::Decade,
        Facet::Edition,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Kind => "Show",
            Self::Language => "Language",
            Self::Genre => "Genre",
            Self::Decade => "Decade",
            Self::Edition => "Edition",
        }
    }

    /// How a stored value is shown. A decade is stored as its first year.
    pub fn label(self, value: &str) -> String {
        match self {
            Self::Decade => format!("{value}s"),
            _ => value.to_owned(),
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// The values each facet has in the catalog, with how many songs each covers.
///
/// Supplied by the application, which is the only thing holding the catalog. Counts are of the
/// whole catalog rather than of the current results: a filter list that empties itself as you
/// use it cannot be used to widen a search again.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FacetValues {
    lists: Vec<Vec<(String, i64)>>,
}

impl FacetValues {
    pub fn new() -> Self {
        Self {
            lists: vec![Vec::new(); Facet::ALL.len()],
        }
    }

    pub fn set(&mut self, facet: Facet, values: Vec<(String, i64)>) {
        if self.lists.len() < Facet::ALL.len() {
            self.lists = vec![Vec::new(); Facet::ALL.len()];
        }
        self.lists[facet.index()] = values;
    }

    pub fn get(&self, facet: Facet) -> &[(String, i64)] {
        self.lists.get(facet.index()).map_or(&[], Vec::as_slice)
    }

    pub fn is_empty(&self) -> bool {
        self.lists.iter().all(Vec::is_empty)
    }
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
    pub genre: String,
    pub edition: String,
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
            genre: song.genre.clone(),
            edition: song.edition.clone(),
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
    /// A row in the filter panel's category column.
    Category(usize),
    /// A row in the filter panel's value column.
    Value(usize),
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
    /// Which of the toggles are on. Empty means everything.
    pub narrow: Vec<Narrow>,
    /// The values each facet offers, filled in by the application.
    pub facets: FacetValues,
    /// Values chosen per facet, in `Facet::ALL` order. Empty means no constraint.
    picked: Vec<Vec<String>>,
    facet_cursor: usize,
    value_cursor: usize,
    /// `true` while the value column has focus rather than the category column.
    on_values: bool,
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
            narrow: Vec::new(),
            facets: FacetValues::new(),
            picked: vec![Vec::new(); Facet::ALL.len()],
            facet_cursor: 0,
            value_cursor: 0,
            on_values: false,
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
            Mode::Filtering => self.handle_filtering(input),
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
                self.mode = Mode::Filtering;
                self.on_values = false;
                return (Transition::None, UsdbOutcome::None);
            }
            Input::CycleLayout => {
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
            Input::ContextMenu => {
                // One key, two meanings, decided by whether there is anything to stop:
                // cancelling is urgent and repairing is not, so the urgent one wins.
                return (
                    Transition::None,
                    if self.activity.busy() {
                        UsdbOutcome::Cancel
                    } else {
                        UsdbOutcome::Repair
                    },
                );
            }
            Input::Back => return (Transition::Pop, UsdbOutcome::None),
            // Fetching yt-dlp is a letter rather than a button on the screen: it is done once
            // ever, and a permanent button for a one-off is clutter on every other visit.
            Input::Type('g') | Input::Type('G') => return (Transition::None, UsdbOutcome::GetTool),
            _ => {}
        }
        (Transition::None, UsdbOutcome::None)
    }

    /// The filter category under the cursor.
    pub fn facet_title(&self) -> &'static str {
        self.facet().title()
    }

    fn facet(&self) -> Facet {
        Facet::ALL[self.facet_cursor.min(Facet::ALL.len() - 1)]
    }

    /// The values chosen for a facet.
    pub fn picked(&self, facet: Facet) -> &[String] {
        &self.picked[facet.index()]
    }

    /// How many values are chosen across every facet, for the header.
    pub fn active_filters(&self) -> usize {
        self.narrow.len() + self.picked.iter().map(Vec::len).sum::<usize>()
    }

    /// Put every filter back to showing everything.
    pub fn clear_filters(&mut self) {
        self.narrow.clear();
        for picked in &mut self.picked {
            picked.clear();
        }
        self.stale = true;
        self.cursor = 0;
    }

    /// Whether a row survives every filter. Values within a facet are OR; facets are AND.
    pub fn keeps(&self, row: &Row) -> bool {
        // Every toggle that is on has to be satisfied, which is what a list of checkboxes
        // looks like it means: new *and* well rated, not new *or* well rated.
        if !self.narrow.iter().all(|narrow| narrow.keeps(row)) {
            return false;
        }
        let matches = |facet: Facet, value: &str| {
            let picked = self.picked(facet);
            picked.is_empty() || picked.iter().any(|want| want.eq_ignore_ascii_case(value))
        };
        matches(Facet::Language, &row.language)
            && matches(Facet::Genre, &row.genre)
            && matches(Facet::Edition, &row.edition)
            && {
                let picked = self.picked(Facet::Decade);
                picked.is_empty()
                    || row.year.is_some_and(|year| {
                        let decade = year - year.rem_euclid(10);
                        picked.iter().any(|want| want.parse() == Ok(decade))
                    })
            }
    }

    /// The rows the panel shows for a facet: label, count, and whether it is chosen.
    fn rows_for(&self, facet: Facet) -> Vec<(String, String, bool)> {
        match facet {
            Facet::Kind => Narrow::ALL
                .iter()
                .map(|narrow| {
                    (
                        narrow.label().to_owned(),
                        String::new(),
                        self.narrow.contains(narrow),
                    )
                })
                .collect(),
            _ => {
                let picked = self.picked(facet);
                self.facets
                    .get(facet)
                    .iter()
                    .map(|(value, count)| {
                        (
                            facet.label(value),
                            count.to_string(),
                            picked.iter().any(|c| c == value),
                        )
                    })
                    .collect()
            }
        }
    }

    fn toggle_value(&mut self, facet: Facet, index: usize) {
        match facet {
            Facet::Kind => {
                let Some(narrow) = Narrow::ALL.get(index).copied() else {
                    return;
                };
                match self.narrow.iter().position(|held| *held == narrow) {
                    Some(at) => {
                        self.narrow.remove(at);
                    }
                    None => {
                        // Turning one on turns off the one it contradicts, rather than
                        // refusing: asking for both is really asking for the second.
                        if let Some(other) = narrow.contradicts() {
                            self.narrow.retain(|held| *held != other);
                        }
                        self.narrow.push(narrow);
                    }
                }
            }
            _ => {
                let Some((value, _)) = self.facets.get(facet).get(index).cloned() else {
                    return;
                };
                let picked = &mut self.picked[facet.index()];
                match picked.iter().position(|c| *c == value) {
                    Some(at) => {
                        picked.remove(at);
                    }
                    None => picked.push(value),
                }
            }
        }
        self.stale = true;
        self.cursor = 0;
    }

    fn handle_filtering(&mut self, input: Input) -> (Transition, UsdbOutcome) {
        let rows = self.rows_for(self.facet()).len();
        match input {
            Input::Up if self.on_values => {
                self.value_cursor = self.value_cursor.saturating_sub(1);
            }
            Input::Down if self.on_values => {
                if self.value_cursor + 1 < rows {
                    self.value_cursor += 1;
                }
            }
            Input::PageUp if self.on_values => {
                self.value_cursor = self.value_cursor.saturating_sub(8);
            }
            Input::PageDown if self.on_values => {
                self.value_cursor = (self.value_cursor + 8).min(rows.saturating_sub(1));
            }
            Input::Up => {
                self.facet_cursor = self.facet_cursor.saturating_sub(1);
                self.value_cursor = 0;
            }
            Input::Down => {
                self.facet_cursor = (self.facet_cursor + 1).min(Facet::ALL.len() - 1);
                self.value_cursor = 0;
            }
            Input::Right => self.on_values = true,
            Input::Left => self.on_values = false,
            Input::Confirm | Input::Submit => {
                if self.on_values {
                    let facet = self.facet();
                    self.toggle_value(facet, self.value_cursor);
                } else {
                    self.on_values = true;
                }
            }
            // The panel is where filters are set, so it is also where they are all undone.
            // Hunting through five categories for the one still narrowing the list is the
            // usual way a filter panel loses somebody.
            Input::Search => self.clear_filters(),
            Input::Back | Input::CycleFilter => {
                if self.on_values {
                    self.on_values = false;
                } else {
                    self.mode = Mode::Browsing;
                }
            }
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
            Some(Region::Category(index)) => {
                if index != self.facet_cursor {
                    self.facet_cursor = index;
                    self.value_cursor = 0;
                }
                self.on_values = clicked;
            }
            Some(Region::Value(index)) => {
                self.value_cursor = index;
                self.on_values = true;
                if clicked {
                    let facet = self.facet();
                    self.toggle_value(facet, index);
                }
            }
            None => {}
        }
        (Transition::None, UsdbOutcome::None)
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.regions.clear();
        let widgets = Widgets::new(style);
        // What is being hidden, because a list quietly missing songs is indistinguishable
        // from a catalog that does not have them.
        let mut parts: Vec<String> = Vec::new();
        // The first toggle by name and the rest as a count: "Not in my library +2" fits in a
        // header and still says that something is hidden, which is the part that matters.
        if let Some(first) = self.narrow.first() {
            parts.push(first.label().to_owned());
        }
        match self.active_filters() - usize::from(!self.narrow.is_empty()) {
            0 => {}
            n => parts.push(format!("+{n}")),
        }
        match &self.user {
            Some(user) => parts.push(user.clone()),
            None => parts.push("not signed in".to_owned()),
        }
        parts.push(match self.catalog_size {
            0 => "no catalog yet".to_owned(),
            n if self.rows.len() < n => format!("{} of {n}", self.rows.len()),
            n => format!("{n} songs"),
        });
        let status = parts.join("  \u{b7}  ");
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

        // A tip while signed out, above the list rather than instead of it: browsing works
        // without an account and downloading does not, and somebody who has never heard of
        // USDB has no way to guess that it is a website they have to register on first. Said
        // once at the top and gone the moment they sign in.
        let body = if self.user.is_none() {
            let (tip, rest) = body.cut_top(style.gap(5.2));
            let card = tip.inset_xy(style.gap(2.0), style.gap(0.4));
            list.panel(card, style.surface_raised, style.metrics.radius);
            let inner = card.inset_xy(style.gap(1.4), style.gap(0.4));
            let (first, second) = inner.cut_top(inner.h * 0.5);
            list.text(
                first,
                "You need a free USDB account to download songs",
                TextStyle::new(style.text_size(), style.text)
                    .bold()
                    .valign(VAlign::Bottom)
                    .overflow(Overflow::Ellipsis),
            );
            list.text(
                second,
                "Register at usdb.animux.de, then sign in here. Browsing and syncing the                  catalogue work without one.",
                TextStyle::new(style.scaled_text(0.85), style.muted)
                    .valign(VAlign::Top)
                    .overflow(Overflow::Ellipsis),
            );
            rest
        } else {
            body
        };

        let inner = body.inset(style.gap(2.0));
        if self.rows.is_empty() {
            let filtered = self.active_filters() > 0;
            let (title, detail) = match (self.catalog_size, self.search_text().is_empty()) {
                (0, _) => (
                    "No catalog yet",
                    "Sync to fetch the list of songs USDB has. It is a few hundred requests \
                     the first time and one or two after that.",
                ),
                _ if filtered => (
                    "Nothing matches the filter",
                    "Nothing in the catalog is left once this filter is applied. Change it or \
                     turn it off.",
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

        match self.mode {
            Mode::Browsing => {}
            Mode::Filtering => {
                let mut overlay = Vec::new();
                self.draw_filters(list, area, style, &mut overlay);
                self.regions.extend(overlay);
            }
            Mode::Searching | Mode::LoggingIn { .. } => {
                let mut overlay = Vec::new();
                self.draw_typing(list, area, style, &mut overlay);
                self.regions.extend(overlay);
            }
        }
    }

    /// The filter panel: categories on the left, their values on the right.
    ///
    /// The same shape as the song browser's, because two screens that both filter lists should
    /// not have two different ways of doing it.
    fn draw_filters(
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
            (area.w * 0.62).min(1100.0),
            (area.h * 0.78).min(760.0),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(1.8));

        let row_h = style.gap(3.0);
        let (heading, body) = inner.cut_top(row_h * 1.3);
        list.text(
            heading,
            "Filter",
            TextStyle::new(style.scaled_text(1.2), style.text).bold(),
        );
        let active = self.active_filters();
        list.text(
            heading,
            match active {
                0 => "Everything".to_owned(),
                1 => "1 filter".to_owned(),
                n => format!("{n} filters"),
            },
            TextStyle::new(
                style.text_size(),
                if active > 0 {
                    style.accent
                } else {
                    style.muted
                },
            )
            .align(Align::End),
        );

        let (categories, values) = body.cut_left((body.w * 0.34).min(320.0));
        for (index, facet) in Facet::ALL.iter().enumerate() {
            let row = Rect::new(
                categories.x,
                categories.y + row_h * index as f32,
                categories.w - style.gap(1.0),
                row_h,
            )
            .inset_xy(0.0, style.gap(0.2));
            regions.push((row, Region::Category(index)));
            let selected = index == self.facet_cursor;
            let chosen = match facet {
                Facet::Kind => self.narrow.len(),
                other => self.picked(*other).len(),
            };
            widgets.row(
                list,
                row,
                facet.title(),
                &if chosen == 0 {
                    String::new()
                } else {
                    chosen.to_string()
                },
                selected,
            );
            if selected && self.on_values {
                list.outline(
                    row,
                    style.accent,
                    style.metrics.outline,
                    style.metrics.radius,
                );
            }
        }

        let rows = self.rows_for(self.facet());
        if rows.is_empty() {
            list.text(
                values,
                "Nothing in the catalogue has one",
                TextStyle::new(style.text_size(), style.muted).centered(),
            );
            return;
        }

        let visible = ((values.h / row_h).floor() as usize).max(1);
        let first = self
            .value_cursor
            .saturating_sub(visible.saturating_sub(2))
            .min(rows.len().saturating_sub(visible.min(rows.len())));
        let mut placed = Vec::new();
        list.clipped(values, |list| {
            for (offset, (label, count, chosen)) in
                rows.iter().skip(first).take(visible).enumerate()
            {
                let index = first + offset;
                let row = Rect::new(values.x, values.y + row_h * offset as f32, values.w, row_h)
                    .inset_xy(0.0, style.gap(0.2));
                placed.push((row, Region::Value(index)));
                let selected = self.on_values && index == self.value_cursor;
                list.panel(
                    row,
                    if selected {
                        style.accent
                    } else if *chosen {
                        style.surface_raised
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
                let cell = row.inset_xy(style.gap(1.2), 0.0);
                // The tick has its own column so the labels line up whether ticked or not,
                // which is what makes a long list scannable.
                let (mark, rest) = cell.cut_left(style.gap(2.2));
                if *chosen {
                    list.text(
                        mark,
                        "\u{2713}",
                        TextStyle::new(
                            style.text_size(),
                            if selected {
                                style.on_accent
                            } else {
                                style.accent
                            },
                        )
                        .bold(),
                    );
                }
                let (name, number) = rest.cut_left(rest.w * 0.74);
                list.text(
                    Rect::new(name.x, name.y, (name.w - style.gap(1.0)).max(0.0), name.h),
                    label,
                    TextStyle::new(style.text_size(), text).overflow(Overflow::Ellipsis),
                );
                list.text(
                    number,
                    count,
                    TextStyle::new(style.scaled_text(0.82), muted).align(Align::End),
                );
            }
        });
        regions.extend(placed);

        if rows.len() > visible {
            list.text(
                Rect::new(
                    values.x,
                    values.bottom() - style.gap(2.0),
                    values.w,
                    style.gap(2.0),
                ),
                format!("{}/{}", self.value_cursor + 1, rows.len()),
                TextStyle::new(style.scaled_text(0.75), style.muted).align(Align::End),
            );
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
                    (if pad { "LT" } else { "D" }, "Filter"),
                    (if pad { "Y" } else { "F3" }, "Sync"),
                    (
                        if pad { "LB" } else { "Tab" },
                        if self.user.is_some() {
                            "Sign out"
                        } else {
                            "Sign in"
                        },
                    ),
                ];
                hints.push((
                    if pad { "RB" } else { "M" },
                    if self.activity.busy() {
                        "Stop"
                    } else {
                        "Repair"
                    },
                ));
                hints.push((back, "Back"));
                hints
            }
            Mode::Searching => vec![(confirm, "Press key"), (back, "Done")],
            Mode::Filtering => vec![
                (confirm, "Toggle"),
                ("\u{2190}\u{2192}", "Column"),
                (if pad { "X" } else { "F" }, "Clear all"),
                (back, "Done"),
            ],
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
                // The rating as five pips, drawn rather than written.
                //
                // It was a star glyph and an outlined-star glyph until somebody saw five empty
                // boxes. The game borrows a system font and a borrowed font is not promised to
                // contain any particular character; a shape the renderer draws itself cannot
                // go missing on somebody else's machine.
                let pip = style.gap(0.9);
                let pips = Rect::new(
                    side.right() - pip * 5.0,
                    side.y + style.gap(0.4),
                    pip * 5.0,
                    pip,
                );
                let lit = if selected {
                    style.on_accent
                } else {
                    style.warning
                };
                for star in 0..5 {
                    let cell =
                        Rect::new(pips.x + pip * star as f32, pips.y, pip, pip).inset(pip * 0.16);
                    let whole = row.rating - star as f32;
                    if whole >= 1.0 {
                        list.panel(cell, lit, cell.h / 2.0);
                    } else if whole >= 0.5 {
                        // A half is drawn as a half, not rounded away: USDB rates in halves
                        // and rounding loses the difference between a 3 and a 3.5.
                        list.outline(cell, lit.alpha(0.5), 1.5, cell.h / 2.0);
                        list.panel(
                            Rect::new(cell.x, cell.y, cell.w / 2.0, cell.h),
                            lit,
                            cell.h / 2.0,
                        );
                    } else {
                        list.outline(cell, lit.alpha(0.4), 1.5, cell.h / 2.0);
                    }
                }
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
            Mode::Browsing | Mode::Filtering => "",
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
