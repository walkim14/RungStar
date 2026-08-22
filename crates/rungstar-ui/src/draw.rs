//! The display list screens produce, and backends consume.
//!
//! A screen never calls a graphics API. It appends to a [`DrawList`] in design units, and
//! something else turns that into SDL calls or wgpu draws. Two things fall out of that: the
//! entire user interface can be exercised in a unit test with no window — a test asserts the
//! *commands*, which is a far stronger check than a screenshot — and the renderer can be
//! replaced without touching a screen. Phase 5 draws through SDL's own renderer; wgpu will
//! consume the same list.
//!
//! Text commands carry a rectangle and an alignment rather than a baseline point, so a screen
//! never has to measure a string. The backend has the font and can align exactly; a layout
//! computed from an estimated width would drift between backends.

use crate::color::Color;
use crate::geom::{Point, Rect};

/// Which face to draw with. The theme maps these to files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Font {
    #[default]
    Regular,
    Bold,
    /// Lyrics want a wider, heavier face than the interface does.
    Lyrics,
}

/// Horizontal placement within the text's rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// How far a capital reaches above the baseline, as a fraction of the text size.
///
/// The vertical metrics live here rather than in the backend because they are part of the
/// drawing contract, not a rendering detail: a screen deciding how tall to make a row has to
/// agree with the thing that draws into it, and when they disagree a caption ends up on top of
/// the label above it. `rungstar-platform` places baselines with these, and
/// [`TextStyle::ink`] is how a test sees the same answer with no window open.
pub const CAP: f32 = 0.72;

/// How far a descender reaches below the baseline, as a fraction of the text size.
pub const DESCENT: f32 = 0.21;

/// Vertical placement within the text's rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// What happens when text is wider than its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    /// Cut at the edge.
    #[default]
    Clip,
    /// Cut and append an ellipsis. Song titles in a list.
    Ellipsis,
    /// Cut from the *front*, keeping the end. For a file path, where every one on the machine
    /// starts the same way and the part worth reading is the last folder.
    EllipsisStart,
    /// Shrink until it fits, down to 60% of the requested size. Used where the whole string
    /// matters more than its size — a player name, a score.
    Shrink,
}

/// How a string should be drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font: Font,
    /// Cap height in design units.
    pub size: f32,
    pub color: Color,
    pub align: Align,
    pub valign: VAlign,
    pub overflow: Overflow,
    /// An outline behind the glyphs, for text over artwork or video.
    pub outline: Option<(Color, f32)>,
}

impl TextStyle {
    /// The band a line of this text actually covers inside `box_rect`.
    ///
    /// Not the box: text is placed by its baseline, and the glyphs above and below it can sit
    /// outside the rectangle they were given. Two rows whose boxes merely touch can therefore
    /// still collide, which is the bug this exists to make visible.
    ///
    /// Only the vertical extent, which is the axis that has to be right — horizontal overflow
    /// is handled by [`Overflow`] and is visible immediately.
    pub fn ink(&self, box_rect: crate::geom::Rect) -> (f32, f32) {
        let cap = self.size * CAP;
        let descent = self.size * DESCENT;
        let baseline = match self.valign {
            VAlign::Top => box_rect.y + cap,
            VAlign::Middle => box_rect.y + (box_rect.h + cap) / 2.0,
            VAlign::Bottom => box_rect.y + box_rect.h - descent,
        };
        (baseline - cap, baseline + descent)
    }

    /// The ink height of one line: cap height plus descender, and nothing else.
    ///
    /// What a stacked line needs when it is drawn against the top or the bottom of its own
    /// box, which is how [`crate::theme::Style::stack`] lays them out.
    pub fn line(size: f32) -> f32 {
        size * (CAP + DESCENT)
    }

    /// How tall a box has to be for a line of this text to fit inside it, whatever its
    /// vertical alignment.
    ///
    /// Two descenders' worth rather than one. [`VAlign::Middle`] centres the **cap height**
    /// rather than the whole ink band — deliberately, because centring the band pushes a line
    /// of capitals visibly high — so the descenders hang below the middle of the box and a box
    /// of exactly cap-plus-descent is not enough for them.
    pub fn height(size: f32) -> f32 {
        size * (CAP + 2.0 * DESCENT)
    }

