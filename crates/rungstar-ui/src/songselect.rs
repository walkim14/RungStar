//! The song browser: three layouts, live search, sorting, and the detail panel.
//!
//! The screen holds the result set and a cursor; it does not know how to query a library. When
//! the search text or the sort changes it raises a flag, the application runs the query and
//! hands the rows back. That keeps the screen pure — a test drives it with a list of songs and
//! reads back the display list — and it keeps the query off the render thread, which is what
//! makes typing into a 30,000 song library stay responsive.

use rungstar_library::{SearchField, SongEntry, SortKey};

use crate::browse::{Browser, Layout};
use crate::draw::{Align, DrawList, ImageId, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Rect};
use crate::keyboard::{Key, Keyboard};
use crate::screen::{Transition, Widgets};
use crate::theme::Style;

/// What the screen is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Moving through the list.
    #[default]
    Browsing,
    /// The on-screen keyboard is up and typing filters the list live.
    Searching,
    /// Choosing what to sort by.
    Sorting,
}

/// Semantic inputs the screen understands. Deliberately not device events: the same enum comes
/// from a keyboard, a gamepad or a touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    /// Open or close the search.
    Search,
    /// Cycle the browse layout.
    CycleLayout,
    /// Open the sort picker.
    Sort,
    Random,
    PageUp,
    PageDown,
    /// A character from a physical keyboard, while searching.
    Type(char),
    Backspace,
}

/// The sorts offered, in the order the picker lists them.
///
/// Four of these — times played, last played, duration and difficulty — have no equivalent in
/// UltraStar Deluxe, and they are the ones a real library actually gets sorted by.
pub const SORTS: [(SortKey, &str); 12] = [
    (SortKey::Artist, "Artist"),
    (SortKey::Title, "Title"),
    (SortKey::Edition, "Edition"),
    (SortKey::Genre, "Genre"),
    (SortKey::Language, "Language"),
    (SortKey::Folder, "Folder"),
    (SortKey::Year, "Year"),
    (SortKey::Decade, "Decade"),
    (SortKey::Creator, "Creator"),
    (SortKey::TimesPlayed, "Times played"),
    (SortKey::LastPlayed, "Last played"),
    (SortKey::Difficulty, "Difficulty"),
];

/// Which fields the search box can be pointed at.
pub const FIELDS: [(SearchField, &str); 4] = [
    (SearchField::All, "Everything"),
    (SearchField::Artist, "Artist"),
    (SearchField::Title, "Title"),
    (SearchField::Lyrics, "Lyrics"),
];

/// The song browser.
pub struct SongSelect {
    pub browser: Browser,
    songs: Vec<SongEntry>,
    keyboard: Keyboard,
    mode: Mode,
    sort_cursor: usize,
    /// Whether the player chose the sort, as opposed to it being the default.
    sort_chosen: bool,
    field_cursor: usize,
    descending: bool,
    /// Set when the query the application should be running has changed.
    stale: bool,
    /// Text the current `songs` were fetched for, so a slow query landing late can be ignored.
    fetched_for: String,
}

impl Default for SongSelect {
    fn default() -> Self {
        Self::new()
    }
}

