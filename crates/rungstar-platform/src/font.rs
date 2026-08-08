//! Real text: font loading, glyph rasterisation and a packed atlas.
//!
//! The diagnostics tool draws with SDL's built-in debug font, which is a fixed 8x8 bitmap —
//! fine for a readout of numbers, useless for song titles at three sizes in thirty languages.
//! UltraStar Deluxe solves the same problem with hand-rolled FreeType atlases and a chain of
//! twenty-five configurable fallback fonts.
//!
//! Glyphs are rasterised once per (face, pixel size) and packed into a growing texture, then
//! drawn with colour modulation — so a string costs one texture and N quads however many
//! colours the screen uses it in, and a size that is already on screen costs nothing to draw
//! again. Sizes are quantised to whole pixels because a cache keyed on a float never hits.

use std::collections::HashMap;

use rungstar_ui::draw::Font;

/// A loaded face plus the metrics needed to lay a line out.
pub struct Face {
    font: fontdue::Font,
}

/// Why text could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("reading font `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{path}` is not a font this build understands: {reason}")]
    Parse { path: String, reason: String },
    #[error("no usable font was found; looked at: {0}")]
    NoneFound(String),
}

impl Face {
    pub fn from_bytes(bytes: &[u8], label: &str) -> Result<Self, FontError> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(
            |reason| FontError::Parse {
                path: label.to_owned(),
                reason: reason.to_owned(),
            },
        )?;
        Ok(Self { font })
    }

    pub fn load(path: &std::path::Path) -> Result<Self, FontError> {
        let bytes = std::fs::read(path).map_err(|source| FontError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_bytes(&bytes, &path.display().to_string())
    }

    /// Advance width of a string at a pixel size, before any glyph is drawn.
    ///
    /// Used to align and to decide whether a title needs eliding, so it has to agree exactly
    /// with what [`Atlas`] will lay out — both walk the same metrics.
    pub fn width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }
}

/// A face shipped beside the executable, which is what a packaged build uses.
///
/// Looked at before anything on the system. Borrowing Segoe UI works on a developer's Windows
/// machine and produces a different-looking game on every other one; a Flatpak has no system
/// fonts to borrow at all beyond what the runtime happens to carry.
///
/// `assets/fonts/` is empty in the repository on purpose — a font binary is a megabyte of
/// something nobody reviews in a diff. Packaging drops one in; see `packaging/README.md`.
fn bundled(name: &str) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(beside) = exe.parent() {
            paths.push(beside.join("assets").join("fonts").join(name));
            // One level up as well, for `target/release/rungstar` run from the source tree.
            if let Some(up) = beside.parent() {
                paths.push(up.join("assets").join("fonts").join(name));
            }
        }
    }
    paths.push(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts")
            .join(name),
    );
    paths
}

/// Where the platform looks for a font when the theme does not name one that exists.
///
/// A bundled face first, then whatever the system has. The fallback stays because a build run
/// from the source tree has no bundled font and should still start.
fn system_font_candidates() -> Vec<std::path::PathBuf> {
    let mut paths: Vec<std::path::PathBuf> = bundled("RungStar-Regular.ttf");
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
        for name in ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "verdana.ttf"] {
            paths.push(std::path::Path::new(&root).join("Fonts").join(name));
        }
    } else {
        for path in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/run/host/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ] {
            paths.push(std::path::PathBuf::from(path));
        }
    }
    paths
}

/// Bold variants of the same faces, so headings are not faked by drawing twice.
fn system_bold_candidates() -> Vec<std::path::PathBuf> {
    let mut paths: Vec<std::path::PathBuf> = bundled("RungStar-Bold.ttf");
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
        for name in [
            "segoeuib.ttf",
            "arialbd.ttf",
            "tahomabd.ttf",
            "verdanab.ttf",
        ] {
            paths.push(std::path::Path::new(&root).join("Fonts").join(name));
        }
    } else {
        for path in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/run/host/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        ] {
            paths.push(std::path::PathBuf::from(path));
        }
    }
    paths
}

/// The three roles a theme names, resolved to loaded faces.
pub struct FontSet {
    regular: Face,
    bold: Face,
    lyrics: Face,
}

