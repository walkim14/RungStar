//! The theme format, and resolving one into the concrete style a screen draws with.
//!
//! UltraStar Deluxe themes are `.ini` files of absolute pixel coordinates: a theme decides
//! *where every element goes*, so changing a layout means editing every theme, and a theme
//! written for one screen size is wrong on another. That is why USDX ships a handful of themes
//! that all look the same.
//!
//! Here a theme decides only how things **look** — colours, fonts, corner radius, spacing
//! scale. Layout belongs to the screen and is computed from the window ([`crate::geom`]). A
//! theme is therefore about forty lines of TOML, cannot be broken by a resolution, and cannot
//! break a screen that is added after it was written.
//!
//! Two axes vary independently: a **skin** (dark, light, high contrast) sets the surfaces, and
//! an **accent** recolours the highlight. Every derived colour — hover, pressed, disabled,
//! text-on-accent — is computed from those, so a new accent is one line and cannot produce an
//! unreadable combination.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::color::Color;

/// A parsed theme file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub meta: ThemeMeta,
    /// Skins by name. Must contain at least [`ThemeMeta::default_skin`].
    pub skins: BTreeMap<String, Skin>,
    /// Selectable accent colours by name.
    #[serde(default)]
    pub accents: BTreeMap<String, Color>,
    #[serde(default)]
    pub fonts: Fonts,
    #[serde(default)]
    pub metrics: Metrics,
    #[serde(default)]
    pub players: Vec<Color>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeMeta {
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default = "default_skin_name")]
    pub default_skin: String,
    #[serde(default)]
    pub default_accent: String,
}

fn default_skin_name() -> String {
    "dark".to_owned()
}

/// The surfaces and text colours of one skin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skin {
    /// Behind everything, when no song background is showing.
    pub background: Color,
    /// Panels, list rows, cards.
    pub surface: Color,
    /// Primary text on `surface`.
    pub text: Color,
    /// Secondary text: captions, hints, inactive items.
    pub muted: Color,
    /// The accent used when the player has not chosen one.
    pub accent: Color,
    /// Scrims over artwork so text stays readable on any cover.
    #[serde(default = "default_scrim")]
    pub scrim: Color,
    #[serde(default = "default_danger")]
    pub danger: Color,
    #[serde(default = "default_success")]
    pub success: Color,
    #[serde(default = "default_warning")]
    pub warning: Color,
}

fn default_scrim() -> Color {
    Color::rgba(0, 0, 0, 170)
}
fn default_danger() -> Color {
    Color::rgb(0xe5, 0x48, 0x48)
}
fn default_success() -> Color {
    Color::rgb(0x3d, 0xd6, 0x8c)
}
fn default_warning() -> Color {
    Color::rgb(0xf5, 0xb3, 0x4a)
}

/// Font files, relative to the theme directory. Empty means "use the built-in".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Fonts {
    #[serde(default)]
    pub regular: String,
    #[serde(default)]
    pub bold: String,
    /// Used for lyrics, which want a wider, more legible face than the UI.
    #[serde(default)]
    pub lyrics: String,
}

/// Numbers a theme may tune without touching a layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Corner radius of panels and buttons, in design units.
    pub radius: f32,
    /// Base gap between elements. Layouts express spacing as multiples of this.
    pub spacing: f32,
    /// Body text size in design units.
    pub text_size: f32,
    /// Multiplier applied to every text size. Accessibility, and Deck-at-arms-length.
    pub text_scale: f32,
    /// Thickness of focus rings and outlines.
    pub outline: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            radius: 10.0,
            spacing: 16.0,
            text_size: 30.0,
            text_scale: 1.0,
            outline: 3.0,
        }
    }
}

/// Everything a screen needs to draw, with no lookups and no `Option`s left.
///
/// Produced by [`Theme::resolve`]. Screens take a `&Style` and never see the theme file, so a
/// missing key or an unknown accent is handled once, here, rather than at every use site.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub background: Color,
    pub surface: Color,
    /// One step up from `surface`: the row under the cursor, a raised card.
    pub surface_raised: Color,
    /// One step down: a well, a text field, a track behind a slider.
    pub surface_sunken: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    /// Text and icons drawn *on* the accent. Chosen for contrast, never assumed white.
    pub on_accent: Color,
    /// The accent, dimmed, for the ring around a focused-but-not-active control.
    pub accent_soft: Color,
    pub scrim: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
    /// Per-player highlight colours, always at least six long.
    pub players: Vec<Color>,
    pub metrics: Metrics,
    /// `true` when the skin is light, so callers can pick shadow-vs-glow.
    pub light: bool,
}

impl Style {
    /// The colour for player `index`, wrapping if a mode somehow exceeds the list.
    pub fn player(&self, index: usize) -> Color {
        self.players[index % self.players.len()]
    }

    /// Body text size with the theme's scale applied.
    pub fn text_size(&self) -> f32 {
        self.metrics.text_size * self.metrics.text_scale
    }

    /// A text size relative to body, e.g. `1.6` for a heading.
    pub fn scaled_text(&self, factor: f32) -> f32 {
        self.text_size() * factor
    }

    /// Spacing in multiples of the base gap.
    pub fn gap(&self, multiple: f32) -> f32 {
        self.metrics.spacing * multiple
    }
}

/// Why a theme could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("reading theme `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing theme `{path}`: {source}")]
    Parse {
        path: String,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("theme `{0}` defines no skins")]
    NoSkins(String),
    #[error("theme `{theme}` names `{skin}` as its default skin but does not define it")]
    MissingDefaultSkin { theme: String, skin: String },
}

