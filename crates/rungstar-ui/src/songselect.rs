//! The song browser: three layouts, live search, sorting, and the detail panel.
//!
//! The screen holds the result set and a cursor; it does not know how to query a library. When
//! the search text or the sort changes it raises a flag, the application runs the query and
//! hands the rows back. That keeps the screen pure — a test drives it with a list of songs and
//! reads back the display list — and it keeps the query off the render thread, which is what
//! makes typing into a 30,000 song library stay responsive.

use rungstar_library::{SearchField, SongEntry, SortKey};
use rungstar_party::Challenge;

use crate::browse::{Browser, Layout};
use crate::draw::{Align, DrawList, ImageId, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Point, Rect};
use crate::keyboard::{Key, Keyboard};
use crate::screen::{ControlState, Transition, Widgets};
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
    /// The filter panel.
    Filtering,
    /// Choosing a challenge to sing under.
    Challenging,
    /// The per-song menu.
    Menu,
}

/// Semantic inputs the screen understands. Deliberately not device events: the same enum comes
/// from a keyboard, a gamepad or a touch.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// Cycle what the list is narrowed to.
    CycleFilter,
    /// Switch between the song as recorded and its backing track.
    ToggleInstrumental,
    /// Open the sort picker.
    Sort,
    /// Open the menu for the song under the cursor.
    ContextMenu,
    Random,
    PageUp,
    PageDown,
    /// A character from a physical keyboard, while searching.
    Type(char),
    Backspace,
    /// Finish editing. Enter on a physical keyboard, where it means "done" rather than
    /// "press the highlighted key" — nobody typing on a real keyboard is looking at the
    /// on-screen one's cursor.
    Submit,
    /// The pointer moved. Moves the cursor to whatever is under it, so the highlight follows
    /// the mouse exactly as it follows the stick.
    Hover(Point),
    /// The pointer was clicked. Selects what is under it and activates it.
    Click(Point),
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

/// What the list is narrowed to, beyond the search text.
///
/// One cycling control rather than a tree of checkboxes: these are the three questions people
/// actually ask of a song list, and a filter you have to go looking for is a filter nobody
/// uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Narrow {
    #[default]
    Everything,
    Duets,
    Solos,
    /// Songs with a video, which is what most people mean by "the good ones".
    WithVideo,
    /// Songs whose audio file is actually there.
    Playable,
}

impl Narrow {
    pub const ALL: [Narrow; 5] = [
        Narrow::Everything,
        Narrow::Duets,
        Narrow::Solos,
        Narrow::WithVideo,
        Narrow::Playable,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Everything => "Everything",
            Self::Duets => "Duets only",
            Self::Solos => "Solos only",
            Self::WithVideo => "With a video",
            Self::Playable => "Playable only",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|n| *n == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Turn this into the library's own filter.
    pub fn filters(self) -> rungstar_library::Filters {
        let mut filters = rungstar_library::Filters::default();
        match self {
            Self::Everything => {}
            Self::Duets => filters.duet = Some(true),
            Self::Solos => filters.duet = Some(false),
            Self::WithVideo => filters.has_video = Some(true),
            Self::Playable => filters.playable = Some(true),
        }
        filters
    }
}

/// A category the list can be narrowed by, beyond the search text.
///
/// These are the questions a song library is actually asked at a party -- "what have we got in
/// German", "anything from the eighties" -- and they are answers the index already holds. The
/// values come from the library rather than from a list written here, so a library of nothing
/// but schlager offers exactly the genres it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facet {
    /// Duets, solos, has a video, is playable. One at a time, unlike the others.
    Kind,
    /// How hard the songs are to sing, in the same words the song panel uses.
    Difficulty,
    Genre,
    Language,
    Decade,
    Edition,
    Folder,
    Creator,
}

