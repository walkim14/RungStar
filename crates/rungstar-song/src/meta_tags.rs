//! The mini-format hidden inside the `#VIDEO:` header.
//!
//! USDB overloads `#VIDEO` with a comma-separated `key=value` list describing where to fetch
//! each resource and how to process it, for example:
//!
//! ```text
//! #VIDEO:a=dQw4w9WgXcQ,co=abc123,co-crop=10-20-300-300,bg=xyz789,preview=32.5
//! ```
//!
//! A `#VIDEO` value containing no `=` at all is an ordinary video filename, not meta tags.
//! Only the comma needs escaping, as `%2C`, because it is the field separator.

use std::fmt;

use crate::error::Warnings;

/// Percent-escape the one character that would otherwise split a field.
pub fn encode_value(value: &str) -> String {
    value.replace(',', "%2C")
}

/// Reverse of [`encode_value`].
pub fn decode_value(value: &str) -> String {
    value.replace("%2C", ",")
}

/// Which image a tag applies to. Doubles as the tag key prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePrefix {
    Cover,
    Background,
}

impl ImagePrefix {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "co",
            Self::Background => "bg",
        }
    }
}

/// A crop rectangle.
///
/// Stored as edges but written as `left-upper-width-height`, which is how the tag is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub left: i32,
    pub upper: i32,
    pub right: i32,
    pub lower: i32,
}

impl Crop {
    pub fn parse(value: &str) -> Option<Self> {
        let parts: Vec<i32> = value
            .split('-')
            .map(|p| p.parse().ok())
            .collect::<Option<_>>()?;
        let [left, upper, width, height] = parts.as_slice() else {
            return None;
        };
        Some(Self {
            left: *left,
            upper: *upper,
            right: left + width,
            lower: upper + height,
        })
    }

    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.lower - self.upper
    }
}

impl fmt::Display for Crop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{}-{}-{}",
            self.left,
            self.upper,
            self.width(),
            self.height()
        )
    }
}

/// A target size. A single number means a square.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resize {
    pub width: i32,
    pub height: i32,
}

impl Resize {
    pub fn parse(value: &str) -> Option<Self> {
        if let Some((w, h)) = value.split_once('-') {
            Some(Self {
                width: w.parse().ok()?,
                height: h.parse().ok()?,
            })
        } else {
            let n = value.parse().ok()?;
            Some(Self {
                width: n,
                height: n,
            })
        }
    }
}

impl fmt::Display for Resize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.width == self.height {
            write!(f, "{}", self.width)
        } else {
            write!(f, "{}-{}", self.width, self.height)
        }
    }
}

/// Contrast adjustment: automatic, or an explicit enhancement factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Contrast {
    Auto,
    Factor(f64),
}

impl fmt::Display for Contrast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Factor(v) => f.write_str(&format_float(*v)),
        }
    }
}

/// Where an image comes from and what to do to it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImageMetaTags {
    pub source: String,
    pub rotate: Option<f64>,
    pub crop: Option<Crop>,
    pub resize: Option<Resize>,
    pub contrast: Option<Contrast>,
}

impl ImageMetaTags {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            ..Default::default()
        }
    }

    /// Resolve the source to a URL.
    ///
    /// A bare token with no scheme and no slash is a fanart.tv id — the convention exists
    /// because ids are far shorter than URLs and `#VIDEO` is a single line.
    pub fn source_url(&self) -> String {
        if self.source.contains("://") {
            // fanart.tv moved its assets to a different host; old tags still name the old one.
            return self.source.replace("images.fanart.tv", "assets.fanart.tv");
        }
        if self.source.contains('/') {
            return format!("https://{}", self.source);
        }
        format!("https://assets.fanart.tv/fanart/{}", self.source)
    }

    /// Whether any processing was requested.
    pub fn needs_processing(&self) -> bool {
        self.rotate.is_some_and(|r| r != 0.0)
            || self.crop.is_some()
            || self.resize.is_some()
            || self.contrast.is_some()
    }

    fn write_tags(&self, prefix: ImagePrefix, out: &mut Vec<String>) {
        let p = prefix.as_str();
        out.push(format!("{p}={}", encode_value(&self.source)));
        if let Some(rotate) = self.rotate {
            out.push(format!("{p}-rotate={}", format_float(rotate)));
        }
        // Cover writes crop before resize, background after. The asymmetry is inherited from
        // the reference implementation and kept so files round-trip byte for byte.
        if prefix == ImagePrefix::Cover {
            if let Some(crop) = self.crop {
                out.push(format!("{p}-crop={crop}"));
            }
        }
        if let Some(resize) = self.resize {
            out.push(format!("{p}-resize={resize}"));
        }
        if prefix == ImagePrefix::Background {
            if let Some(crop) = self.crop {
                out.push(format!("{p}-crop={crop}"));
            }
        }
        if let Some(contrast) = self.contrast {
            out.push(format!("{p}-contrast={contrast}"));
        }
    }
}

