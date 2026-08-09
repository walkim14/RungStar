//! Turning a [`DrawList`] into SDL draw calls.
//!
//! This is the only file that knows both what the game wants drawn and how this particular
//! backend draws it. Screens produce design-unit commands and never see a canvas, so the wgpu
//! renderer that follows is a sibling of this file rather than a rewrite of every screen.
//!
//! SDL's 2D renderer is used rather than raw GPU work because a karaoke interface is textured
//! quads and text, which is exactly what it is good at, and it already works on both targets.

use std::collections::HashMap;

use sdl3::pixels::{Color as SdlColor, PixelFormat};
use sdl3::rect::Rect as IRect;
use sdl3::render::{BlendMode, Canvas, FRect, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};

use rungstar_ui::color::Color;
use rungstar_ui::draw::{Align, Command, DrawList, Font, ImageId, Overflow, TextStyle, VAlign};
use rungstar_ui::geom::{Projection, Rect};

use crate::font::{AtlasCache, FontSet};

/// Why the renderer could not start or draw.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("fonts: {0}")]
    Font(#[from] crate::font::FontError),
    #[error("sdl: {0}")]
    Sdl(String),
}

impl From<sdl3::Error> for RenderError {
    fn from(error: sdl3::Error) -> Self {
        Self::Sdl(error.to_string())
    }
}

/// What an image slot holds.
struct Image {
    texture: Texture,
    width: u32,
    height: u32,
}

/// Draws display lists onto an SDL canvas.
pub struct Renderer {
    canvas: Canvas<Window>,
    creator: TextureCreator<WindowContext>,
    fonts: FontSet,
    atlases: AtlasCache,
    /// One texture per (face, size) atlas, uploaded lazily.
    glyph_textures: HashMap<(Font, u32), Texture>,
    images: HashMap<ImageId, Image>,
    next_image: u32,
    projection: Projection,
}

/// The ellipsis appended when a string is elided. A single character, so eliding never needs
/// three glyphs' worth of room.
const ELLIPSIS: char = '\u{2026}';

/// How far `Overflow::Shrink` is allowed to go before it gives up and elides instead.
const MIN_SHRINK: f32 = 0.6;

impl Renderer {
    pub fn new(canvas: Canvas<Window>, fonts: FontSet) -> Result<Self, RenderError> {
        let creator = canvas.texture_creator();
        let (width, height) = canvas.output_size()?;
        Ok(Self {
            canvas,
            creator,
            fonts,
            atlases: AtlasCache::new(),
            glyph_textures: HashMap::new(),
            images: HashMap::new(),
            next_image: 1,
            projection: Projection::new(width, height),
        })
    }

    pub fn canvas(&mut self) -> &mut Canvas<Window> {
        &mut self.canvas
    }

    /// The current design-space projection. Screens lay out against this.
    pub fn projection(&self) -> Projection {
        self.projection
    }

    /// Re-read the window size. Called on a resize, and cheap enough to call every frame.
    pub fn resize(&mut self) -> Result<(), RenderError> {
        let (width, height) = self.canvas.output_size()?;
        self.projection = Projection::new(width, height);
        Ok(())
    }

    /// What is being drawn with.
    pub fn fonts(&self) -> &FontSet {
        &self.fonts
    }

    /// Swap the fonts, dropping every cached glyph — they were drawn with the old face.
    pub fn set_fonts(&mut self, fonts: FontSet) {
        self.fonts = fonts;
        self.atlases.clear();
        self.glyph_textures.clear();
    }

    /// Upload RGBA pixels as an image the display list can reference.
    pub fn add_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<ImageId, RenderError> {
        let format = PixelFormat::ABGR8888;
        let mut texture = self
            .creator
            .create_texture_streaming(format, width, height)
            .map_err(|e| RenderError::Sdl(e.to_string()))?;
        texture
            .update(None, rgba, (width * 4) as usize)
            .map_err(|e| RenderError::Sdl(e.to_string()))?;
        texture.set_blend_mode(BlendMode::Blend);

        let id = ImageId(self.next_image);
        self.next_image += 1;
        self.images.insert(
            id,
            Image {
                texture,
                width,
                height,
            },
        );
        Ok(id)
    }

