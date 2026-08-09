//! The on-screen keyboard, and the text field it edits.
//!
//! This exists because the game has to be fully usable from a controller, and searching a
//! 30,000 song library without typing is not usable. UltraStar Deluxe has no on-screen
//! keyboard at all — its "controller support" simulates keystrokes, so searching means
//! reaching for a keyboard.
//!
//! Layout is a grid so d-pad navigation is obvious, and moving off an edge wraps to the other
//! side rather than sticking. Nothing here draws: it reports which key is where and what the
//! text is now, and a screen turns that into rectangles.

/// Which set of keys is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Letters,
    /// The same letters in upper case.
    ///
    /// Not needed to search — that is case-insensitive — but a password is not, and without
    /// this a capital in one simply cannot be typed from a controller.
    Capitals,
    /// Digits and punctuation. Everything printable in ASCII, because this is also how a
    /// password gets typed and picking a subset means picking whose password works.
    Symbols,
    /// Accented Latin, so a European library is searchable as it is spelled.
    Accents,
}

impl Page {
    pub fn label(self) -> &'static str {
        match self {
            Self::Letters => "abc",
            Self::Capitals => "ABC",
            Self::Symbols => "123",
            Self::Accents => "áöü",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Letters => Self::Capitals,
            Self::Capitals => Self::Symbols,
            Self::Symbols => Self::Accents,
            Self::Accents => Self::Letters,
        }
    }
}

/// A key on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Space,
    Backspace,
    Clear,
    /// Switch to the next page of characters.
    Shift,
    /// Finish editing.
    Done,
}

impl Key {
    /// What to draw on the key.
    pub fn label(self) -> String {
        match self {
            Self::Char(c) => c.to_string(),
            Self::Space => "space".to_owned(),
            Self::Backspace => "\u{232b}".to_owned(),
            Self::Clear => "clear".to_owned(),
            Self::Shift => "next".to_owned(),
            Self::Done => "done".to_owned(),
        }
    }

    /// Whether this key is wider than one cell, for layout.
    pub fn wide(self) -> bool {
        !matches!(self, Self::Char(_))
    }
}

/// Columns in the character grid. Ten keeps rows short enough to cross quickly with a d-pad.
pub const COLUMNS: usize = 10;

const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";
const CAPITALS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
/// Every printable ASCII symbol, not a chosen few.
///
/// The subset that turns up in song titles was enough while this only searched. It is not
/// enough to type a password with, and a keyboard that is missing one character is a keyboard
/// that locks somebody out of their account with no way to tell why.
const SYMBOLS: &str = r##"0123456789.,;:'"!?-_+=*/\&%$#@^~|<>()[]{}`"##;
const ACCENTS: &str = "áàâäãåæçéèêëíìîïñóòôöõøßúùûüý";

/// State of a text field being edited with the on-screen keyboard.
#[derive(Debug, Clone, Default)]
pub struct Keyboard {
    text: String,
    page: Page,
    /// Position in the flattened key list.
    cursor: usize,
    /// Cap on how much text is accepted, so a stuck key cannot grow it without bound.
    limit: usize,
}

/// Long enough for any real search or player name; short enough to bound the field.
const DEFAULT_LIMIT: usize = 64;

impl Keyboard {
    pub fn new() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            ..Default::default()
        }
    }

    /// Start editing an existing string.
    pub fn with_text(text: impl Into<String>) -> Self {
        let mut keyboard = Self::new();
        keyboard.text = text.into();
        keyboard
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit.max(1);
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Put the cursor on a specific key, for the pointer.
    pub fn set_cursor(&mut self, index: usize) {
        self.cursor = index.min(self.keys().len().saturating_sub(1));
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.truncate();
    }

    /// Every key on the current page, in reading order.
    ///
    /// The trailing controls are on every page: a player who switched to symbols to type an
    /// apostrophe should not have to switch back to press backspace.
    pub fn keys(&self) -> Vec<Key> {
        let characters = match self.page {
            Page::Letters => LETTERS,
            Page::Capitals => CAPITALS,
            Page::Symbols => SYMBOLS,
            Page::Accents => ACCENTS,
        };
        let mut keys: Vec<Key> = characters.chars().map(Key::Char).collect();
        keys.extend([
            Key::Shift,
            Key::Space,
            Key::Backspace,
            Key::Clear,
            Key::Done,
        ]);
        keys
    }

    /// Row and column of the key at `index`, for drawing.
    pub fn position(index: usize) -> (usize, usize) {
        (index / COLUMNS, index % COLUMNS)
    }

    /// Rows the current page occupies.
    pub fn rows(&self) -> usize {
        self.keys().len().div_ceil(COLUMNS)
    }

    /// The key under the cursor.
    pub fn selected(&self) -> Key {
        let keys = self.keys();
        keys[self.cursor.min(keys.len() - 1)]
    }

    /// Move the cursor. Horizontal movement wraps within the row; vertical wraps within the
    /// column, and lands on the last key of a short final row rather than nowhere.
    pub fn navigate(&mut self, dx: isize, dy: isize) {
        let keys = self.keys();
        let total = keys.len();
        if total == 0 {
            return;
        }
        self.cursor = self.cursor.min(total - 1);
        let rows = total.div_ceil(COLUMNS) as isize;
        let (row, column) = Self::position(self.cursor);
        let (mut row, mut column) = (row as isize, column as isize);

        if dx != 0 {
            let width = Self::row_width(total, row as usize) as isize;
            column = (column + dx).rem_euclid(width.max(1));
        }
        if dy != 0 {
            row = (row + dy).rem_euclid(rows.max(1));
            // The last row is usually short. Sliding down a right-hand column should reach
            // its last key, not fall off the grid.
            let width = Self::row_width(total, row as usize) as isize;
            column = column.min(width.max(1) - 1);
        }
        self.cursor = (row as usize * COLUMNS + column as usize).min(total - 1);
    }

    fn row_width(total: usize, row: usize) -> usize {
        let start = row * COLUMNS;
        total.saturating_sub(start).min(COLUMNS)
    }

    /// Press the selected key. Returns `true` when editing is finished.
    pub fn press(&mut self) -> bool {
        self.apply(self.selected())
    }

    /// Press a specific key, wherever the cursor is. Used by the physical keyboard, which does
    /// not move the on-screen cursor.
    pub fn apply(&mut self, key: Key) -> bool {
        match key {
            Key::Char(c) => self.push(c),
            Key::Space => self.push(' '),
            Key::Backspace => {
                self.text.pop();
            }
            Key::Clear => self.text.clear(),
            Key::Shift => {
                self.page = self.page.next();
                self.cursor = self.cursor.min(self.keys().len() - 1);
            }
            Key::Done => return true,
        }
        false
    }

    /// Type a character directly, from a physical keyboard.
    pub fn push(&mut self, c: char) {
        // Control characters would be invisible in the field and meaningless in a search.
        if c.is_control() {
            return;
        }
        if self.text.chars().count() < self.limit {
            self.text.push(c);
        }
    }

    /// Delete the last character.
    pub fn backspace(&mut self) {
        self.text.pop();
    }

    fn truncate(&mut self) {
        if self.text.chars().count() > self.limit {
            self.text = self.text.chars().take(self.limit).collect();
        }
    }
}
