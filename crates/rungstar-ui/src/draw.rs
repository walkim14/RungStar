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
    }

    /// Every clip pushed must be popped, or the backend inherits a clip from the last frame.
    /// Checked in tests rather than trusted.
    pub fn is_balanced(&self) -> bool {
        self.clip_depth == 0
    }

    pub fn push(&mut self, command: Command) -> &mut Self {
        match command {
            Command::PushClip(_) => self.clip_depth += 1,
            Command::PopClip => self.clip_depth = self.clip_depth.saturating_sub(1),
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
/// to its longest entry. Deliberately a slight over-estimate, because a box a little too wide
/// looks fine and one a little too narrow clips.
pub fn approx_text_width(text: &str, size: f32) -> f32 {
    // Average advance across Latin text at this size. Wide scripts overshoot, which is the
    // safe direction.
    text.chars().count() as f32 * size * 0.55
}