    /// Replace an image's pixels in place, keeping its handle.
    ///
    /// A video is a new picture thirty times a second; making a fresh texture for each would
    /// churn GPU memory and leave the display list holding a handle that changes every frame.
    /// The size has to match — a video does not change shape part way through.
    pub fn update_image(
        &mut self,
        id: ImageId,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), RenderError> {
        let Some(image) = self.images.get_mut(&id) else {
            return Ok(());
        };
        if image.width != width || image.height != height {
            return Err(RenderError::Sdl(format!(
                "image {id:?} is {}x{} and cannot take a {width}x{height} frame",
                image.width, image.height
            )));
        }
        image
            .texture
            .update(None, rgba, (width * 4) as usize)
            .map_err(|e| RenderError::Sdl(e.to_string()))
    }

    /// Forget an image. Covers scroll out of the browser and their textures should go with
    /// them, or browsing a large library grows without bound.
    pub fn drop_image(&mut self, id: ImageId) {
        self.images.remove(&id);
    }

    pub fn image_size(&self, id: ImageId) -> Option<(u32, u32)> {
        self.images.get(&id).map(|i| (i.width, i.height))
    }

    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    /// Width of a string in design units, for the few places that need it before drawing.
    pub fn measure(&self, font: Font, text: &str, size_units: f32) -> f32 {
        let pixels = self.projection.px(size_units);
        let quantised = AtlasCache::quantise(pixels) as f32;
        self.fonts.width(font, text, quantised) / self.projection.px(1.0)
    }

    /// Draw a whole frame.
    pub fn render(&mut self, list: &DrawList, clear: Color) -> Result<(), RenderError> {
        self.canvas.set_blend_mode(BlendMode::Blend);
        self.canvas.set_draw_color(sdl(clear));
        self.canvas.clear();

        // Clips nest, and SDL has one. Keeping the stack here means a screen can clip inside
        // a clip without the inner one leaking out when it pops.
        let mut clips: Vec<Rect> = Vec::new();
        for command in list.commands() {
            self.draw(command, &mut clips)?;
        }
        self.canvas.set_clip_rect(None);
        self.canvas.present();
        Ok(())
    }

    fn draw(&mut self, command: &Command, clips: &mut Vec<Rect>) -> Result<(), RenderError> {
        match command {
            Command::Rect {
                rect,
                color,
                radius,
            } => self.fill_rounded(*rect, *color, *radius),
            Command::Outline {
                rect,
                color,
                width,
                radius,
            } => self.stroke_rounded(*rect, *color, *width, *radius),
            Command::Line { a, b, color, width } => {
                // SDL draws hairlines only, so a thick line is a quad. Axis-aligned lines are
                // almost all of them, and this keeps those exact.
                let (a, b) = (self.projection.point(*a), self.projection.point(*b));
                let thickness = self.projection.px(*width).max(1.0);
                self.canvas.set_draw_color(sdl(*color));
                let rect = if (a.y - b.y).abs() < (a.x - b.x).abs() {
                    FRect::new(
                        a.x.min(b.x),
                        (a.y + b.y) / 2.0 - thickness / 2.0,
                        (b.x - a.x).abs(),
                        thickness,
                    )
                } else {
                    FRect::new(
                        (a.x + b.x) / 2.0 - thickness / 2.0,
                        a.y.min(b.y),
                        thickness,
                        (b.y - a.y).abs(),
                    )
                };
                self.canvas.fill_rect(rect)?;
                Ok(())
            }
            Command::Image {
                rect,
                image,
                tint,
                source,
                ..
            } => self.draw_image(*rect, *image, *tint, *source),
            Command::Text { rect, text, style } => self.draw_text(*rect, text, style),
            Command::PushClip(rect) => {
                // The effective clip is the intersection with whatever is already in force.
                let effective = clips
                    .last()
                    .and_then(|outer| outer.intersect(rect))
                    .unwrap_or(*rect);
                clips.push(effective);
                self.apply_clip(Some(effective));
                Ok(())
            }
            Command::PopClip => {
                clips.pop();
                self.apply_clip(clips.last().copied());
                Ok(())
            }
        }
    }