impl Facet {
    pub const ALL: [Facet; 8] = [
        Facet::Kind,
        Facet::Difficulty,
        Facet::Genre,
        Facet::Language,
        Facet::Decade,
        Facet::Edition,
        Facet::Folder,
        Facet::Creator,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Kind => "Kind",
            Self::Difficulty => "Difficulty",
            Self::Genre => "Genre",
            Self::Language => "Language",
            Self::Decade => "Decade",
            Self::Edition => "Edition",
            Self::Folder => "Folder",
            Self::Creator => "Creator",
        }
    }

    /// The library column its values come from, or `None` when the screen supplies them.
    pub fn column(self) -> Option<&'static str> {
        match self {
            Self::Kind => None,
            Self::Difficulty => Some("difficulty"),
            Self::Genre => Some("genre"),
            Self::Language => Some("language"),
            Self::Decade => Some("decade"),
            Self::Edition => Some("edition"),
            Self::Folder => Some("folder"),
            Self::Creator => Some("creator"),
        }
    }

    /// How a stored value is shown. A decade is stored as its first year, and a difficulty
    /// band as a key that does not change with how it is displayed.
    pub fn label(self, value: &str) -> String {
        match self {
            Self::Decade => format!("{value}s"),
            Self::Difficulty => rungstar_library::DifficultyBand::from_key(value)
                .map_or_else(|| value.to_owned(), |band| band.label().to_owned()),
            _ => value.to_owned(),
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// The values each facet has in this library, with how many songs each covers.
///
/// Supplied by the application, because the screen has no database. Counts are of the whole
/// library rather than of the current results: a filter list that empties itself as you use it
/// cannot be used to widen a search again.
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

/// Which fields the search box can be pointed at.
pub const FIELDS: [(SearchField, &str); 4] = [
    (SearchField::All, "Everything"),
    (SearchField::Artist, "Artist"),
    (SearchField::Title, "Title"),
    (SearchField::Lyrics, "Lyrics"),
];

/// Something the pointer can be over.
///
/// Recorded while drawing rather than recomputed, so hit testing cannot drift from the layout
/// — the bug where a button moves and its clickable area stays behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Song(usize),
    Key(usize),
    Sort(usize),
    Menu(usize),
    /// A row in the filter panel's category column.
    Category(usize),
    /// A row in the filter panel's value column.
    Value(usize),
    /// A row in the challenge picker.
    Challenge(usize),
}

/// What the per-song menu offers.
///
/// UltraStar has one of these too, and it is where the things you do to a song rather than
/// with it belong — so they are not buttons cluttering a screen you spend most of your time
/// scrolling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SongAction {
    Sing,
    /// Play from the medley point, or the preview point when there is none.
    SingFromChorus,
    ToggleFavourite,
    /// Choose how the next song is sung: blind, deaf, first to 2000, and the rest.
    PickChallenge,
    /// Copy what the browser knows to the clipboard, for reporting a broken file.
    ShowDetails,
    /// Open the song in the editor.
    Edit,
    OpenFolder,
}

impl SongAction {
    pub const ALL: [SongAction; 7] = [
        SongAction::Sing,
        SongAction::SingFromChorus,
        SongAction::ToggleFavourite,
        SongAction::PickChallenge,
        SongAction::ShowDetails,
        SongAction::Edit,
        SongAction::OpenFolder,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sing => "Sing",
            Self::SingFromChorus => "Sing from the chorus",
            Self::ToggleFavourite => "Favourite",
            Self::PickChallenge => "Sing it how\u{2026}",
            Self::ShowDetails => "Song details",
            Self::Edit => "Edit this song",
            Self::OpenFolder => "Open the song folder",
        }
    }
}