/// The player colours used when a theme does not supply its own.
///
/// The first four match UltraStar Deluxe's so a returning player's colour does not move; five
/// and six are new, because USDX's own source calls its 8- and 12-player paths untested.
const FALLBACK_PLAYERS: [Color; 6] = [
    Color::rgb(0x41, 0x9d, 0xff),
    Color::rgb(0xff, 0x4d, 0x4d),
    Color::rgb(0x4d, 0xd6, 0x6b),
    Color::rgb(0xff, 0xd2, 0x3f),
    Color::rgb(0xc2, 0x6b, 0xff),
    Color::rgb(0x3f, 0xe0, 0xd8),
];

impl Theme {
    /// Read a theme from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ThemeError> {
        let path = path.as_ref();
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ThemeError::Io {
            path: shown.clone(),
            source,
        })?;
        Self::parse(&text).map_err(|source| ThemeError::Parse {
            path: shown,
            source: Box::new(source),
        })
    }

    /// Read a theme from TOML text.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// The theme shipped with the game, available with no files on disk at all.
    ///
    /// A game that cannot start because a theme file is missing is a game that cannot start
    /// after a bad update, so the default is compiled in and the files on disk are optional.
    pub fn builtin() -> Self {
        Self::parse(include_str!("../themes/rung.toml"))
            .expect("the built-in theme is parsed by a test")
    }

    /// Check the invariants `resolve` relies on.
    pub fn validate(&self) -> Result<(), ThemeError> {
        if self.skins.is_empty() {
            return Err(ThemeError::NoSkins(self.meta.name.clone()));
        }
        if !self.skins.contains_key(&self.meta.default_skin) {
            return Err(ThemeError::MissingDefaultSkin {
                theme: self.meta.name.clone(),
                skin: self.meta.default_skin.clone(),
            });
        }
        Ok(())
    }

    /// The names of the skins, in a stable order for a settings screen.
    pub fn skin_names(&self) -> Vec<&str> {
        self.skins.keys().map(String::as_str).collect()
    }

    /// The names of the accents, in a stable order for a settings screen.
    pub fn accent_names(&self) -> Vec<&str> {
        self.accents.keys().map(String::as_str).collect()
    }

    /// Flatten into the style screens draw with.
    ///
    /// An unknown skin or accent falls back rather than failing: the names come from a config
    /// file the player can edit and from themes that get replaced, and refusing to draw
    /// anything because a colour was renamed is the wrong trade.
    pub fn resolve(&self, skin_name: &str, accent_name: &str) -> Style {
        let skin = self
            .skins
            .get(skin_name)
            .or_else(|| self.skins.get(&self.meta.default_skin))
            .or_else(|| self.skins.values().next())
            .cloned()
            .unwrap_or_else(Skin::fallback_dark);

        let accent = self
            .accents
            .get(accent_name)
            .copied()
            .or_else(|| self.accents.get(&self.meta.default_accent).copied())
            .unwrap_or(skin.accent);

        let light = skin.background.luminance() > 0.5;
        // Raised and sunken are derived rather than authored, because asking every theme to
        // get them right by hand is how themes end up with an invisible list cursor.
        //
        // The direction is not simply "up on light, down on dark": a surface at either
        // extreme has no room to step that way. A light skin's surface is usually pure white,
        // and lightening white returns white — so both steps are taken downwards instead, and
        // `raised` stays the brighter of the two so the pair never swap roles.
        const NO_ROOM: f32 = 0.06;
        let luminance = skin.surface.luminance();
        let (raised, sunken) = if luminance > 1.0 - NO_ROOM {
            (skin.surface.darken(0.05), skin.surface.darken(0.13))
        } else if luminance < NO_ROOM {
            (skin.surface.lighten(0.13), skin.surface.lighten(0.05))
        } else if light {
            (skin.surface.lighten(0.5), skin.surface.darken(0.07))
        } else {
            (skin.surface.lighten(0.10), skin.surface.darken(0.45))
        };

        let players = if self.players.len() >= 6 {
            self.players.clone()
        } else {
            let mut p = self.players.clone();
            p.extend(FALLBACK_PLAYERS.iter().skip(p.len()).copied());
            p
        };

        Style {
            background: skin.background,
            surface: skin.surface,
            surface_raised: raised,
            surface_sunken: sunken,
            text: skin.text,
            muted: skin.muted,
            accent,
            on_accent: accent.contrasting(),
            accent_soft: accent.alpha(0.45),
            scrim: skin.scrim,
            danger: skin.danger,
            success: skin.success,
            warning: skin.warning,
            players,
            metrics: self.metrics.clone(),
            light,
        }
    }

    /// Resolve with the theme's own defaults.
    pub fn resolve_default(&self) -> Style {
        self.resolve(&self.meta.default_skin, &self.meta.default_accent)
    }
}

impl Skin {
    fn fallback_dark() -> Self {
        Self {
            background: Color::rgb(0x0d, 0x0d, 0x14),
            surface: Color::rgb(0x1a, 0x1a, 0x26),
            text: Color::rgb(0xf2, 0xf2, 0xf7),
            muted: Color::rgb(0x9a, 0x9a, 0xb0),
            accent: Color::rgb(0xff, 0x4d, 0x8d),
            scrim: default_scrim(),
            danger: default_danger(),
            success: default_success(),
            warning: default_warning(),
        }
    }
}
