//! Theme, layout, widgets and screen state — with no graphics API anywhere in it.
//!
//! Everything here is pure: screens turn state into a [`draw::DrawList`] of rectangles and
//! strings in design units, and a backend turns that into pixels. That boundary is what lets
//! the whole user interface be tested without a window, and what lets the renderer be
//! replaced — the SDL backend and the wgpu one that follows consume the same list.

pub mod browse;
pub mod calibratescreen;
pub mod chime;
pub mod color;
pub mod draw;
pub mod editorscreen;
pub mod geom;
pub mod glyphs;
pub mod keyboard;
pub mod menu;
pub mod menus;
pub mod micscreen;
pub mod options;
pub mod partyscreen;
pub mod playerscreen;
pub mod screen;
pub mod settings;
pub mod singscreen;
pub mod songselect;
pub mod statsscreen;
pub mod theme;
pub mod usdbscreen;

pub use browse::{Browser, Layout, Placement};
pub use calibratescreen::CalibrateScreen;
pub use chime::Chime;
pub use color::Color;
pub use draw::{Align, Command, DrawList, Font, ImageId, TextStyle, VAlign};
pub use geom::{Anchor, Point, Projection, Rect, DESIGN_HEIGHT};
pub use keyboard::Keyboard;
pub use menu::{Cursor, Repeat};
pub use menus::{MainMenu, OptionsOutcome, OptionsScreen};
pub use micscreen::{MicOutcome, MicScreen};
pub use playerscreen::{PlayerOutcome, PlayerScreen};
pub use screen::{Route, Transition, Widgets};
pub use settings::{Choice, Settings};
pub use singscreen::{Note, NoteKind, PauseChoice, SingScreen, Singer, Sung, Syllable};
pub use songselect::SongSelect;
pub use statsscreen::StatsScreen;
pub use theme::{Style, Theme};