/// The song browser.
pub struct SongSelect {
    pub browser: Browser,
    songs: Vec<SongEntry>,
    keyboard: Keyboard,
    mode: Mode,
    sort_cursor: usize,
    /// Whether the player chose the sort, as opposed to it being the default.
    sort_chosen: bool,
    /// What the list is narrowed to.
    narrow: Narrow,
    /// Which challenge the next song is sung under, as an index into `Challenge::ALL`.
    challenge: usize,
    /// Values chosen per facet, in `Facet::ALL` order. Empty means no constraint.
    picked: Vec<Vec<String>>,
    /// The values each facet offers, filled in by the application.
    facets: FacetValues,
    /// Set when the facet lists need fetching, which is once and after every scan.
    facets_stale: bool,
    facet_cursor: usize,
    value_cursor: usize,
    /// `true` while the value column has focus rather than the category column.
    on_values: bool,
    field_cursor: usize,
    descending: bool,
    /// Set when the query the application should be running has changed.
    stale: bool,
    /// Text the current `songs` were fetched for, so a slow query landing late can be ignored.
    fetched_for: String,
    /// Clickable areas from the last frame.
    regions: Vec<(Rect, Region)>,
    menu_cursor: usize,
    /// Set when the player chose something from the song menu.
    pub chosen: Option<SongAction>,
    /// Whether to label the on-screen hints with gamepad buttons or keyboard keys.
    pub gamepad: bool,
    /// The selected song's best scores, highest first. Supplied by the application, which is
    /// the only thing that knows who has sung what.
    pub highscores: Vec<(String, i32)>,
    /// What a scan is doing, when one is running. Shown instead of "no songs", because a
    /// first run reaches this screen before the library exists.
    pub scanning: Option<String>,
    /// Whether songs are sung to their backing track rather than to the record.
    ///
    /// Set by the application from the saved setting, never by the screen. The same mode is
    /// also on the options page, and two places holding their own copy of one answer is how
    /// they end up disagreeing -- so this screen asks for a change and is told the result.
    pub instrumental: bool,
    /// Set when the player asked to switch. Consumed by the application, which owns the
    /// setting and the folder behind it.
    pub instrumental_toggled: bool,
    /// Whether there are any backing tracks to switch to.
    ///
    /// Supplied by the application, which is what has looked at the folder. When there are
    /// none the control is not offered at all -- a button that says the mode exists and then
    /// refuses to enter it is worse than no button.
    pub instrumental_available: bool,
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
            narrow: Narrow::default(),
            challenge: 0,
            picked: vec![Vec::new(); Facet::ALL.len()],
            facets: FacetValues::new(),
            facets_stale: true,
            facet_cursor: 0,
            value_cursor: 0,
            on_values: false,
            field_cursor: 0,
            descending: false,
            stale: true,
            fetched_for: String::new(),
            regions: Vec::new(),
            menu_cursor: 0,
            chosen: None,
            gamepad: false,
            highscores: Vec::new(),
            scanning: None,
            instrumental: false,
            instrumental_toggled: false,
            instrumental_available: false,
        }
    }

    /// Mark the results stale, so the next frame re-queries.
    ///
    /// Used when a scan has changed the library underneath the browser.
    pub fn invalidate(&mut self) {
        self.stale = true;
        // A scan can add a genre that was not there before, so the filter lists go with it.
        self.facets_stale = true;
    }

    /// Whether the application should fetch the facet lists.
    pub fn needs_facets(&self) -> bool {
        self.facets_stale
    }

    pub fn set_facets(&mut self, facets: FacetValues) {
        self.facets = facets;
        self.facets_stale = false;
        // A value that no longer exists cannot stay chosen, or the list stays empty with no
        // visible reason why.
        for (index, facet) in Facet::ALL.iter().enumerate() {
            if facet.column().is_none() {
                continue;
            }
            let available = self.facets.get(*facet);
            self.picked[index].retain(|value| available.iter().any(|(v, _)| v == value));
        }
    }

    /// What is under a point, topmost first — the overlays are recorded last and so win.
    fn region_at(&self, point: Point) -> Option<Region> {
        self.regions
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, region)| *region)
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether a text field has focus.
    ///
    /// While it does, letter keys are text and nothing else. Without this, typing "r" jumps
    /// to a random song, "m" opens the context menu and "f" closes the search — every
    /// single-letter shortcut fires underneath the thing you are typing into.
    pub fn wants_text(&self) -> bool {
        self.mode == Mode::Searching
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

    /// What the list is narrowed to.
    pub fn narrow(&self) -> Narrow {
        self.narrow
    }

    /// The library filters for the current narrowing.
    ///
    /// Values within a facet are an "any of these" match and the facets are ANDed, which is
    /// what the checkboxes look like they do: German *or* Swedish, and from the eighties.
    pub fn filters(&self) -> rungstar_library::Filters {
        let mut filters = self.narrow.filters();
        for (index, facet) in Facet::ALL.iter().enumerate() {
            let chosen = &self.picked[index];
            if chosen.is_empty() {
                continue;
            }
            match facet {
                Facet::Kind => {}
                Facet::Difficulty => {
                    filters.difficulty = chosen
                        .iter()
                        .filter_map(|key| rungstar_library::DifficultyBand::from_key(key))
                        .collect();
                }
                Facet::Genre => filters.genres.clone_from(chosen),
                Facet::Language => filters.languages.clone_from(chosen),
                Facet::Edition => filters.editions.clone_from(chosen),
                Facet::Folder => filters.folders.clone_from(chosen),
                Facet::Creator => filters.creators.clone_from(chosen),
                Facet::Decade => {
                    filters.decades = chosen.iter().filter_map(|d| d.parse().ok()).collect();
                }
            }
        }
        filters
    }

    /// What is being filtered out, in a few words, or `None` when nothing is.
    ///
    /// One name and a count rather than a list of every value: "Language +3" fits in a header
    /// and still says that something is hidden, which is the part that matters.
    pub fn filter_summary(&self) -> Option<String> {
        let mut named: Option<String> = None;
        let mut extra = 0;
        if self.narrow != Narrow::Everything {
            named = Some(self.narrow.label().to_owned());
        }
        for (index, facet) in Facet::ALL.iter().enumerate() {
            for value in &self.picked[index] {
                match &named {
                    None => named = Some(facet.label(value)),
                    Some(_) => extra += 1,
                }
            }
        }
        match (named, extra) {
            (None, _) => None,
            (Some(name), 0) => Some(name),
            (Some(name), n) => Some(format!("{name} +{n}")),
        }
    }

    /// How many values are chosen across every facet, for the badge on the filter button.
    pub fn active_filters(&self) -> usize {
        let kinds = usize::from(self.narrow != Narrow::Everything);
        kinds + self.picked.iter().map(Vec::len).sum::<usize>()
    }

    /// Ask to switch between the record and the backing track.
    ///
    /// Silently ignored when there are no backing tracks, which is also when the control is
    /// not drawn. The results go stale because the list itself changes: a song with no backing
    /// track cannot be sung to one, so it is not in the list while the mode is on.
    pub fn toggle_instrumental(&mut self) {
        if !self.instrumental_available {
            return;
        }
        self.instrumental_toggled = true;
        self.stale = true;
        crate::chime::emit(crate::chime::Chime::Select);
    }

    /// Put every filter back to showing everything.
    pub fn clear_filters(&mut self) {
        self.narrow = Narrow::Everything;
        for picked in &mut self.picked {
            picked.clear();
        }
        self.stale = true;
    }

    /// The rows the filter panel shows for a facet: label, count, and whether it is chosen.
    fn rows_for(&self, facet: Facet) -> Vec<(String, String, bool)> {
        match facet {
            Facet::Kind => Narrow::ALL
                .iter()
                .map(|narrow| {
                    (
                        narrow.label().to_owned(),
                        String::new(),
                        *narrow == self.narrow,
                    )
                })
                .collect(),
            _ => {
                let chosen = &self.picked[facet.index()];
                self.facets
                    .get(facet)
                    .iter()
                    .map(|(value, count)| {
                        (
                            facet.label(value),
                            count.to_string(),
                            chosen.iter().any(|c| c == value),
                        )
                    })
                    .collect()
            }
        }
    }

    /// Turn the value at `index` of `facet` on or off.
    fn toggle_value(&mut self, facet: Facet, index: usize) {
        match facet {
            // One kind at a time: "duets only" and "solos only" together is an empty list.
            Facet::Kind => {
                let Some(narrow) = Narrow::ALL.get(index).copied() else {
                    return;
                };
                // Choosing the one already chosen turns it off rather than doing nothing.
                self.narrow = if self.narrow == narrow {
                    Narrow::Everything
                } else {
                    narrow
                };
            }
            _ => {
                let Some((value, _)) = self.facets.get(facet).get(index).cloned() else {
                    return;
                };
                let chosen = &mut self.picked[facet.index()];
                match chosen.iter().position(|c| *c == value) {
                    Some(at) => {
                        chosen.remove(at);
                    }
                    None => chosen.push(value),
                }
            }
        }
        self.stale = true;
    }

    /// The filter category under the cursor, for tests and for the panel.
    pub fn facet_title(&self) -> &'static str {
        self.facet().title()
    }

    fn facet(&self) -> Facet {
        Facet::ALL[self.facet_cursor.min(Facet::ALL.len() - 1)]
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
        if let Input::Hover(point) | Input::Click(point) = input {
            let clicked = matches!(input, Input::Click(_));
            return self.handle_pointer(point, clicked);
        }
        match self.mode {
            Mode::Browsing => self.handle_browsing(input, area),
            Mode::Searching => self.handle_searching(input),
            Mode::Sorting => self.handle_sorting(input),
            Mode::Filtering => self.handle_filtering(input),
            Mode::Challenging => self.handle_challenging(input),
            Mode::Menu => self.handle_menu(input),
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
            Input::Confirm | Input::Submit => {
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
            Input::ContextMenu => {
                if !self.songs.is_empty() {
                    self.menu_cursor = 0;
                    self.mode = Mode::Menu;
                }
            }
            Input::CycleLayout => {
                self.browser.layout = self.browser.layout.next();
            }
            Input::CycleFilter => {
                self.mode = Mode::Filtering;
                self.on_values = false;
            }
            Input::ToggleInstrumental => self.toggle_instrumental(),
            Input::Random => {
                // Deliberately not a random number: with no source of entropy in this crate,
                // the application supplies one by jumping the cursor. Here it steps by a
                // prime so repeated presses do not cycle a short loop.
                if !self.songs.is_empty() {
                    self.browser
                        .jump_to((self.browser.cursor() + 7919) % self.songs.len());
                    crate::chime::emit(crate::chime::Chime::Move);
                }
            }
            Input::Type(_) | Input::Backspace | Input::Hover(_) | Input::Click(_) => {}
        }
        Transition::None
    }

    /// Move the cursor to whatever the pointer is over, and act on it if it was clicked.
    fn handle_pointer(&mut self, point: Point, clicked: bool) -> Transition {
        let region = self.region_at(point);

        // An overlay is modal. While the keyboard, the sort picker or the song menu is up,
        // the list behind it is not clickable — otherwise a click aimed just past the dialog
        // selects a song, or starts one, which is the last thing a search box should do.
        // Clicking away closes the overlay, as it does everywhere else.
        if self.mode != Mode::Browsing {
            let on_overlay = matches!(
                region,
                Some(Region::Key(_)) | Some(Region::Sort(_)) | Some(Region::Menu(_))
            );
            if !on_overlay {
                if clicked {
                    self.mode = Mode::Browsing;
                }
                return Transition::None;
            }
        }

        match region {
            Some(Region::Song(index)) => {
                // Hovering deliberately does *not* move the cursor. In the list and the
                // roulette the cursor is always centred and the songs scroll past it, so
                // selecting on hover would drag the list out from under the pointer and you
                // would click a different song than the one you aimed at.
                if clicked {
                    if index == self.browser.cursor() {
                        // Second click on the same song: sing it. The first only selects, so
                        // a stray click never starts a song.
                        if let Some(song) = self.selected() {
                            return Transition::Sing(song.id);
                        }
                    } else {
                        // A click that only selects. Deliberate, so it makes the same noise as
                        // steering there would have.
                        if index != self.browser.cursor() {
                            crate::chime::emit(crate::chime::Chime::Move);
                        }
                        self.browser.jump_to(index);
                    }
                }
            }
            Some(Region::Key(index)) => {
                self.keyboard.set_cursor(index);
                if clicked && self.keyboard.press() {
                    self.mode = Mode::Browsing;
                }
                if clicked {
                    self.stale = true;
                }
            }
            Some(Region::Menu(index)) => {
                self.menu_cursor = index;
                if clicked {
                    return self.handle_menu(Input::Confirm);
                }
            }
            Some(Region::Challenge(index)) => {
                self.challenge = index.min(Challenge::ALL.len() - 1);
                if clicked {
                    self.mode = Mode::Browsing;
                }
            }
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
            Some(Region::Sort(index)) => {
                if index != self.sort_cursor {
                    self.sort_cursor = index;
                    self.sort_chosen = true;
                    self.stale = true;
                }
                if clicked {
                    self.mode = Mode::Browsing;
                }
            }
            None => {}
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

    fn handle_menu(&mut self, input: Input) -> Transition {
        match input {
            Input::Up => {
                self.menu_cursor =
                    (self.menu_cursor + SongAction::ALL.len() - 1) % SongAction::ALL.len();
            }
            Input::Down => {
                self.menu_cursor = (self.menu_cursor + 1) % SongAction::ALL.len();
            }
            Input::Confirm | Input::Submit => {
                let action = SongAction::ALL[self.menu_cursor];
                self.mode = Mode::Browsing;
                // Sing is the one the browser already knows how to do; the rest go back to
                // the application, which is the only thing that can open a folder.
                if action == SongAction::Sing {
                    if let Some(song) = self.selected() {
                        return Transition::Sing(song.id);
                    }
                }
                self.chosen = Some(action);
            }
            Input::Back | Input::ContextMenu => self.mode = Mode::Browsing,
            _ => {}
        }
        Transition::None
    }

    /// Take whatever the song menu chose, if anything.
    pub fn take_choice(&mut self) -> Option<(SongAction, i64)> {
        let action = self.chosen.take()?;
        let id = self.selected()?.id;
        Some((action, id))
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
            Input::Back | Input::Search | Input::Submit => self.mode = Mode::Browsing,
            Input::Type(c) => self.keyboard.push(c),
            Input::Backspace => self.keyboard.backspace(),
            // Cycling which field is searched without leaving the keyboard: "I typed a lyric,
            // not a title" is a correction you make mid-search, not before it.
            Input::Sort => {
                self.field_cursor = (self.field_cursor + 1) % FIELDS.len();
                self.stale = true;
            }
            // Nothing here reaches past the dialog. Cycling the browse layout behind an open
            // search box is a change you cannot see and did not ask for.
            Input::CycleLayout
            | Input::CycleFilter
            | Input::ToggleInstrumental
            | Input::Random
            | Input::PageUp
            | Input::PageDown
            | Input::ContextMenu
            | Input::Hover(_)
            | Input::Click(_) => {}
        }
        if self.keyboard.text() != before {
            // Every keystroke re-queries. At 3 ms for a prefix search over 30,000 songs that
            // is affordable, and it is what makes the list narrow as you type rather than
            // when you finish.
            self.stale = true;
        }
        Transition::None
    }

    /// The challenge the next song is sung under.
    pub fn challenge(&self) -> &'static Challenge {
        &Challenge::ALL[self.challenge.min(Challenge::ALL.len() - 1)]
    }

    /// Open the challenge picker.
    pub fn pick_challenge(&mut self) {
        self.mode = Mode::Challenging;
    }

    fn handle_challenging(&mut self, input: Input) -> Transition {
        let count = Challenge::ALL.len();
        match input {
            Input::Up => self.challenge = (self.challenge + count - 1) % count,
            Input::Down => self.challenge = (self.challenge + 1) % count,
            Input::PageUp => self.challenge = self.challenge.saturating_sub(6),
            Input::PageDown => self.challenge = (self.challenge + 6).min(count - 1),
            // No cancel that reverts: the cursor *is* the choice, so moving it has already
            // chosen and pretending otherwise would need a second confirm on every row.
            Input::Confirm | Input::Submit | Input::Back | Input::ContextMenu => {
                self.mode = Mode::Browsing
            }
            _ => {}
        }
        Transition::None
    }

    /// The filter panel: categories on the left, their values on the right.
    fn handle_filtering(&mut self, input: Input) -> Transition {
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
            // Hunting through seven categories for the one still narrowing the list is the
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
            Input::Confirm | Input::Submit | Input::Back | Input::Sort => {
                self.mode = Mode::Browsing
            }
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
        self.regions.clear();
        // A list row holds a title over an artist, so how short it may be is a fact about the
        // text — and `browse` has no theme in it to work that out for itself.
        self.browser.row_min = row_min(style);
        let widgets = Widgets::new(style);
        let counted = if self.songs.is_empty() {
            String::new()
        } else {
            format!("{} of {}", self.browser.cursor() + 1, self.songs.len())
        };
        // Saying what is being hidden, because a list quietly missing songs is
        // indistinguishable from a library missing them.
        let status = match (self.filter_summary(), self.challenge) {
            (Some(filters), 0) => format!("{filters}  ·  {counted}"),
            (Some(filters), _) => format!("{}  ·  {filters}  ·  {counted}", self.challenge().name),
            (None, 0) => counted,
            (None, _) => format!("{}  ·  {counted}", self.challenge().name),
        };
        // In front of everything else, because it changes what will be heard rather than what
        // is listed, and a list quietly playing backing tracks is the surprise worth avoiding.
        let status = if self.instrumental {
            format!("No vocals  ·  {status}")
        } else {
            status
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

        // The overlays cover the list, so their regions are recorded after it and win.
        let mut overlay = Vec::new();
        match self.mode {
            Mode::Searching => self.draw_keyboard(list, area, style, &mut overlay),
            Mode::Sorting => self.draw_sort_picker(list, area, style, &mut overlay),
            Mode::Filtering => self.draw_filters(list, area, style, &mut overlay),
            Mode::Challenging => self.draw_challenges(list, area, style, &mut overlay),
            Mode::Menu => self.draw_menu(list, area, style, &mut overlay),
            Mode::Browsing => {}
        }
        self.regions.extend(overlay);
    }

    /// What the buttons do right now, named for whatever is in the player's hands.
    ///
    /// Labelling a keyboard key "X" is worse than no hint at all: it says a button exists and
    /// then gives the wrong name for it.
    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        let pad = self.gamepad;
        let confirm = if pad { "A" } else { "Enter" };
        let back = if pad { "B" } else { "Esc" };
        let search = if pad { "X" } else { "F" };
        let sort = if pad { "Y" } else { "F3" };
        let layout = if pad { "LB/RB" } else { "Tab" };
        match self.mode {
            Mode::Browsing => {
                let mut hints = vec![
                    (confirm, "Sing"),
                    (back, "Back"),
                    (search, "Search"),
                    (sort, "Sort"),
                    (layout, "Layout"),
                    (if pad { "LT" } else { "D" }, "Filter"),
                ];
                // Only when there is something to switch to, and it says which way it goes
                // rather than what the mode is called: the header says which mode is on.
                if self.instrumental_available {
                    hints.push((
                        if pad { "RS" } else { "V" },
                        if self.instrumental {
                            "Vocals on"
                        } else {
                            "No vocals"
                        },
                    ));
                }
                hints
            }
            Mode::Searching => vec![(confirm, "Press key"), (back, "Done"), (sort, "Search in")],
            Mode::Sorting => vec![
                (confirm, "Choose"),
                ("\u{2190}\u{2192}", "Reverse"),
                (back, "Back"),
            ],
            Mode::Filtering => vec![
                (confirm, "Toggle"),
                ("\u{2190}\u{2192}", "Column"),
                (search, "Clear all"),
                (back, "Done"),
            ],
            Mode::Challenging => vec![(confirm, "Choose"), (back, "Back")],
            Mode::Menu => vec![(confirm, "Choose"), (back, "Back")],
        }
    }

    fn draw_empty(&self, list: &mut DrawList, area: Rect, widgets: &Widgets) {
        if let Some(progress) = &self.scanning {
            // A first run arrives here before the library exists. Saying what is happening is
            // the difference between waiting and giving up.
            widgets.empty_state(list, area, "Finding your songs", progress);
            return;
        }
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
        for placement in &placements {
            self.regions
                .push((placement.rect, Region::Song(placement.index)));
        }
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

        // The song's table, which is the point of keeping scores at all: seeing what there is
        // to beat before choosing.
        if !self.highscores.is_empty() && y + line * 2.0 < area.bottom() {
            y += line * 0.5;
            list.text(
                Rect::new(area.x, y, area.w, line),
                "Best scores",
                TextStyle::new(style.scaled_text(0.8), style.muted).bold(),
            );
            y += line;
            for (place, (name, points)) in self.highscores.iter().enumerate() {
                if y + line > area.bottom() {
                    break;
                }
                let row = Rect::new(area.x, y, area.w, line);
                let colour = if place == 0 { style.accent } else { style.text };
                list.text(
                    row,
                    format!("{}. {name}", place + 1),
                    TextStyle::new(style.scaled_text(0.85), colour).overflow(Overflow::Ellipsis),
                );
                list.text(
                    row,
                    points.to_string(),
                    TextStyle::new(style.scaled_text(0.85), colour).align(Align::End),
                );
                y += line;
            }
        }
    }

    fn draw_keyboard(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
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
            regions.push((cell, Region::Key(index)));
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

    fn draw_menu(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let row_h = style.gap(3.4);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.36).min(680.0),
            row_h * (SongAction::ALL.len() as f32 + 2.2),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(1.5));

        let title = self
            .selected()
            .map(|s| s.display_name())
            .unwrap_or_default();
        list.text(
            Rect::new(inner.x, inner.y, inner.w, row_h),
            title,
            TextStyle::new(style.scaled_text(1.0), style.text)
                .bold()
                .overflow(Overflow::Ellipsis),
        );

        for (index, action) in SongAction::ALL.iter().enumerate() {
            let row = Rect::new(
                inner.x,
                inner.y + row_h * (index as f32 + 1.4),
                inner.w,
                row_h,
            )
            .inset_xy(0.0, style.gap(0.2));
            regions.push((row, Region::Menu(index)));
            widgets.row(list, row, action.label(), "", index == self.menu_cursor);
        }
    }

    /// The filter panel.
    ///
    /// Two columns, because the alternative is one list of every genre, language and edition
    /// in the library run together -- eight thousand songs is 354 genres and 295 editions, and
    /// a flat list of that is not a filter, it is a haystack.
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
            // The count of what is chosen in a category, so a filter set three categories ago
            // is still visible from here.
            let chosen = match facet {
                Facet::Kind => usize::from(self.narrow != Narrow::Everything),
                _ => self.picked[index].len(),
            };
            let state = match (selected, self.on_values) {
                (true, false) => ControlState::Active,
                (true, true) => ControlState::Context,
                (false, _) => ControlState::Idle,
            };
            widgets.row_state(
                list,
                row,
                facet.title(),
                &if chosen == 0 {
                    String::new()
                } else {
                    chosen.to_string()
                },
                state,
            );
        }

        let rows = self.rows_for(self.facet());
        if rows.is_empty() {
            list.text(
                values,
                "Nothing in the library has one",
                TextStyle::new(style.text_size(), style.muted).centered(),
            );
            return;
        }

        let visible = ((values.h / row_h).floor() as usize).max(1);
        let first = self
            .value_cursor
            .saturating_sub(visible.saturating_sub(2))
            .min(rows.len().saturating_sub(visible.min(rows.len())));

        list.clipped(values, |list| {
            for (offset, (label, count, chosen)) in
                rows.iter().skip(first).take(visible).enumerate()
            {
                let index = first + offset;
                let row = Rect::new(values.x, values.y + row_h * offset as f32, values.w, row_h)
                    .inset_xy(0.0, style.gap(0.2));
                regions.push((row, Region::Value(index)));
                let selected = self.on_values && index == self.value_cursor;
                let palette = widgets.selectable(
                    list,
                    row,
                    if selected {
                        ControlState::Active
                    } else if *chosen {
                        ControlState::Chosen
                    } else {
                        ControlState::Idle
                    },
                );

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
                    TextStyle::new(style.text_size(), palette.text).overflow(Overflow::Ellipsis),
                );
                list.text(
                    number,
                    count,
                    TextStyle::new(style.scaled_text(0.82), palette.muted).align(Align::End),
                );
            }
        });

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

    /// The challenge picker: fifteen ways to sing the same song.
    fn draw_challenges(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);

        let row_h = style.gap(2.9);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.52).min(820.0),
            (row_h * (Challenge::ALL.len() as f32 + 3.4)).min(area.h * 0.92),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(1.6));
        let (heading, rest) = inner.cut_top(row_h);
        list.text(
            heading,
            "Sing it how",
            TextStyle::new(style.scaled_text(1.2), style.text).bold(),
        );
        // The blurb for the row under the cursor, so what a mode does is readable before it is
        // chosen rather than discovered halfway through a song.
        let (rows_area, blurb) = rest.cut_bottom(row_h * 1.6);
        list.text(
            blurb,
            self.challenge().blurb,
            TextStyle::new(style.scaled_text(0.82), style.muted).valign(VAlign::Middle),
        );

        let visible = ((rows_area.h / row_h).floor() as usize).max(1);
        let first = self
            .challenge
            .saturating_sub(visible.saturating_sub(2))
            .min(Challenge::ALL.len().saturating_sub(visible));
        list.clipped(rows_area, |list| {
            for (offset, challenge) in Challenge::ALL.iter().skip(first).take(visible).enumerate() {
                let index = first + offset;
                let row = Rect::new(
                    rows_area.x,
                    rows_area.y + row_h * offset as f32,
                    rows_area.w,
                    row_h,
                )
                .inset_xy(0.0, style.gap(0.2));
                regions.push((row, Region::Challenge(index)));
                widgets.row(list, row, challenge.name, "", index == self.challenge);
            }
        });
    }

    fn draw_sort_picker(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
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
            regions.push((row, Region::Sort(index)));
            widgets.row(list, row, label, "", index == self.sort_cursor);
        }
    }
}