impl FontSet {
    /// Load the faces a theme asks for, falling back to a system font for any it does not
    /// supply or that cannot be read.
    ///
    /// A theme naming a font that is not there must not stop the game — it should look
    /// slightly different, not fail to start.
    pub fn load(
        regular: Option<&std::path::Path>,
        bold: Option<&std::path::Path>,
        lyrics: Option<&std::path::Path>,
    ) -> Result<Self, FontError> {
        let regular_face = Self::load_or_system(regular, &system_font_candidates())?;
        // Bold falls back to the regular face rather than to a system bold that may not match
        // it: one family drawn at one weight reads better than two families.
        let bold_face = Self::load_or_system(bold, &system_bold_candidates())
            .or_else(|_| Self::load_or_system(bold, &system_font_candidates()))?;
        let lyrics_face = match lyrics.filter(|p| p.exists()) {
            Some(path) => Face::load(path)?,
            None => Self::load_or_system(None, &system_bold_candidates())
                .or_else(|_| Self::load_or_system(None, &system_font_candidates()))?,
        };
        Ok(Self {
            regular: regular_face,
            bold: bold_face,
            lyrics: lyrics_face,
        })
    }

    fn load_or_system(
        preferred: Option<&std::path::Path>,
        candidates: &[std::path::PathBuf],
    ) -> Result<Face, FontError> {
        if let Some(path) = preferred.filter(|p| p.exists()) {
            return Face::load(path);
        }
        for candidate in candidates {
            if candidate.exists() {
                if let Ok(face) = Face::load(candidate) {
                    return Ok(face);
                }
            }
        }
        Err(FontError::NoneFound(
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ))
    }

    pub fn face(&self, font: Font) -> &Face {
        match font {
            Font::Regular => &self.regular,
            Font::Bold => &self.bold,
            Font::Lyrics => &self.lyrics,
        }
    }

    /// Width of a string in pixels.
    pub fn width(&self, font: Font, text: &str, size: f32) -> f32 {
        self.face(font).width(text, size)
    }
}

/// One rasterised glyph and where it sits in the atlas.
#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    /// Position and size within the atlas texture, in pixels.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Offset from the pen position to the top-left of the bitmap.
    pub left: f32,
    pub top: f32,
    pub advance: f32,
}

/// The atlas texture is square and grows by doubling. Starting small keeps the first frame
/// cheap; a menu at one size rarely needs more than this.
const INITIAL_SIDE: u32 = 512;

/// Beyond this a further doubling is more likely to be a runaway than a real need, and the
/// atlas resets instead — a rebuilt cache costs a frame, an out-of-memory costs the session.
const MAX_SIDE: u32 = 4096;