impl SongSelect {
    pub fn new() -> Self {
        Self {
            browser: Browser::new(),
            songs: Vec::new(),
            keyboard: Keyboard::new(),
            mode: Mode::Browsing,
            sort_cursor: 0,
            sort_chosen: false,
            field_cursor: 0,
            descending: false,
            stale: true,
            fetched_for: String::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn songs(&self) -> &[SongEntry] {
        &self.songs
    }

    pub fn search_text(&self) -> &str {
        self.keyboard.text()
    }

    /// The sort to query with.
    ///
    /// A search that has not been given an explicit sort ranks by relevance, because the
    /// alphabetically-first song containing your words is almost never the one you meant --
    /// typing a line of a chorus and getting every song that merely contains those five
    /// words, in artist order, is not a lyric search. UltraStar Deluxe sorts alphabetically
    /// always, which is why searching it only works when you already know the title.
    pub fn sort(&self) -> SortKey {
        if self.ranking() {
            SortKey::Relevance
        } else {
            SORTS[self.sort_cursor].0
        }
    }

    /// The sort the picker is showing, which is the chosen one even while ranking is in use.
    pub fn chosen_sort(&self) -> SortKey {
        SORTS[self.sort_cursor].0
    }

    /// Whether results are currently ranked rather than ordered.
    pub fn ranking(&self) -> bool {
        !self.sort_chosen && !self.keyboard.is_empty()
    }

    pub fn field(&self) -> SearchField {
        FIELDS[self.field_cursor].0
    }

    pub fn descending(&self) -> bool {
        self.descending
    }

    /// The song under the cursor.
    pub fn selected(&self) -> Option<&SongEntry> {
        self.songs.get(self.browser.cursor())
    }

    /// Whether the application should re-run the query.
    pub fn needs_query(&self) -> bool {
        self.stale
    }

    /// Hand back the rows for the current query.
    ///
    /// The cursor is kept where it was when the result set still contains the song it was on,
    /// so refining a search does not throw away the selection you were narrowing towards.
    pub fn set_results(&mut self, songs: Vec<SongEntry>) {
        let previous = self.selected().map(|s| s.id);
        self.songs = songs;
        self.browser.set_count(self.songs.len());
        if let Some(id) = previous {
            if let Some(index) = self.songs.iter().position(|s| s.id == id) {
                self.browser.jump_to(index);
            }
        }
        self.stale = false;
        self.fetched_for = self.keyboard.text().to_owned();
    }

    /// Advance animations. Returns whether anything is still moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.browser.tick(dt);
        self.browser.animating()
    }

    pub fn handle(&mut self, input: Input, area: Rect) -> Transition {
        match self.mode {
            Mode::Browsing => self.handle_browsing(input, area),
            Mode::Searching => self.handle_searching(input),
            Mode::Sorting => self.handle_sorting(input),
        }
    }

    fn handle_browsing(&mut self, input: Input, area: Rect) -> Transition {
        let page = self.browser.page_size(area) as isize;
        // In a grid, left and right are the horizontal axis; in a strip or an arc they are
        // the fast scroll, because there is nothing beside the cursor to move to.
        let horizontal = self.browser.layout.horizontal_steps();
        match input {
            Input::Up => self.browser.move_by(-self.step_up()),
            Input::Down => self.browser.move_by(self.step_up()),
            Input::Left => self.browser.move_by(if horizontal { -1 } else { -page }),
            Input::Right => self.browser.move_by(if horizontal { 1 } else { page }),
            Input::PageUp => self.browser.move_by(-page),
            Input::PageDown => self.browser.move_by(page),
            Input::Confirm => {
                if let Some(song) = self.selected() {
                    return Transition::Sing(song.id);
                }
            }
            Input::Back => return Transition::Pop,
            Input::Search => {
                self.mode = Mode::Searching;
            }
            Input::Sort => {
                self.mode = Mode::Sorting;
            }
            Input::CycleLayout => {
                self.browser.layout = self.browser.layout.next();
            }
            Input::Random => {
                // Deliberately not a random number: with no source of entropy in this crate,
                // the application supplies one by jumping the cursor. Here it steps by a
                // prime so repeated presses do not cycle a short loop.
                if !self.songs.is_empty() {
                    self.browser
                        .jump_to((self.browser.cursor() + 7919) % self.songs.len());
                }
            }
            Input::Type(_) | Input::Backspace => {}
        }
        Transition::None
    }

    /// How far one press of up or down moves.
    ///
    /// In a grid that is a whole row, because the cursor moving one cell sideways when you
    /// pressed down is disorienting.
    fn step_up(&self) -> isize {
        match self.browser.layout {
            Layout::Chessboard => 4,
            _ => 1,
        }
    }

    fn handle_searching(&mut self, input: Input) -> Transition {
        let before = self.keyboard.text().to_owned();
        match input {
            Input::Up => self.keyboard.navigate(0, -1),
            Input::Down => self.keyboard.navigate(0, 1),
            Input::Left => self.keyboard.navigate(-1, 0),
            Input::Right => self.keyboard.navigate(1, 0),
            Input::Confirm => {
                if self.keyboard.press() {
                    self.mode = Mode::Browsing;
                }
            }
            Input::Back | Input::Search => self.mode = Mode::Browsing,
            Input::Type(c) => self.keyboard.push(c),
            Input::Backspace => self.keyboard.backspace(),
            // Cycling which field is searched without leaving the keyboard: "I typed a lyric,
            // not a title" is a correction you make mid-search, not before it.
            Input::Sort => {
                self.field_cursor = (self.field_cursor + 1) % FIELDS.len();
                self.stale = true;
            }
            Input::CycleLayout => self.browser.layout = self.browser.layout.next(),
            Input::Random | Input::PageUp | Input::PageDown => {}
        }
        if self.keyboard.text() != before {
            // Every keystroke re-queries. At 3 ms for a prefix search over 30,000 songs that
            // is affordable, and it is what makes the list narrow as you type rather than
            // when you finish.
            self.stale = true;
        }
        Transition::None
    }

    fn handle_sorting(&mut self, input: Input) -> Transition {
        match input {
            Input::Up => {
                self.sort_cursor = (self.sort_cursor + SORTS.len() - 1) % SORTS.len();
                self.sort_chosen = true;
                self.stale = true;
            }
            Input::Down => {
                self.sort_cursor = (self.sort_cursor + 1) % SORTS.len();
                self.sort_chosen = true;
                self.stale = true;
            }
            Input::Left | Input::Right => {
                self.descending = !self.descending;
                self.stale = true;
            }
            Input::Confirm | Input::Back | Input::Sort => self.mode = Mode::Browsing,
            _ => {}
        }
        Transition::None
    }

    /// Clear the search and go back to the whole library.
    pub fn clear_search(&mut self) {
        self.keyboard.apply(Key::Clear);
        self.stale = true;
    }

    /// Go back to ranking by relevance on the next search.
    pub fn forget_chosen_sort(&mut self) {
        self.sort_chosen = false;
        self.stale = true;
    }

    /// Draw the screen. `cover` supplies an already-loaded texture for a song, or `None` while
    /// it is still being read.
    pub fn draw(
        &mut self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        cover: &dyn Fn(i64) -> Option<ImageId>,
    ) {
        let widgets = Widgets::new(style);
        let status = if self.songs.is_empty() {
            String::new()
        } else {
            format!("{} of {}", self.browser.cursor() + 1, self.songs.len())
        };
        let body = widgets.header(list, area, "Songs", &status);
        let body = widgets.footer(list, body, &self.hints());

        if self.songs.is_empty() {
            self.draw_empty(list, body, &widgets);
        } else {
            // The detail panel takes a fixed share on the left, so the list geometry does not
            // change when a song with a long title comes under the cursor.
            let (detail, rest) = body.cut_left((body.w * 0.34).min(560.0));
            self.draw_detail(list, detail.inset(style.gap(1.5)), style, cover);
            self.draw_list(list, rest.inset(style.gap(1.0)), style, cover);
        }

        match self.mode {
            Mode::Searching => self.draw_keyboard(list, area, style),
            Mode::Sorting => self.draw_sort_picker(list, area, style),
            Mode::Browsing => {}
        }
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        match self.mode {
            Mode::Browsing => vec![
                ("A", "Sing"),
                ("B", "Back"),
                ("X", "Search"),
                ("Y", "Sort"),
                ("LB/RB", "Layout"),
            ],
            Mode::Searching => vec![("A", "Type"), ("B", "Done"), ("Y", "Search in")],
            Mode::Sorting => vec![("A", "Choose"), ("<>", "Reverse"), ("B", "Back")],
        }
    }

    fn draw_empty(&self, list: &mut DrawList, area: Rect, widgets: &Widgets) {
        if self.keyboard.is_empty() {
            widgets.empty_state(
                list,
                area,
                "No songs yet",
                "Add a folder of songs in Options, or download some from USDB. Any folder \
                 with a .txt file in it counts as a song.",
            );
        } else {
            widgets.empty_state(
                list,
                area,
                "Nothing matched",
                "Press B to clear the search, or Y to search a different field — lyrics are \
                 indexed too, so a line you half-remember will find it.",
            );
        }
    }

    fn draw_list(
        &mut self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        cover: &dyn Fn(i64) -> Option<ImageId>,
    ) {
        let layout = self.browser.layout;
        let placements = self.browser.placements(area);
        let songs = &self.songs;
        // Clipped, so a row sliding in is cut at the edge rather than drawn over the header.
        list.clipped(area, |list| {
            for placement in &placements {
                let Some(song) = songs.get(placement.index) else {
                    continue;
                };
                match layout {
                    Layout::List => draw_row(list, style, placement.rect, song, placement.selected),
                    Layout::Chessboard | Layout::Roulette => draw_tile(
                        list,
                        style,
                        placement.rect,
                        song,
                        placement.selected,
                        placement.emphasis,
                        cover,
                    ),
                }
            }
        });
    }

    fn draw_detail(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        cover: &dyn Fn(i64) -> Option<ImageId>,
    ) {
        let Some(song) = self.selected() else {
            return;
        };
        let art = area.cut_top(area.w).0.fit_aspect(1.0);
        match cover(song.id) {
            Some(image) => {
                list.image_tinted(art, image, crate::Color::WHITE, style.metrics.radius);
            }
            None => {
                list.panel(art, style.surface_sunken, style.metrics.radius);
                list.text(
                    art,
                    initials(&song.artist),
                    TextStyle::new(art.h * 0.35, style.muted).centered().bold(),
                );
            }
        }

        let mut y = art.bottom() + style.gap(1.5);
        let line = style.text_size() * 1.35;
        list.text(
            Rect::new(area.x, y, area.w, line * 1.4),
            &song.title,
            TextStyle::new(style.scaled_text(1.2), style.text)
                .bold()
                .overflow(Overflow::Ellipsis),
        );
        y += line * 1.4;
        list.text(
            Rect::new(area.x, y, area.w, line),
            &song.artist,
            TextStyle::new(style.text_size(), style.accent).overflow(Overflow::Ellipsis),
        );
        y += line * 1.4;

        // Facts, not a wall of metadata: what you would want to know before choosing to sing
        // it. A missing field is left out rather than shown empty.
        // Most important first, because the panel is only as tall as the window leaves it and
        // the tail gets cut. A song that cannot be played is the one thing you must know
        // before pressing A, so it goes above the metadata rather than after it.
        let mut facts: Vec<(String, String)> = Vec::new();
        if !song.is_playable() {
            facts.push(("Warning".into(), "No audio file".into()));
        }
        facts.push(("Length".into(), format_duration(song.duration_secs)));
        facts.push((
            "Difficulty".into(),
            difficulty_label(song.difficulty).to_owned(),
        ));
        if song.is_duet {
            facts.push(("Mode".into(), "Duet".into()));
        }
        if let Some(year) = song.year {
            facts.push(("Year".into(), year.to_string()));
        }
        if let Some(language) = &song.language {
            facts.push(("Language".into(), language.clone()));
        }
        if let Some(genre) = &song.genre {
            facts.push(("Genre".into(), genre.clone()));
        }
        if song.times_played > 0 {
            facts.push(("Sung".into(), format!("{} times", song.times_played)));
        }

        for (label, value) in facts {
            if y + line > area.bottom() {
                break;
            }
            let row = Rect::new(area.x, y, area.w, line);
            let warning = label == "Warning";
            list.text(
                row,
                label,
                TextStyle::new(style.scaled_text(0.85), style.muted),
            );
            list.text(
                row,
                value,
                TextStyle::new(
                    style.scaled_text(0.85),
                    if warning { style.warning } else { style.text },
                )
                .align(Align::End),
            );
            y += line;
        }
    }

    fn draw_keyboard(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);

        let keys = self.keyboard.keys();
        let rows = self.keyboard.rows();
        let key_size = (area.w / 16.0).min(area.h / (rows as f32 + 6.0));
        let gap = key_size * 0.12;
        let grid_w = key_size * crate::keyboard::COLUMNS as f32;
        let grid_h = key_size * rows as f32;

        let card = area
            .anchored(
                Anchor::Center,
                grid_w + style.gap(4.0),
                grid_h + style.gap(11.0),
                0.0,
            )
            .offset(0.0, -area.h * 0.05);
        widgets.card(list, card);

        // The field, and what is being searched in it.
        let field = Rect::new(
            card.x + style.gap(2.0),
            card.y + style.gap(2.0),
            card.w - style.gap(4.0),
            style.gap(3.5),
        );
        list.panel(field, style.surface_sunken, style.metrics.radius);
        let shown = if self.keyboard.is_empty() {
            "Type to search\u{2026}".to_owned()
        } else {
            format!("{}\u{2502}", self.keyboard.text())
        };
        let text_color = if self.keyboard.is_empty() {
            style.muted
        } else {
            style.text
        };
        list.text(
            field.inset_xy(style.gap(1.0), 0.0),
            shown,
            TextStyle::new(style.text_size(), text_color).overflow(Overflow::Ellipsis),
        );

        let caption = Rect::new(
            field.x,
            field.bottom() + style.gap(0.4),
            field.w,
            style.gap(2.0),
        );
        list.text(
            caption,
            format!("Searching {}", FIELDS[self.field_cursor].1.to_lowercase()),
            TextStyle::new(style.scaled_text(0.8), style.muted),
        );
        list.text(
            caption,
            format!("{} found", self.songs.len()),
            TextStyle::new(style.scaled_text(0.8), style.muted).align(Align::End),
        );

        let grid_origin = Rect::new(
            card.center().x - grid_w / 2.0,
            caption.bottom() + style.gap(1.0),
            grid_w,
            grid_h,
        );
        for (index, key) in keys.iter().enumerate() {
            let (row, column) = Keyboard::position(index);
            let cell = Rect::new(
                grid_origin.x + column as f32 * key_size,
                grid_origin.y + row as f32 * key_size,
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
            let label = key.label();
            // A wide key's label has to shrink to fit a single cell.
            let size = if key.wide() {
                key_size * 0.24
            } else {
                key_size * 0.45
            };
            list.text(
                cell,
                label,
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
    }

    fn draw_sort_picker(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);

        let row_h = style.gap(3.0);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.4).min(620.0),
            row_h * (SORTS.len() as f32 + 2.5),
            0.0,
        );
        widgets.card(list, card);

        let inner = card.inset(style.gap(1.5));
        list.text(
            Rect::new(inner.x, inner.y, inner.w, row_h),
            "Sort by",
            TextStyle::new(style.scaled_text(1.2), style.text).bold(),
        );
        list.text(
            Rect::new(inner.x, inner.y, inner.w, row_h),
            if self.descending {
                "Z \u{2192} A"
            } else {
                "A \u{2192} Z"
            },
            TextStyle::new(style.text_size(), style.muted).align(Align::End),
        );
        if self.ranking() {
            // Otherwise a list that is not in the order the picker names is just a bug.
            list.text(
                Rect::new(inner.x, inner.bottom() - row_h, inner.w, row_h),
                "Best match first while searching",
                TextStyle::new(style.scaled_text(0.8), style.muted).centered(),
            );
        }

        for (index, (_, label)) in SORTS.iter().enumerate() {
            let row = Rect::new(
                inner.x,
                inner.y + row_h * (index as f32 + 1.5),
                inner.w,
                row_h,
            )
            .inset_xy(0.0, style.gap(0.2));
            widgets.row(list, row, label, "", index == self.sort_cursor);
        }
    }
}

