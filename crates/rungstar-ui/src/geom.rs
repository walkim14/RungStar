//! Design-unit geometry, and the projection onto a real window.
//!
//! UltraStar Deluxe lays every screen out in absolute 800x600 coordinates and then stretches
//! the result, so a 1280x800 Steam Deck and a 21:9 monitor both get a picture built for a CRT.
//! Every player count has its own hand-placed copy of the same layout for the same reason.
//!
//! Here the design space is **1000 units tall and as wide as the window's aspect ratio makes
//! it**. Vertical sizes are therefore stable everywhere — a lyric line is 64 units tall on
//! every display — and a wider screen simply has more room beside the content rather than a
//! stretched copy of it. Layout is composition of rectangles, so a six-player screen is the
//! same code as a one-player screen with a different split.

/// Design units from top to bottom, on every display.
pub const DESIGN_HEIGHT: f32 = 1000.0;

/// A point in design units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in design units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Where inside a box something sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Horizontal fraction, `0.0` left to `1.0` right.
    pub fn fx(self) -> f32 {
        match self {
            Self::TopLeft | Self::Left | Self::BottomLeft => 0.0,
            Self::Top | Self::Center | Self::Bottom => 0.5,
            Self::TopRight | Self::Right | Self::BottomRight => 1.0,
        }
    }

    /// Vertical fraction, `0.0` top to `1.0` bottom.
    pub fn fy(self) -> f32 {
        match self {
            Self::TopLeft | Self::Top | Self::TopRight => 0.0,
            Self::Left | Self::Center | Self::Right => 0.5,
            Self::BottomLeft | Self::Bottom | Self::BottomRight => 1.0,
        }
    }
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// A rect from its edges rather than its size.
    pub fn from_edges(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self::new(left, top, right - left, bottom - top)
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Shrink by the same amount on every side. A negative amount grows it.
    pub fn inset(&self, by: f32) -> Self {
        self.inset_xy(by, by)
    }

    /// Shrink horizontally and vertically by different amounts.
    pub fn inset_xy(&self, x: f32, y: f32) -> Self {
        Self::new(
            self.x + x,
            self.y + y,
            (self.w - x * 2.0).max(0.0),
            (self.h - y * 2.0).max(0.0),
        )
    }

    /// Shrink each side independently.
    pub fn inset_each(&self, left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self::from_edges(
            self.x + left,
            self.y + top,
            (self.right() - right).max(self.x + left),
            (self.bottom() - bottom).max(self.y + top),
        )
    }

    /// Move by an offset.
    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// A `w` x `h` box placed at `anchor` inside this one, with a margin from the anchored
    /// edges. Centre anchors ignore the corresponding margin, which is what you want: a
    /// centred title should not drift when the margin changes.
    pub fn anchored(&self, anchor: Anchor, w: f32, h: f32, margin: f32) -> Self {
        let inner = self.inset(margin);
        let x = inner.x + (inner.w - w) * anchor.fx();
        let y = inner.y + (inner.h - h) * anchor.fy();
        Self::new(x, y, w, h)
    }

    /// The largest rect of the given aspect ratio that fits, centred. Used for cover art and
    /// video, where stretching is worse than letterboxing.
    pub fn fit_aspect(&self, aspect: f32) -> Self {
        if aspect <= 0.0 || self.w <= 0.0 || self.h <= 0.0 {
            return *self;
        }
        let (w, h) = if self.w / self.h > aspect {
            (self.h * aspect, self.h)
        } else {
            (self.w, self.w / aspect)
        };
        self.anchored(Anchor::Center, w, h, 0.0)
    }

    /// The smallest rect of the given aspect ratio that covers this one, centred. Used for
    /// backgrounds, where a border is worse than losing an edge.
    pub fn cover_aspect(&self, aspect: f32) -> Self {
        if aspect <= 0.0 || self.w <= 0.0 || self.h <= 0.0 {
            return *self;
        }
        let (w, h) = if self.w / self.h > aspect {
            (self.w, self.w / aspect)
        } else {
            (self.h * aspect, self.h)
        };
        self.anchored(Anchor::Center, w, h, 0.0)
    }

    /// Split into `n` columns with `gap` between them.
    pub fn columns(&self, n: usize, gap: f32) -> Vec<Rect> {
        self.strips(n, gap, true)
    }

    /// Split into `n` rows with `gap` between them.
    pub fn rows(&self, n: usize, gap: f32) -> Vec<Rect> {
        self.strips(n, gap, false)
    }

    fn strips(&self, n: usize, gap: f32, horizontal: bool) -> Vec<Rect> {
        if n == 0 {
            return Vec::new();
        }
        let total = if horizontal { self.w } else { self.h };
        let each = ((total - gap * (n - 1) as f32) / n as f32).max(0.0);
        (0..n)
            .map(|i| {
                let step = (each + gap) * i as f32;
                if horizontal {
                    Self::new(self.x + step, self.y, each, self.h)
                } else {
                    Self::new(self.x, self.y + step, self.w, each)
                }
            })
            .collect()
    }

    /// Cut `amount` off the top, returning `(cut, remainder)`.
    pub fn cut_top(&self, amount: f32) -> (Rect, Rect) {
        let a = amount.clamp(0.0, self.h);
        (
            Self::new(self.x, self.y, self.w, a),
            Self::new(self.x, self.y + a, self.w, self.h - a),
        )
    }

    /// Cut `amount` off the bottom, returning `(cut, remainder)`.
    pub fn cut_bottom(&self, amount: f32) -> (Rect, Rect) {
        let a = amount.clamp(0.0, self.h);
        (
            Self::new(self.x, self.bottom() - a, self.w, a),
            Self::new(self.x, self.y, self.w, self.h - a),
        )
    }

    /// Cut `amount` off the left, returning `(cut, remainder)`.
    pub fn cut_left(&self, amount: f32) -> (Rect, Rect) {
        let a = amount.clamp(0.0, self.w);
        (
            Self::new(self.x, self.y, a, self.h),
            Self::new(self.x + a, self.y, self.w - a, self.h),
        )
    }

    /// Cut `amount` off the right, returning `(cut, remainder)`.
    pub fn cut_right(&self, amount: f32) -> (Rect, Rect) {
        let a = amount.clamp(0.0, self.w);
        (
            Self::new(self.right() - a, self.y, a, self.h),
            Self::new(self.x, self.y, self.w - a, self.h),
        )
    }

    /// Scale about the centre. `1.0` is unchanged.
    pub fn scaled(&self, factor: f32) -> Self {
        let c = self.center();
        let (w, h) = (self.w * factor, self.h * factor);
        Self::new(c.x - w / 2.0, c.y - h / 2.0, w, h)
    }

    /// The overlapping part of two rects, or `None` when they do not touch.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let r = Self::from_edges(
            self.x.max(other.x),
            self.y.max(other.y),
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        );
        (r.w > 0.0 && r.h > 0.0).then_some(r)
    }
}

