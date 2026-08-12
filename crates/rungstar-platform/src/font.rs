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
///
/// Carries a **fallback chain**. No single face worth shipping covers everything a real song
/// library contains: measured over 8,134 songs, 99.94% of the text is ASCII, but the remainder
/// includes 28,000 accented letters, 200 Cyrillic characters, a few hundred CJK brackets and a
/// handful of Hangul — and a face chosen for how it looks will always be missing some of it.
///
/// Without a chain the choice is between a face with character and a face with coverage, and
/// the failure mode of choosing wrong is silent: an empty box where a letter should be. That
/// already happened once, with the star glyphs in the USDB ratings.
pub struct Face {
    font: fontdue::Font,
    /// Tried in order for any character `font` cannot draw.
    fallbacks: Vec<fontdue::Font>,
    /// Where this face came from, for `--check` to report.
    ///
    /// Which face is in use is otherwise invisible until somebody looks at the screen, and
    /// the whole failure mode being guarded against here is a machine quietly borrowing a
    /// different one.
    source: String,
    /// The names of the faces behind it, in order.
    behind: Vec<String>,
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
        Ok(Self {
            font,
            fallbacks: Vec::new(),
            source: short_name(label),
            behind: Vec::new(),
        })
    }

    /// What this face is, as a name worth printing.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The faces tried behind it, nearest first.
    pub fn behind(&self) -> &[String] {
        &self.behind
    }

    /// Add a face to try for characters this one does not have.
    pub fn with_fallback(mut self, other: Face) -> Self {
        self.fallbacks.push(other.font);
        self.fallbacks.extend(other.fallbacks);
        self.behind.push(other.source);
        self.behind.extend(other.behind);
        self
    }

    /// The face that can actually draw this character.
    ///
    /// The main face when it has the glyph, otherwise the first fallback that does. When
    /// nothing has it the main face is returned anyway, so the result is its notdef box —
    /// which is at least a consistent box rather than a missing advance.
    pub(crate) fn for_char(&self, c: char) -> &fontdue::Font {
        if self.font.lookup_glyph_index(c) != 0 {
            return &self.font;
        }
        self.fallbacks
            .iter()
            .find(|face| face.lookup_glyph_index(c) != 0)
            .unwrap_or(&self.font)
    }

    /// Whether anything in the chain can draw this character.
    pub fn has(&self, c: char) -> bool {
        self.font.lookup_glyph_index(c) != 0
            || self
                .fallbacks
                .iter()
                .any(|face| face.lookup_glyph_index(c) != 0)
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
            .map(|c| self.for_char(c).metrics(c, size).advance_width)
            .sum()
    }
}