/// A growing shelf-packed atlas of glyphs for one face at one pixel size.
///
/// Shelf packing rather than anything cleverer because glyphs at one size are all roughly one
/// height, which is the case shelf packing is optimal for.
pub struct Atlas {
    /// RGBA pixels, kept on the CPU so the texture can be rebuilt after a device loss and so
    /// growing does not need a GPU read-back.
    pixels: Vec<u8>,
    side: u32,
    /// Left edge of the next glyph on the current shelf.
    pen_x: u32,
    /// Top edge of the current shelf.
    shelf_y: u32,
    /// Height of the current shelf.
    shelf_h: u32,
    glyphs: HashMap<char, Glyph>,
    /// Set when a glyph was added, so the caller knows to re-upload.
    dirty: bool,
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

impl Atlas {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; (INITIAL_SIDE * INITIAL_SIDE * 4) as usize],
            side: INITIAL_SIDE,
            pen_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            glyphs: HashMap::new(),
            dirty: true,
        }
    }

    pub fn side(&self) -> u32 {
        self.side
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Look a glyph up, rasterising it if this is the first time it has been seen.
    pub fn glyph(&mut self, face: &Face, size: f32, c: char) -> Glyph {
        if let Some(glyph) = self.glyphs.get(&c) {
            return *glyph;
        }
        let (metrics, bitmap) = face.font.rasterize(c, size);
        let (w, h) = (metrics.width as u32, metrics.height as u32);

        // A space has metrics but no pixels. It still needs an entry, or every space would be
        // rasterised again on every frame.
        let placed = if w == 0 || h == 0 {
            (0, 0)
        } else {
            self.reserve(w, h)
        };

        // One pixel of padding stops a neighbouring glyph bleeding in under filtering.
        let glyph = Glyph {
            x: placed.0,
            y: placed.1,
            w,
            h,
            left: metrics.xmin as f32,
            top: -(metrics.height as f32 + metrics.ymin as f32),
            advance: metrics.advance_width,
        };

        if w > 0 && h > 0 {
            self.blit(&bitmap, glyph);
        }
        self.glyphs.insert(c, glyph);
        self.dirty = true;
        glyph
    }

    /// Find room for a `w` x `h` glyph, growing or resetting the atlas if there is none.
    fn reserve(&mut self, w: u32, h: u32) -> (u32, u32) {
        const PAD: u32 = 1;
        loop {
            if self.pen_x + w + PAD <= self.side {
                if self.shelf_y + h + PAD <= self.side {
                    let position = (self.pen_x, self.shelf_y);
                    self.pen_x += w + PAD;
                    self.shelf_h = self.shelf_h.max(h);
                    return position;
                }
            } else if self.shelf_y + self.shelf_h + h + PAD * 2 <= self.side {
                // Start a new shelf.
                self.shelf_y += self.shelf_h + PAD;
                self.shelf_h = 0;
                self.pen_x = 0;
                continue;
            }
            // Out of room on this side length.
            if self.side < MAX_SIDE {
                self.grow();
            } else {
                // A cache this large is either a script with an enormous character set or a
                // runaway. Rebuilding costs a frame; refusing to draw costs the session.
                self.reset();
            }
        }
    }

    fn grow(&mut self) {
        let new_side = self.side * 2;
        let mut pixels = vec![0u8; (new_side * new_side * 4) as usize];
        // Copy row by row: the old rows are shorter than the new ones.
        for row in 0..self.side {
            let from = (row * self.side * 4) as usize;
            let to = (row * new_side * 4) as usize;
            let width = (self.side * 4) as usize;
            pixels[to..to + width].copy_from_slice(&self.pixels[from..from + width]);
        }
        self.pixels = pixels;
        self.side = new_side;
        self.dirty = true;
    }

    fn reset(&mut self) {
        self.pixels.fill(0);
        self.pen_x = 0;
        self.shelf_y = 0;
        self.shelf_h = 0;
        self.glyphs.clear();
        self.dirty = true;
    }

    /// Copy a rasterised coverage bitmap in as white pixels with the coverage as alpha, so one
    /// atlas serves every colour the screen draws in.
    fn blit(&mut self, bitmap: &[u8], glyph: Glyph) {
        for row in 0..glyph.h {
            for column in 0..glyph.w {
                let coverage = bitmap[(row * glyph.w + column) as usize];
                let offset = (((glyph.y + row) * self.side + glyph.x + column) * 4) as usize;
                self.pixels[offset] = 255;
                self.pixels[offset + 1] = 255;
                self.pixels[offset + 2] = 255;
                self.pixels[offset + 3] = coverage;
            }
        }
    }
}

/// Atlases keyed by face and whole-pixel size.
///
/// Quantising the size is what makes the cache work at all: design units scale by the window
/// height, so an un-quantised key would miss on almost every frame and rasterise the alphabet
/// again each time.
#[derive(Default)]
pub struct AtlasCache {
    atlases: HashMap<(Font, u32), Atlas>,
}

impl AtlasCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Round a size to the key the cache uses. Public so callers measure at the same size
    /// they draw at.
    pub fn quantise(size: f32) -> u32 {
        size.round().clamp(6.0, 400.0) as u32
    }

    pub fn atlas(&mut self, font: Font, size: u32) -> &mut Atlas {
        self.atlases.entry((font, size)).or_default()
    }

    pub fn len(&self) -> usize {
        self.atlases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.atlases.is_empty()
    }

    /// Drop every atlas. Called when the theme's fonts change, since the glyphs cached are
    /// from the old face.
    pub fn clear(&mut self) {
        self.atlases.clear();
    }
}