/// Start and end of the medley section, in beats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MedleyTag {
    pub start: i32,
    pub end: i32,
}

impl MedleyTag {
    pub fn parse(value: &str) -> Option<Self> {
        let (start, end) = value.split_once('-')?;
        Some(Self {
            start: start.parse().ok()?,
            end: end.parse().ok()?,
        })
    }
}

impl fmt::Display for MedleyTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

/// Everything the `#VIDEO` header can carry beyond a plain filename.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetaTags {
    pub audio: Option<String>,
    pub video: Option<String>,
    pub cover: Option<ImageMetaTags>,
    pub background: Option<ImageMetaTags>,
    pub player1: Option<String>,
    pub player2: Option<String>,
    /// Preview start, in seconds.
    pub preview: Option<f64>,
    pub medley: Option<MedleyTag>,
    pub tags: Option<String>,
}

impl MetaTags {
    /// Parse a `#VIDEO` value.
    ///
    /// Returns an empty set when the value has no `=` at all, which means it is an ordinary
    /// video filename rather than a tag list.
    pub fn parse(video_tag: &str, warnings: &mut Warnings) -> Self {
        let mut tags = Self::default();
        if !video_tag.contains('=') {
            return tags;
        }
        for pair in video_tag.split(',') {
            let trimmed = pair.trim_start();
            let Some((key, value)) = trimmed.split_once('=') else {
                warnings.warn(format!("missing key or value for meta tag: '{pair}'"));
                continue;
            };
            tags.set(&key.to_lowercase(), &decode_value(value), warnings);
        }
        tags
    }

    fn set(&mut self, key: &str, value: &str, warnings: &mut Warnings) {
        match key {
            "v" => self.video = Some(strip_url_params(value)),
            "a" => self.audio = Some(strip_url_params(value)),
            // Recognised so they do not warn, but no processing is implemented for them.
            "v-trim" | "v-crop" => {}
            "co" => self.cover = Some(ImageMetaTags::new(value)),
            "bg" => self.background = Some(ImageMetaTags::new(value)),
            // Modifiers only apply if the image they belong to was declared first.
            "co-rotate" => set_rotate(self.cover.as_mut(), value, warnings),
            "co-crop" => set_crop(self.cover.as_mut(), value, warnings),
            "co-resize" => set_resize(self.cover.as_mut(), value, warnings),
            "co-contrast" => {
                if let Some(cover) = self.cover.as_mut() {
                    cover.contrast = parse_contrast(value, warnings);
                }
            }
            "bg-crop" => set_crop(self.background.as_mut(), value, warnings),
            "bg-resize" => set_resize(self.background.as_mut(), value, warnings),
            "p1" => self.player1 = Some(value.to_owned()),
            "p2" => self.player2 = Some(value.to_owned()),
            "preview" => self.preview = parse_float(value, warnings),
            "medley" => {
                self.medley = MedleyTag::parse(value);
                if self.medley.is_none() {
                    warnings.warn(format!("invalid value for medley meta tag: '{value}'"));
                }
            }
            "tags" => self.tags = Some(value.to_owned()),
            _ => warnings.warn(format!("unknown key for meta tag: '{key}={value}'")),
        }
    }

    /// Whether the song explicitly asks for audio without video.
    pub fn is_audio_only(&self) -> bool {
        self.audio.is_some() && self.video.is_none()
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl fmt::Display for MetaTags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if let Some(audio) = &self.audio {
            parts.push(format!("a={}", encode_value(audio)));
        }
        if let Some(video) = &self.video {
            parts.push(format!("v={}", encode_value(video)));
        }
        if let Some(cover) = &self.cover {
            cover.write_tags(ImagePrefix::Cover, &mut parts);
        }
        if let Some(background) = &self.background {
            background.write_tags(ImagePrefix::Background, &mut parts);
        }
        if let Some(p1) = &self.player1 {
            parts.push(format!("p1={}", encode_value(p1)));
        }
        if let Some(p2) = &self.player2 {
            parts.push(format!("p2={}", encode_value(p2)));
        }
        if let Some(preview) = self.preview {
            parts.push(format!("preview={}", format_float(preview)));
        }
        if let Some(medley) = self.medley {
            parts.push(format!("medley={medley}"));
        }
        if let Some(tags) = &self.tags {
            parts.push(format!("tags={}", encode_value(tags)));
        }
        f.write_str(&parts.join(","))
    }
}