/// One row of the list layout.
fn draw_row(list: &mut DrawList, style: &Style, rect: Rect, song: &SongEntry, selected: bool) {
    let rect = rect.inset_xy(0.0, style.gap(0.25));
    let radius = style.metrics.radius;
    list.panel(
        rect,
        if selected {
            style.accent
        } else {
            style.surface
        },
        radius,
    );

    let (text, secondary) = if selected {
        (style.on_accent, style.on_accent.alpha(0.75))
    } else {
        (style.text, style.muted)
    };
    let inner = rect.inset_xy(style.gap(1.2), style.gap(0.4));
    let (top, bottom) = inner.cut_top(inner.h * 0.58);
    list.text(
        top,
        &song.title,
        TextStyle::new(style.text_size(), text)
            .bold()
            .valign(VAlign::Bottom)
            .overflow(Overflow::Ellipsis),
    );
    list.text(
        bottom,
        &song.artist,
        TextStyle::new(style.scaled_text(0.8), secondary)
            .valign(VAlign::Top)
            .overflow(Overflow::Ellipsis),
    );
    // Length on the right, where the eye can compare down the column.
    list.text(
        inner,
        format_duration(song.duration_secs),
        TextStyle::new(style.scaled_text(0.8), secondary).align(Align::End),
    );
}