    pub fn new(size: f32, color: Color) -> Self {
        Self {
            font: Font::Regular,
            size,
            color,
            align: Align::Start,
            valign: VAlign::Middle,
            overflow: Overflow::Clip,
            outline: None,
        }
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    pub fn bold(self) -> Self {
        self.font(Font::Bold)
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn valign(mut self, valign: VAlign) -> Self {
        self.valign = valign;
        self
    }

    pub fn centered(self) -> Self {
        self.align(Align::Center)
    }

    pub fn overflow(mut self, overflow: Overflow) -> Self {
        self.overflow = overflow;
        self
    }

    pub fn ellipsis(self) -> Self {
        self.overflow(Overflow::Ellipsis)
    }

    pub fn outlined(mut self, color: Color, width: f32) -> Self {
        self.outline = Some((color, width));
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

/// A texture the backend has loaded. The UI crate never touches the filesystem, so this is
/// just a handle the application hands back after loading a cover or a background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(pub u32);

/// One thing to draw.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Rect {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    Outline {
        rect: Rect,
        color: Color,
        width: f32,
        radius: f32,
    },
    /// A dimensional capsule: shadow, fill, rim, and a restrained top highlight.
    Bubble {
        rect: Rect,
        fill: Color,
        rim: Color,
    },
    /// A shader-generated halo behind an active control or live pitch marker.
    Glow {
        rect: Rect,
        color: Color,
    },
    /// Draw the ambient stage wash here, modulated by the current musical beat.
    StagePulse {
        strength: f32,
    },
    Text {
        rect: Rect,
        text: String,
        style: TextStyle,
    },
    Image {
        rect: Rect,
        image: ImageId,
        /// Multiplied with the texture. `WHITE` draws it unchanged; a faded alpha dims it.
        tint: Color,
        radius: f32,
        /// The part of the source to draw, in `0.0..=1.0`. Used to crop a cover to a square
        /// without reprocessing the file.
        source: Rect,
    },
    Line {
        a: Point,
        b: Point,
        color: Color,
        width: f32,
    },
    /// Restrict drawing to this rectangle until the matching [`Command::PopClip`].
    PushClip(Rect),
    PopClip,
}

/// The whole frame, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    commands: Vec<Command>,
    clip_depth: usize,
    clip_underflowed: bool,
}

/// The full source rectangle — draw the whole image.
pub const WHOLE_IMAGE: Rect = Rect::new(0.0, 0.0, 1.0, 1.0);

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn clear(&mut self) {
        self.commands.clear();
        self.clip_depth = 0;
        self.clip_underflowed = false;
    }

    /// Every clip pushed must be popped, or the backend inherits a clip from the last frame.
    /// Checked in tests rather than trusted.
    pub fn is_balanced(&self) -> bool {
        self.clip_depth == 0 && !self.clip_underflowed
    }

    pub fn push(&mut self, command: Command) -> &mut Self {
        match command {
            Command::PushClip(_) => self.clip_depth += 1,
            Command::PopClip if self.clip_depth > 0 => self.clip_depth -= 1,
            Command::PopClip => self.clip_underflowed = true,
            _ => {}
        }
        self.commands.push(command);
        self
    }

    /// A filled rectangle with square corners.
    pub fn fill(&mut self, rect: Rect, color: Color) -> &mut Self {
        self.push(Command::Rect {
            rect,
            color,
            radius: 0.0,
        })
    }

    /// A filled rectangle with rounded corners.
    pub fn panel(&mut self, rect: Rect, color: Color, radius: f32) -> &mut Self {
        self.push(Command::Rect {
            rect,
            color,
            radius,
        })
    }

    pub fn outline(&mut self, rect: Rect, color: Color, width: f32, radius: f32) -> &mut Self {
        self.push(Command::Outline {
            rect,
            color,
            width,
            radius,
        })
    }