fn set_rotate(image: Option<&mut ImageMetaTags>, value: &str, warnings: &mut Warnings) {
    if let Some(image) = image {
        image.rotate = parse_float(value, warnings);
    }
}

fn set_crop(image: Option<&mut ImageMetaTags>, value: &str, warnings: &mut Warnings) {
    if let Some(image) = image {
        image.crop = Crop::parse(value);
        if image.crop.is_none() {
            warnings.warn(format!("invalid value for crop meta tag: '{value}'"));
        }
    }
}

fn set_resize(image: Option<&mut ImageMetaTags>, value: &str, warnings: &mut Warnings) {
    if let Some(image) = image {
        image.resize = Resize::parse(value);
        if image.resize.is_none() {
            warnings.warn(format!("invalid value for resize meta tag: '{value}'"));
        }
    }
}

fn parse_float(value: &str, warnings: &mut Warnings) -> Option<f64> {
    match value.parse::<f64>() {
        Ok(v) if v.is_finite() => Some(v),
        _ => {
            warnings.warn(format!("invalid number for meta tag: '{value}'"));
            None
        }
    }
}

fn parse_contrast(value: &str, warnings: &mut Warnings) -> Option<Contrast> {
    if value == "auto" {
        return Some(Contrast::Auto);
    }
    match value.parse::<f64>() {
        Ok(v) if v.is_finite() => Some(Contrast::Factor(v)),
        _ => {
            warnings.warn(format!("invalid value for contrast meta tag: '{value}'"));
            None
        }
    }
}

/// Drop everything from the first `&` onward.
///
/// Tag values are often pasted straight from a browser and carry tracking parameters that
/// would otherwise be treated as part of the id.
fn strip_url_params(url: &str) -> String {
    url.split_once('&')
        .map_or_else(|| url.to_owned(), |(base, _)| base.to_owned())
}

/// Format a float the way the reference tooling does: integral values keep a `.0`.
fn format_float(v: f64) -> String {
    format!("{v:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> (MetaTags, Warnings) {
        let mut w = Warnings::new();
        let tags = MetaTags::parse(s, &mut w);
        (tags, w)
    }

    #[test]
    fn plain_filename_is_not_meta_tags() {
        let (tags, w) = parse("some video.mp4");
        assert!(tags.is_empty());
        assert!(w.is_empty());
    }

    #[test]
    fn round_trips_a_full_tag_set() {
        let input = "a=audioid,v=videoid,co=coverid,co-rotate=1.5,co-crop=10-20-300-300,\
co-resize=1000,co-contrast=auto,bg=bgid,bg-resize=1920-1080,bg-crop=0-0-100-50,\
p1=Alice,p2=Bob,preview=32.5,medley=120-480,tags=rock";
        let (tags, w) = parse(input);
        assert!(w.is_empty(), "unexpected warnings: {:?}", w.as_slice());
        assert_eq!(tags.to_string(), input);
    }

    #[test]
    fn commas_in_values_are_escaped() {
        let (tags, _) = parse("tags=rock%2Cpop");
        assert_eq!(tags.tags.as_deref(), Some("rock,pop"));
        assert_eq!(tags.to_string(), "tags=rock%2Cpop");
    }

    #[test]
    fn bare_source_resolves_to_fanart() {
        assert_eq!(
            ImageMetaTags::new("abc123").source_url(),
            "https://assets.fanart.tv/fanart/abc123"
        );
        assert_eq!(
            ImageMetaTags::new("example.com/a.jpg").source_url(),
            "https://example.com/a.jpg"
        );
        assert_eq!(
            ImageMetaTags::new("https://images.fanart.tv/fanart/x.jpg").source_url(),
            "https://assets.fanart.tv/fanart/x.jpg"
        );
    }

    #[test]
    fn url_parameters_are_stripped() {
        let (tags, _) = parse("v=abc123&list=PL123");
        assert_eq!(tags.video.as_deref(), Some("abc123"));
    }

    #[test]
    fn crop_stores_edges_but_writes_extent() {
        let crop = Crop::parse("10-20-300-400").unwrap();
        assert_eq!(
            (crop.left, crop.upper, crop.right, crop.lower),
            (10, 20, 310, 420)
        );
        assert_eq!(crop.to_string(), "10-20-300-400");
    }

    #[test]
    fn modifiers_without_an_image_are_ignored() {
        let (tags, _) = parse("co-crop=1-2-3-4");
        assert!(tags.cover.is_none());
    }

    #[test]
    fn unknown_keys_warn_but_do_not_abort() {
        let (tags, w) = parse("a=x,bogus=y,v=z");
        assert_eq!(tags.audio.as_deref(), Some("x"));
        assert_eq!(tags.video.as_deref(), Some("z"));
        assert_eq!(w.len(), 1);
    }
}