/// Maps design units onto a physical window.
///
/// Held by the renderer, not by screens: a screen that knew the pixel size could be written to
/// depend on it, and then it would only be correct on one display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    scale: f32,
    width: f32,
    height: f32,
}

impl Projection {
    /// For a window of the given pixel size.
    pub fn new(pixel_width: u32, pixel_height: u32) -> Self {
        let height = pixel_height.max(1) as f32;
        let width = pixel_width.max(1) as f32;
        Self {
            scale: height / DESIGN_HEIGHT,
            width,
            height,
        }
    }

    /// The whole screen in design units. Always `DESIGN_HEIGHT` tall; wider on a wider display.
    pub fn screen(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width / self.scale, DESIGN_HEIGHT)
    }

    /// Design units per pixel — the size of the thinnest line worth drawing.
    pub fn unit(&self) -> f32 {
        1.0 / self.scale
    }

    /// Design units to pixels.
    pub fn px(&self, units: f32) -> f32 {
        units * self.scale
    }

    pub fn point(&self, p: Point) -> Point {
        Point::new(p.x * self.scale, p.y * self.scale)
    }

    pub fn rect(&self, r: Rect) -> Rect {
        Rect::new(
            r.x * self.scale,
            r.y * self.scale,
            r.w * self.scale,
            r.h * self.scale,
        )
    }

    /// Pixels back to design units, for mouse and touch.
    pub fn unproject(&self, pixel_x: f32, pixel_y: f32) -> Point {
        Point::new(pixel_x / self.scale, pixel_y / self.scale)
    }

    /// A content column of at most `max_width` design units, centred.
    ///
    /// This is what stops an ultrawide display from spreading a menu across a metre of glass:
    /// the content stays a readable width and the extra space becomes margin.
    pub fn content(&self, max_width: f32) -> Rect {
        let screen = self.screen();
        if screen.w <= max_width {
            screen
        } else {
            screen.anchored(Anchor::Center, max_width, screen.h, 0.0)
        }
    }
}