    fn apply_clip(&mut self, rect: Option<Rect>) {
        match rect {
            Some(rect) => {
                let r = self.projection.rect(rect);
                self.canvas.set_clip_rect(IRect::new(
                    r.x as i32,
                    r.y as i32,
                    r.w.max(0.0) as u32,
                    r.h.max(0.0) as u32,
                ));
            }
            None => self.canvas.set_clip_rect(None),
        }
    }

    fn fill_rounded(&mut self, rect: Rect, color: Color, radius: f32) -> Result<(), RenderError> {
        let r = self.projection.rect(rect);
        self.canvas.set_draw_color(sdl(color));
        let radius = self.projection.px(radius).min(r.w / 2.0).min(r.h / 2.0);
        if radius < 1.0 {
            self.canvas.fill_rect(FRect::new(r.x, r.y, r.w, r.h))?;
            return Ok(());
        }
        // Rounded corners as a stack of horizontal spans. SDL has no rounded-rect primitive,
        // and spans keep it to one fill_rects call rather than a texture per corner size.
        let mut spans = Vec::with_capacity(radius as usize * 2 + 1);
        spans.push(FRect::new(r.x, r.y + radius, r.w, r.h - radius * 2.0));
        for step in 0..radius.ceil() as i32 {
            let y = step as f32;
            // Horizontal half-width of the corner circle at this row.
            let inset = radius
                - (radius * radius - (radius - y - 0.5).powi(2))
                    .max(0.0)
                    .sqrt();
            let x = r.x + inset;
            let w = r.w - inset * 2.0;
            spans.push(FRect::new(x, r.y + y, w, 1.0));
            spans.push(FRect::new(x, r.y + r.h - y - 1.0, w, 1.0));
        }
        self.canvas.fill_rects(&spans)?;
        Ok(())
    }

    fn stroke_rounded(
        &mut self,
        rect: Rect,
        color: Color,
        width: f32,
        radius: f32,
    ) -> Result<(), RenderError> {
        // An outline is the difference of two filled rounded rects, but drawing it that way
        // needs the background colour. Four bars plus corner arcs is closer than it sounds:
        // at the radii a UI uses, the arcs are a handful of pixels.
        let r = self.projection.rect(rect);
        let t = self.projection.px(width).max(1.0);
        self.canvas.set_draw_color(sdl(color));
        let corner = self.projection.px(radius).min(r.w / 2.0).min(r.h / 2.0);
        let bars = [
            FRect::new(r.x + corner, r.y, r.w - corner * 2.0, t),
            FRect::new(r.x + corner, r.y + r.h - t, r.w - corner * 2.0, t),
            FRect::new(r.x, r.y + corner, t, r.h - corner * 2.0),
            FRect::new(r.x + r.w - t, r.y + corner, t, r.h - corner * 2.0),
        ];
        self.canvas.fill_rects(&bars)?;
        if corner >= 1.0 {
            let mut arcs = Vec::new();
            let steps = (corner * 2.0).ceil() as i32;
            for step in 0..=steps {
                let angle = std::f32::consts::FRAC_PI_2 * step as f32 / steps as f32;
                let (sin, cos) = angle.sin_cos();
                for (cx, cy, sx, sy) in [
                    (r.x + corner, r.y + corner, -1.0, -1.0),
                    (r.x + r.w - corner, r.y + corner, 1.0, -1.0),
                    (r.x + corner, r.y + r.h - corner, -1.0, 1.0),
                    (r.x + r.w - corner, r.y + r.h - corner, 1.0, 1.0),
                ] {
                    let x = cx + sx * cos * (corner - t / 2.0);
                    let y = cy + sy * sin * (corner - t / 2.0);
                    arcs.push(FRect::new(x - t / 2.0, y - t / 2.0, t, t));
                }
            }
            self.canvas.fill_rects(&arcs)?;
        }
        Ok(())
    }

    fn draw_image(
        &mut self,
        rect: Rect,
        id: ImageId,
        tint: Color,
        source: Rect,
    ) -> Result<(), RenderError> {
        let Some(image) = self.images.get_mut(&id) else {
            // A cover that has not finished loading is not an error; the screen draws a
            // placeholder underneath and the picture appears when it arrives.
            return Ok(());
        };
        image.texture.set_color_mod(tint.r, tint.g, tint.b);
        image.texture.set_alpha_mod(tint.a);
        let destination = self.projection.rect(rect);
        let src = FRect::new(
            source.x * image.width as f32,
            source.y * image.height as f32,
            source.w * image.width as f32,
            source.h * image.height as f32,
        );
        self.canvas.copy(
            &image.texture,
            src,
            FRect::new(destination.x, destination.y, destination.w, destination.h),
        )?;
        Ok(())
    }