/// One row of the list layout.
/// How short a list row may be, given the two lines of text it holds.
fn row_min(style: &Style) -> f32 {
    style.row_height(&[style.text_size(), style.scaled_text(0.8)]) + style.gap(0.6)
}

fn draw_row(list: &mut DrawList, style: &Style, rect: Rect, song: &SongEntry, selected: bool) {
    let rect = rect.inset_xy(0.0, style.gap(0.25));
    let palette = Widgets::new(style).selectable(
        list,
        rect,
        if selected {
            ControlState::Active
        } else {
            ControlState::Idle
        },
    );

    let inner = rect.inset_xy(style.gap(1.2), style.gap(0.3));
    let lines = style.stack(inner, &[style.text_size(), style.scaled_text(0.8)]);
    list.text(
        lines[0],
        &song.title,
        TextStyle::new(style.text_size(), palette.text)
            .bold()
            .valign(VAlign::Bottom)
            .overflow(Overflow::Ellipsis),
    );
    list.text(
        lines[1],
        &song.artist,
        TextStyle::new(style.scaled_text(0.8), palette.muted)
            .valign(VAlign::Top)
            .overflow(Overflow::Ellipsis),
    );
    // Length on the right, where the eye can compare down the column.
    list.text(
        inner,
        format_duration(song.duration_secs),
        TextStyle::new(style.scaled_text(0.8), palette.muted).align(Align::End),
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
///
/// Deferred to the library's own bands rather than repeated here, because the filter tree
/// offers those bands by name: two copies of one scale is how the panel comes to call a song
/// Moderate while the filter has it under Hard.
pub fn difficulty_label(difficulty: f64) -> &'static str {
    rungstar_library::DifficultyBand::of(difficulty).label()
}