/// A face shipped beside the executable, which is what a packaged build uses.
///
/// Looked at before anything on the system. Borrowing Segoe UI works on a developer's Windows
/// machine and produces a different-looking game on every other one; a Flatpak has no system
/// fonts to borrow at all beyond what the runtime happens to carry.
///
/// The faces live in `assets/fonts/` and are committed, not dropped in at packaging time. A
/// font binary is a megabyte nobody reviews in a diff, which was the argument for keeping it
/// out — but the cost of keeping it out was that the game looked different on every machine and
/// nobody saw the shipped one until release. `crates/rungstar-platform/tests/fonts.rs` is what
/// makes them reviewable instead: it asserts what they can actually draw.
fn bundled(name: &str) -> Vec<std::path::PathBuf> {
    crate::assets::asset_paths("fonts", name)
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

/// Characters a chain has to be able to draw, one per script a real library turns out to use.
///
/// Measured over 8,134 songs: 99.94% of the text is ASCII, and the remainder is 160,908 curly
/// quotes, 27,868 accented letters, 202 Cyrillic characters, a few hundred CJK brackets and a
/// handful of Hangul. One representative of each is enough — a face with `д` has the alphabet.
///
/// This is also what stops the chain being built out of everything on the machine. Adding a
/// face costs parsing a megabyte, and until this existed the game parsed ten of them three
/// times over at startup, for coverage it mostly already had.
const PROBE: &[char] = &[
    '\u{2019}', // ’ — the single most common non-ASCII character in a song library
    '\u{00F1}', // ñ
    '\u{0153}', // œ
    '\u{20AC}', // €
    '\u{0434}', // д — Cyrillic
    '\u{03B1}', // α — Greek
    '\u{300C}', // 「 — CJK punctuation, in Japanese song titles
    '\u{C228}', // 숨 — Hangul
    '\u{2605}', // ★ — the star that drew as an empty box on the USDB screen
    '\u{266A}', // ♪
];

/// Put coverage faces behind a chosen one, and stop as soon as there is nothing left to cover.
///
/// Greedy rather than exhaustive: a candidate joins the chain only if it draws something the
/// chain cannot already draw, and the search stops once every [`PROBE`] character is covered.
/// A face that adds nothing is not free — it is a megabyte parsed and held — and the first
/// version of this added every readable font on the machine, including the chosen face a
/// second time.
fn with_coverage(face: Face) -> Face {
    let mut chained = face;
    let mut wanted: Vec<char> = PROBE.iter().copied().filter(|c| !chained.has(*c)).collect();
    for candidate in coverage_candidates() {
        if wanted.is_empty() {
            break;
        }
        if !candidate.exists() {
            continue;
        }
        let Ok(other) = Face::load(&candidate) else {
            continue;
        };
        if !wanted.iter().any(|c| other.has(*c)) {
            continue;
        }
        wanted.retain(|c| !other.has(*c));
        chained = chained.with_fallback(other);
    }
    chained
}

/// Faces worth having behind the chosen one, widest coverage first.
fn coverage_candidates() -> Vec<std::path::PathBuf> {
    let mut paths = bundled("RungStar-Fallback.ttf");
    // The system's own faces after that. On Windows Segoe UI covers Cyrillic and Greek, and
    // the CJK face covers the brackets a handful of Japanese songs use in their titles.
    paths.extend(system_font_candidates());
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
        for name in ["msgothic.ttc", "malgun.ttf", "seguisym.ttf"] {
            paths.push(std::path::Path::new(&root).join("Fonts").join(name));
        }
    } else {
        for path in [
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
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
        // Lyrics are the one thing read from across a room, so they get their own heavier
        // face rather than the heading weight.
        let lyrics_face = Self::load_or_system(lyrics, &bundled("RungStar-Lyrics.ttf"))
            .or_else(|_| Self::load_or_system(None, &system_bold_candidates()))
            .or_else(|_| Self::load_or_system(None, &system_font_candidates()))?;
        // Every face gets the same chain behind it, so a Cyrillic title is drawn the same way
        // wherever it appears and a face is chosen for how it looks rather than for what it
        // happens to contain.
        Ok(Self {
            regular: with_coverage(regular_face),
            bold: with_coverage(bold_face),
            lyrics: with_coverage(lyrics_face),
        })
    }

    fn load_or_system(
        preferred: Option<&std::path::Path>,
        candidates: &[std::path::PathBuf],
    ) -> Result<Face, FontError> {
        if let Some(path) = preferred {
            match Face::load(path) {
                Ok(face) => return Ok(face),
                Err(error) => {
                    tracing::warn!(
                        "could not load theme font {}: {error}; using a fallback",
                        path.display()
                    );
                }
            }
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

    /// What is actually being drawn with, for `--check`.
    ///
    /// `Poppins-Regular.ttf + FiraSans-Regular.ttf` on a good day, `segoeui.ttf` on a machine
    /// where the bundled faces did not make it into the build — which is precisely the
    /// difference nobody notices until a release is in front of somebody.
    pub fn describe(&self) -> String {
        let one = |face: &Face| {
            let mut name = face.source().to_owned();
            if !face.behind().is_empty() {
                name.push_str(" + ");
                name.push_str(&face.behind().join(" + "));
            }
            name
        };
        let regular = one(&self.regular);
        let lyrics = one(&self.lyrics);
        if regular == lyrics {
            regular
        } else {
            format!("{regular}, lyrics {lyrics}")
        }
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
        // Through the chain, so a character the chosen face lacks is drawn by one that has
        // it rather than coming out as an empty box.
        let (metrics, bitmap) = face.for_char(c).rasterize(c, size);
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

/// A path reduced to something worth printing on one line.
fn short_name(label: &str) -> String {
    std::path::Path::new(label)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| label.to_owned())
}