    fn draw_text(&mut self, rect: Rect, text: &str, style: &TextStyle) -> Result<(), RenderError> {
        if text.is_empty() {
            return Ok(());
        }
        let box_px = self.projection.rect(rect);
        let requested = self.projection.px(style.size);

        // Fit the string to the box, which is where the three overflow policies differ.
        let (size, shown) = self.fit(text, style, requested, box_px.w);
        let quantised = AtlasCache::quantise(size);
        let width = self.fonts.width(style.font, &shown, quantised as f32);

        let x = box_px.x + (box_px.w - width) * align_fraction(style.align);
        // Vertical placement uses the cap height rather than the full line height, so a row
        // of text sits where the eye expects rather than being pushed down by descenders no
        // glyph in it uses.
        let cap = quantised as f32 * 0.72;
        // Descenders reach below the baseline, so bottom alignment has to leave room for them.
        // Putting the baseline on the box edge instead is what makes a label's `g` and `y`
        // land in the caption underneath it.
        let descent = quantised as f32 * 0.21;
        let y = match style.valign {
            VAlign::Top => box_px.y + cap,
            VAlign::Middle => box_px.y + (box_px.h + cap) / 2.0,
            VAlign::Bottom => box_px.y + box_px.h - descent,
        };

        if let Some((outline_color, outline_width)) = style.outline {
            // An outline is the same string drawn around itself. Eight offsets rather than
            // four, because four leaves the diagonals thin over a bright video frame.
            let offset = self.projection.px(outline_width).max(1.0);
            for (dx, dy) in [
                (-1.0, 0.0),
                (1.0, 0.0),
                (0.0, -1.0),
                (0.0, 1.0),
                (-0.7, -0.7),
                (0.7, -0.7),
                (-0.7, 0.7),
                (0.7, 0.7),
            ] {
                self.blit_string(
                    &shown,
                    style.font,
                    quantised,
                    x + dx * offset,
                    y + dy * offset,
                    outline_color,
                )?;
            }
        }
        self.blit_string(&shown, style.font, quantised, x, y, style.color)
    }

    /// Choose the size and the string to draw, honouring the overflow policy.
    fn fit(&self, text: &str, style: &TextStyle, requested: f32, available: f32) -> (f32, String) {
        let width_at = |size: f32, s: &str| {
            self.fonts
                .width(style.font, s, AtlasCache::quantise(size) as f32)
        };
        if width_at(requested, text) <= available {
            return (requested, text.to_owned());
        }
        match style.overflow {
            Overflow::Clip => (requested, text.to_owned()),
            Overflow::Shrink => {
                // Shrink to fit, but only so far: past this the text is smaller than the
                // interface around it and eliding reads better than squinting.
                let floor = requested * MIN_SHRINK;
                let mut size = requested;
                while size > floor && width_at(size, text) > available {
                    size -= 1.0;
                }
                if width_at(size, text) <= available {
                    (size, text.to_owned())
                } else {
                    (size, self.elide(text, style.font, size, available))
                }
            }
            Overflow::Ellipsis => (
                requested,
                self.elide(text, style.font, requested, available),
            ),
            Overflow::EllipsisStart => (
                requested,
                self.elide_start(text, style.font, requested, available),
            ),
        }
    }

    /// Cut a string down until it fits, ending in an ellipsis.
    fn elide(&self, text: &str, font: Font, size: f32, available: f32) -> String {
        let quantised = AtlasCache::quantise(size) as f32;
        let ellipsis_width = self.fonts.width(font, "\u{2026}", quantised);
        let room = available - ellipsis_width;
        if room <= 0.0 {
            return String::new();
        }
        let mut kept = String::new();
        let mut used = 0.0;
        // Character by character rather than by byte, so this cannot split a multi-byte
        // character — which for a library full of accents is not a rare case.
        for c in text.chars() {
            let advance = self.fonts.width(font, &c.to_string(), quantised);
            if used + advance > room {
                break;
            }
            used += advance;
            kept.push(c);
        }
        kept.push(ELLIPSIS);
        kept
    }

