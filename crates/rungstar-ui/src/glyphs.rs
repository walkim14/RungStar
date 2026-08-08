//! What the buttons are called, which is not the same on every controller.
//!
//! The face buttons are the easy part: a Steam Deck uses A, B, X and Y exactly as an Xbox pad
//! does, so a hint saying "A" is right on both. What differs is everything else — the
//! shoulders are LB/RB and LT/RT on Xbox and L1/R1 and L2/R2 on a Deck or a PlayStation pad,
//! and the two menu buttons have no letters at all.
//!
//! A hint that names a button the controller does not have is worse than no hint: it says a
//! button exists and then gives the wrong name for it, which is the same mistake as labelling
//! a keyboard key "X".
//!
//! Detected rather than configured by default. Somebody on a Deck should not have to find a
//! setting to be told the right names, and the setting exists for the case detection cannot
//! cover — a PlayStation pad plugged into a desktop.

use serde::{Deserialize, Serialize};

/// Which naming to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Glyphs {
    /// Work it out from the machine and the pad.
    #[default]
    Automatic,
    /// A, B, X, Y with LB/RB and LT/RT.
    Xbox,
    /// A, B, X, Y with L1/R1 and L2/R2, and the Deck's own menu buttons.
    Deck,
    /// Cross, Circle, Square, Triangle with L1/R1 and L2/R2.
    PlayStation,
}

impl Glyphs {
    pub const ALL: [Glyphs; 4] = [
        Glyphs::Automatic,
        Glyphs::Xbox,
        Glyphs::Deck,
        Glyphs::PlayStation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Xbox => "Xbox",
            Self::Deck => "Steam Deck",
            Self::PlayStation => "PlayStation",
        }
    }

    /// What to use, once the machine has been looked at.
    ///
    /// SteamOS sets `SteamDeck=1` in the environment of anything it launches, and the OS
    /// release file names the distribution — both are cheap and neither needs the pad to be
    /// connected, which matters because the hints are drawn before anybody presses anything.
    pub fn resolve(self) -> Named {
        match self {
            Self::Automatic if on_a_deck() => Named::DECK,
            Self::Automatic | Self::Xbox => Named::XBOX,
            Self::Deck => Named::DECK,
            Self::PlayStation => Named::PLAYSTATION,
        }
    }
}

/// Whether this is a Steam Deck.
pub fn on_a_deck() -> bool {
    if std::env::var("SteamDeck").is_ok_and(|value| value == "1") {
        return true;
    }
    std::fs::read_to_string("/etc/os-release")
        .map(|text| text.to_lowercase().contains("steamos"))
        .unwrap_or(false)
}

/// The names of the buttons, for one kind of controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named {
    pub confirm: &'static str,
    pub back: &'static str,
    pub west: &'static str,
    pub north: &'static str,
    pub left_shoulder: &'static str,
    pub right_shoulder: &'static str,
    pub left_trigger: &'static str,
    pub right_trigger: &'static str,
    pub left_stick: &'static str,
    pub right_stick: &'static str,
    /// The two small ones. Xbox calls them View and Menu; a Deck draws two symbols.
    pub view: &'static str,
    pub menu: &'static str,
}

impl Named {
    pub const XBOX: Named = Named {
        confirm: "A",
        back: "B",
        west: "X",
        north: "Y",
        left_shoulder: "LB",
        right_shoulder: "RB",
        left_trigger: "LT",
        right_trigger: "RT",
        left_stick: "LS",
        right_stick: "RS",
        view: "View",
        menu: "Menu",
    };

    /// A Deck's face buttons are an Xbox pad's; its shoulders are a PlayStation pad's; and its
    /// two menu buttons are printed as symbols with no name at all.
    pub const DECK: Named = Named {
        confirm: "A",
        back: "B",
        west: "X",
        north: "Y",
        left_shoulder: "L1",
        right_shoulder: "R1",
        left_trigger: "L2",
        right_trigger: "R2",
        left_stick: "L3",
        right_stick: "R3",
        view: "\u{29C9}",
        menu: "\u{2630}",
    };

    pub const PLAYSTATION: Named = Named {
        // Named rather than drawn as the shapes: the glyphs are in no font the game can rely
        // on borrowing, and a missing glyph is an empty box where a button name should be.
        confirm: "Cross",
        back: "Circle",
        west: "Square",
        north: "Triangle",
        left_shoulder: "L1",
        right_shoulder: "R1",
        left_trigger: "L2",
        right_trigger: "R2",
        left_stick: "L3",
        right_stick: "R3",
        view: "Create",
        menu: "Options",
    };
}

impl crate::settings::Choice for Glyphs {
    const VALUES: &'static [Glyphs] = &Glyphs::ALL;

    fn label(self) -> &'static str {
        Glyphs::label(self)
    }
}