/// One cover in the grid or on the arc.
fn draw_tile(
    list: &mut DrawList,
    style: &Style,
    rect: Rect,
    song: &SongEntry,
    selected: bool,
    emphasis: f32,
    cover: &dyn Fn(i64) -> Option<ImageId>,
) {
    let radius = style.metrics.radius;
    // Distance dims as well as shrinks, so the arc reads as depth rather than as a row of
    // different-sized pictures.
    let tint = crate::Color::WHITE.alpha(0.45 + 0.55 * emphasis);
    match cover(song.id) {
        Some(image) => {
            list.image_tinted(rect, image, tint, radius);
        }
        None => {
            list.panel(
                rect,
                style.surface_sunken.fade(tint.a as f32 / 255.0),
                radius,
            );
            list.text(
                rect,
                initials(&song.artist),
                TextStyle::new(rect.h * 0.3, style.muted).centered().bold(),
            );
        }
    }

    if selected {
        list.outline(rect, style.accent, style.metrics.outline, radius);
        // The title goes under the cursor only. Labelling every cover turns a wall of artwork
        // into a wall of text, which is the thing the grid exists to avoid.
        let caption = Rect::new(rect.x, rect.bottom() - rect.h * 0.28, rect.w, rect.h * 0.28);
        list.panel(caption, style.scrim, radius);
        let inner = caption.inset_xy(style.gap(0.6), 0.0);
        let (top, bottom) = inner.cut_top(inner.h * 0.55);
        list.text(
            top,
            &song.title,
            TextStyle::new(inner.h * 0.34, crate::Color::WHITE)
                .bold()
                .centered()
                .valign(VAlign::Bottom)
                .overflow(Overflow::Ellipsis),
        );
        list.text(
            bottom,
            &song.artist,
            TextStyle::new(inner.h * 0.28, crate::Color::WHITE.alpha(0.75))
                .centered()
                .valign(VAlign::Top)
                .overflow(Overflow::Ellipsis),
        );
    }
}

/// Up to two initials, for the placeholder shown before a cover has loaded.
fn initials(artist: &str) -> String {
    artist
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// `m:ss`, or `h:mm:ss` for the occasional epic.
pub fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "\u{2013}".to_owned();
    }
    let total = seconds.round() as u64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

/// The computed difficulty as words rather than a number nobody can calibrate.
pub fn difficulty_label(difficulty: f64) -> &'static str {
    match difficulty {
        d if d < 0.2 => "Gentle",
        d if d < 0.4 => "Easy",
        d if d < 0.6 => "Moderate",
        d if d < 0.8 => "Hard",
        _ => "Brutal",
    }
}