    /// Cut a string down from the front until it fits, starting with an ellipsis.
    ///
    /// The mirror of [`Self::elide`], for strings whose tail is the informative part. Every
    /// song folder on a machine begins the same way, so eliding the end of one shows that
    /// same prefix on every row and says nothing about which folder it actually is.
    fn elide_start(&self, text: &str, font: Font, size: f32, available: f32) -> String {
        let quantised = AtlasCache::quantise(size) as f32;
        let ellipsis_width = self.fonts.width(font, "\u{2026}", quantised);
        let room = available - ellipsis_width;
        if room <= 0.0 {
            return String::new();
        }
        let mut kept: Vec<char> = Vec::new();
        let mut used = 0.0;
        for c in text.chars().rev() {
            let advance = self.fonts.width(font, &c.to_string(), quantised);
            if used + advance > room {
                break;
            }
            used += advance;
            kept.push(c);
        }
        let mut out = String::from(ELLIPSIS);
        out.extend(kept.into_iter().rev());
        out
    }

    /// Draw a string at a pixel baseline, filling the atlas as it goes.
    fn blit_string(
        &mut self,
        text: &str,
        font: Font,
        size: u32,
        x: f32,
        y: f32,
        color: Color,
    ) -> Result<(), RenderError> {
        let face = self.fonts.face(font);
        let atlas = self.atlases.atlas(font, size);

        // Collect placements first: rasterising may grow the atlas, and a quad recorded
        // against the old texture would sample the wrong pixels.
        let mut quads = Vec::with_capacity(text.len());
        let mut pen = x;
        for c in text.chars() {
            let glyph = atlas.glyph(face, size as f32, c);
            if glyph.w > 0 && glyph.h > 0 {
                quads.push((
                    FRect::new(
                        glyph.x as f32,
                        glyph.y as f32,
                        glyph.w as f32,
                        glyph.h as f32,
                    ),
                    FRect::new(
                        pen + glyph.left,
                        y + glyph.top,
                        glyph.w as f32,
                        glyph.h as f32,
                    ),
                ));
            }
            pen += glyph.advance;
        }

        let dirty = atlas.take_dirty();
        let side = atlas.side();
        if dirty {
            let pixels = atlas.pixels().to_vec();
            self.upload_atlas(font, size, side, &pixels)?;
        }
        let Some(texture) = self.glyph_textures.get_mut(&(font, size)) else {
            return Ok(());
        };
        texture.set_color_mod(color.r, color.g, color.b);
        texture.set_alpha_mod(color.a);
        for (src, dst) in quads {
            self.canvas.copy(texture, src, dst)?;
        }
        Ok(())
    }

    fn upload_atlas(
        &mut self,
        font: Font,
        size: u32,
        side: u32,
        pixels: &[u8],
    ) -> Result<(), RenderError> {
        let needs_new = match self.glyph_textures.get(&(font, size)) {
            Some(texture) => {
                let query = texture.query();
                query.width != side || query.height != side
            }
            None => true,
        };
        if needs_new {
            let format = PixelFormat::ABGR8888;
            let mut texture = self
                .creator
                .create_texture_streaming(format, side, side)
                .map_err(|e| RenderError::Sdl(e.to_string()))?;
            texture.set_blend_mode(BlendMode::Blend);
            self.glyph_textures.insert((font, size), texture);
        }
        let texture = self
            .glyph_textures
            .get_mut(&(font, size))
            .expect("just inserted");
        texture
            .update(None, pixels, (side * 4) as usize)
            .map_err(|e| RenderError::Sdl(e.to_string()))?;
        Ok(())
    }
}

/// Where along the box the string starts, as a fraction of the spare width.
fn align_fraction(align: Align) -> f32 {
    match align {
        Align::Start => 0.0,
        Align::Center => 0.5,
        Align::End => 1.0,
    }
}

fn sdl(color: Color) -> SdlColor {
    SdlColor::RGBA(color.r, color.g, color.b, color.a)
}
