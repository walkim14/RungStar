//! Colour, and the small amount of arithmetic a theme needs to derive one from another.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Straight (non-premultiplied) 8-bit RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Why a colour string could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ColorError {
    #[error("colour `{0}` must start with `#`")]
    NoHash(String),
    #[error("colour `{0}` must have 3, 4, 6 or 8 hex digits")]
    BadLength(String),
    #[error("colour `{0}` contains a non-hex digit")]
    BadDigit(String),
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    pub fn parse(text: &str) -> Result<Self, ColorError> {
        let body = text
            .strip_prefix('#')
            .ok_or_else(|| ColorError::NoHash(text.to_owned()))?;
        let digit = |c: char| {
            c.to_digit(16)
                .map(|d| d as u8)
                .ok_or_else(|| ColorError::BadDigit(text.to_owned()))
        };
        let short = |c: char| digit(c).map(|d| d * 17);
        let chars: Vec<char> = body.chars().collect();
        match chars.len() {
            3 | 4 => {
                let a = if chars.len() == 4 {
                    short(chars[3])?
                } else {
                    255
                };
                Ok(Self::rgba(
                    short(chars[0])?,
                    short(chars[1])?,
                    short(chars[2])?,
                    a,
                ))
            }
            6 | 8 => {
                let byte = |i: usize| -> Result<u8, ColorError> {
                    Ok(digit(chars[i])? * 16 + digit(chars[i + 1])?)
                };
                let a = if chars.len() == 8 { byte(6)? } else { 255 };
                Ok(Self::rgba(byte(0)?, byte(2)?, byte(4)?, a))
            }
            _ => Err(ColorError::BadLength(text.to_owned())),
        }
    }

    /// The same colour at a different opacity, `0.0..=1.0`.
    pub fn alpha(self, alpha: f32) -> Self {
        Self {
            a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            ..self
        }
    }

    /// Multiply the existing opacity, for fading a whole screen out.
    pub fn fade(self, factor: f32) -> Self {
        Self {
            a: (self.a as f32 * factor.clamp(0.0, 1.0)).round() as u8,
            ..self
        }
    }

    /// Blend towards another colour. `t` of `0.0` is self, `1.0` is `other`.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self {
            r: mix(self.r, other.r),
            g: mix(self.g, other.g),
            b: mix(self.b, other.b),
            a: mix(self.a, other.a),
        }
    }

    /// Towards black, keeping opacity.
    pub fn darken(self, amount: f32) -> Self {
        let a = self.a;
        Self {
            a,
            ..self.lerp(Self::BLACK, amount)
        }
    }

    /// Towards white, keeping opacity.
    pub fn lighten(self, amount: f32) -> Self {
        let a = self.a;
        Self {
            a,
            ..self.lerp(Self::WHITE, amount)
        }
    }

    /// Perceived brightness, `0.0..=1.0`, by the usual luma weights.
    pub fn luminance(self) -> f32 {
        (0.2126 * self.r as f32 + 0.7152 * self.g as f32 + 0.0722 * self.b as f32) / 255.0
    }

    /// Black or white, whichever stays readable on this background.
    ///
    /// Themes let you pick any accent colour, and a caption baked to white vanishes on a
    /// yellow one. Choosing per colour means an accent cannot make its own label unreadable.
    pub fn contrasting(self) -> Self {
        if self.luminance() > 0.55 {
            Self::BLACK
        } else {
            Self::WHITE
        }
    }

    pub fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Linear floats, for a shader.
    pub fn to_linear(self) -> [f32; 4] {
        let c = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        [c(self.r), c(self.g), c(self.b), self.a as f32 / 255.0]
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            write!(
                f,
                "#{:02x}{:02x}{:02x}{:02x}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

impl FromStr for Color {
    type Err = ColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}