    pub fn bubble(&mut self, rect: Rect, fill: Color, rim: Color) -> &mut Self {
        self.push(Command::Bubble { rect, fill, rim })
    }

    pub fn glow(&mut self, rect: Rect, color: Color) -> &mut Self {
        self.push(Command::Glow { rect, color })
    }

    pub fn stage_pulse(&mut self, strength: f32) -> &mut Self {
        self.push(Command::StagePulse {
            strength: strength.clamp(0.0, 1.0),
        })
    }

    pub fn text(&mut self, rect: Rect, text: impl Into<String>, style: TextStyle) -> &mut Self {
        self.push(Command::Text {
            rect,
            text: text.into(),
            style,
        })
    }

    pub fn image(&mut self, rect: Rect, image: ImageId) -> &mut Self {
        self.push(Command::Image {
            rect,
            image,
            tint: Color::WHITE,
            radius: 0.0,
            source: WHOLE_IMAGE,
        })
    }

    pub fn image_tinted(
        &mut self,
        rect: Rect,
        image: ImageId,
        tint: Color,
        radius: f32,
    ) -> &mut Self {
        self.push(Command::Image {
            rect,
            image,
            tint,
            radius,
            source: WHOLE_IMAGE,
        })
    }

    pub fn line(&mut self, a: Point, b: Point, color: Color, width: f32) -> &mut Self {
        self.push(Command::Line { a, b, color, width })
    }

    /// Draw inside a clip, popping it afterwards whatever the body does.
    pub fn clipped(&mut self, rect: Rect, body: impl FnOnce(&mut Self)) -> &mut Self {
        self.push(Command::PushClip(rect));
        body(self);
        self.push(Command::PopClip)
    }
}

/// Roughly how wide a string will be, in design units.
///
/// The backend measures exactly when it draws; this is for the handful of places that need a
/// size *before* the string exists as pixels — a chip that hugs its label, a column that sizes
/// to its longest entry, and the sing screen laying a line of syllables side by side.
///
/// **An upper bound, not an average.** It was a flat 0.55 em per character, described as a
/// slight over-estimate and measured across seven faces as nothing of the sort: the median
/// advance is 0.70 em and `'W'` is 1.13, so "Wo" was handed 0.73 of the room it needed. Where
/// the number only sizes a box that is harmless, but the sing screen lays syllables edge to
/// edge from it and centres each one in its box — so a syllable wider than its estimate spills
/// half the difference into the syllable on either side, and the words are drawn through each
/// other. Over-estimating leaves a gap; under-estimating is a collision.
pub fn approx_text_width(text: &str, size: f32) -> f32 {
    text.chars().map(|c| advance_of(c) * size).sum()
}

/// The widest this character is across the faces the game is likely to draw with, as a
/// fraction of the em size.
///
/// Measured rather than guessed, over Segoe UI, Arial, Verdana and Tahoma in regular and bold:
/// `'W'` 1.13, `'m'` 1.06, `'w'` 0.98, `'M'` 0.96, nothing else in Latin above 0.85, and the
/// thin letters down at 0.33. Three buckets is enough — the point is a bound, and a table with
/// an entry per glyph would still be wrong for the next font somebody installs.
fn advance_of(c: char) -> f32 {
    match c {
        // The four that broke it. Nothing else in Latin text comes close.
        'W' | 'M' | 'm' | 'w' => 1.15,
        // Punctuation, spaces and the thin letters. Without these the wide bucket would nearly
        // double an ordinary line and shrink it to fit a box it was already inside.
        ' ' | 'i' | 'l' | 'j' | 'I' | 'f' | 't' | 'r' | '!' | '.' | ',' | ';' | ':' | '\''
        | '\u{2019}' | '|' | '(' | ')' | '[' | ']' => 0.55,
        c if c.is_ascii() => 0.90,
        // Anything unmeasured — CJK is a full em by construction, and an accented Latin letter
        // is no wider than the letter under it.
        _ => 1.15,
    }
}
