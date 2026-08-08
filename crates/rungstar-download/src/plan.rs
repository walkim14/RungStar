//! Working out what to fetch, and in what order, before fetching anything.
//!
//! Separated from the fetching so the decisions are testable: given a song file and what is
//! already on disk, exactly which URLs are wanted, what they will be called, and which of them
//! can be skipped. None of that needs a network, and all of it is where the bugs are.

use std::path::Path;

use rungstar_song::meta_tags::MetaTags;
use rungstar_song::SongTxt;
use rungstar_usdb::{SongDetails, SongId};

use crate::meta::{Kind, SyncMeta};

/// One thing to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: Kind,
    /// Where to get it. A `yt-dlp` step names a page; the rest name a file.
    pub source: Source,
    /// What it will be called in the song folder, without an extension for media whose
    /// container is not known until it has been fetched.
    pub stem: String,
}

/// How a resource is fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A plain HTTP GET.
    Url(String),
    /// A page yt-dlp knows how to extract from. Never reimplemented: YouTube extraction needs
    /// constant updates and a JavaScript runtime for the signature challenges, and the
    /// reference bundles a whole Deno for exactly this.
    Extract { page: String, audio_only: bool },
    /// Text already in hand, from the `gettxt` page.
    Text(String),
}

/// Everything to do for one song.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub usdb_id: SongId,
    pub artist: String,
    pub title: String,
    /// The folder name, already made safe for the file system.
    pub folder: String,
    pub steps: Vec<Step>,
    /// Steps that were skipped because the file on disk is already the right one.
    pub skipped: Vec<Kind>,
}

impl Plan {
    /// Whether anything at all needs fetching.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The steps that have to finish before the song is singable.
    pub fn essential(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|step| !step.kind.optional())
    }
}

/// Work out what to fetch for a song.
///
/// `txt` is the note file already downloaded from the `gettxt` page — it is what names every
/// other resource, so nothing else can be planned without it.
pub fn plan(
    id: SongId,
    txt: &str,
    song: &SongTxt,
    details: Option<&SongDetails>,
    held: Option<&SyncMeta>,
    folder: &Path,
) -> Plan {
    let artist = song.headers.artist.clone();
    let title = song.headers.title.clone();
    let stem = safe_name(&format!("{artist} - {title}"));
    // Already parsed by the song reader; the `#VIDEO` header is a comma-separated tag list
    // whenever it contains an `=`, and a plain filename when it does not.
    let meta: &MetaTags = &song.meta_tags;

    let mut steps = Vec::new();
    let mut skipped = Vec::new();
    let mut want = |kind: Kind, source: Source| {
        // Already here and intact? Then it is not worth a request. This is what makes a
        // repair cheap and a re-download of a whole library nearly free.
        if let Some(held) = held {
            if let Some(resource) = held.get(kind) {
                let path = folder.join(&resource.file);
                let same_source = match &source {
                    Source::Url(url) => resource.source == *url,
                    Source::Extract { page, .. } => resource.source == *page,
                    Source::Text(_) => true,
                };
                if same_source
                    && std::fs::read(&path)
                        .is_ok_and(|bytes| crate::meta::hash(&bytes) == resource.hash)
                {
                    skipped.push(kind);
                    return;
                }
            }
        }
        steps.push(Step {
            kind,
            source,
            stem: stem.clone(),
        });
    };

    want(Kind::Txt, Source::Text(txt.to_owned()));

    // Audio: the `a=` meta tag names it, and it is a page for yt-dlp rather than a file.
    // Falling back to the video source is right — most songs give one link for both.
    let audio_page = meta.audio.clone().or_else(|| meta.video.clone());
    if let Some(page) = audio_page {
        want(
            Kind::Audio,
            Source::Extract {
                page: watchable(&page),
                audio_only: true,
            },
        );
    }
    if let Some(page) = meta.video.clone() {
        want(
            Kind::Video,
            Source::Extract {
                page: watchable(&page),
                audio_only: false,
            },
        );
    } else if let Some(url) = details.and_then(|d| d.comment_videos.first()) {
        // No meta tag, but somebody posted the video in the comments. That fallback is the
        // difference between a song that plays and a song that is only lyrics.
        want(
            Kind::Video,
            Source::Extract {
                page: url.clone(),
                audio_only: false,
            },
        );
    }

    if let Some(cover) = &meta.cover {
        want(Kind::Cover, Source::Url(cover.source_url()));
    } else if details.and_then(|d| d.cover_url.as_ref()).is_some() {
        // USDB's own cover, which is small but better than none.
        want(
            Kind::Cover,
            Source::Url(rungstar_usdb::client::Usdb::<crate::NoTransport>::cover_url(id)),
        );
    }
    if let Some(background) = &meta.background {
        want(Kind::Background, Source::Url(background.source_url()));
    }

    Plan {
        usdb_id: id,
        artist,
        title,
        folder: stem,
        steps,
        skipped,
    }
}

/// Turn a meta-tag video source into something yt-dlp can open.
///
/// A bare eleven-character id means YouTube — that is the shorthand the format uses and has
/// used since before anything else was supported.
pub fn watchable(source: &str) -> String {
    let source = source.trim();
    if source.contains("://") {
        return source.to_owned();
    }
    if let Some(rest) = source.strip_prefix("v=") {
        return format!("https://www.youtube.com/watch?v={rest}");
    }
    if source.len() == 11
        && source
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return format!("https://www.youtube.com/watch?v={source}");
    }
    format!("https://{source}")
}

/// A folder or file name that every file system will accept.
///
/// Windows is the strict one: the reserved characters, no trailing dot or space, and the
/// device names. A song called `AUX` is rare and a library that cannot be copied to a
/// Windows machine is not.
pub fn safe_name(name: &str) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches('.').trim().to_owned();
    if out.is_empty() {
        out.push_str("song");
    }
    let stem = out.split('.').next().unwrap_or(&out).to_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        out.insert(0, '_');
    }
    // Long enough for any real title, short enough that the whole path fits inside Windows'
    // 260 characters with a folder, a file name and an extension still to come.
    if out.chars().count() > 120 {
        out = out.chars().take(120).collect::<String>().trim().to_owned();
    }
    out
}
