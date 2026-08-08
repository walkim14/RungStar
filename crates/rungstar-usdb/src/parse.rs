//! Turning USDB's HTML into values.
//!
//! Quarantined here on purpose. USDB is a PHP site from 2008 with no API, no versioning and no
//! promise not to change; when it does change, this is the file that breaks and the only file
//! that has to be fixed. Everything above it works on the structs at the bottom of this module.
//!
//! Two things about the site drive the design. **Errors are HTML bodies, not status codes** —
//! a page for a song that does not exist is a 200 whose text says "Datensatz nicht gefunden" —
//! so [`check_page`] runs on every response before anything else looks at it. And **the page
//! language varies per account**, so every label lookup goes through [`Labels`] detected from
//! the response rather than from a setting.

use std::sync::OnceLock;

use regex::Regex;

use crate::strings::{fixed, Labels};
use crate::{SongId, UsdbError};

/// One row of the catalog, as the song list gives it.
///
/// Deliberately flat and owned: thirty thousand of these come back from a full sync and go
/// straight into SQLite.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogSong {
    pub id: SongId,
    /// Unix seconds of the last edit on USDB. This is what makes an incremental sync possible.
    pub last_change: i64,
    pub artist: String,
    pub title: String,
    pub genre: String,
    pub year: Option<i32>,
    pub edition: String,
    pub language: String,
    pub creator: String,
    pub golden_notes: bool,
    /// Zero to five, in halves.
    pub rating: f32,
    pub views: i64,
    /// An audio preview, when the row has one. Not on USDB's own server.
    pub sample_url: Option<String>,
}

/// Everything the detail page adds to a catalog row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SongDetails {
    pub id: SongId,
    pub artist: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub language: String,
    pub year: Option<i32>,
    pub genre: String,
    pub edition: String,
    pub bpm: Option<f64>,
    pub gap: Option<f64>,
    pub golden_notes: bool,
    pub song_check: bool,
    /// As USDB prints it: `dd.mm.yy - HH:MM`. Kept verbatim rather than parsed, because it is
    /// shown and never compared — `last_change` from the list is the field that is compared.
    pub date: String,
    pub uploaded_by: String,
    pub views: i64,
    pub rating: f32,
    /// Video links found in the comments, in the order they appear. The first is the one the
    /// downloader tries first.
    pub comment_videos: Vec<String>,
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a regex in this file is malformed"))
}

/// Reject a page that is an error dressed as a 200.
///
/// Run before parsing anything. USDB answers a request for a private page, or for a song that
/// does not exist, with a perfectly ordinary 200 and an explanation in the body — so a client
/// that trusts status codes silently parses the error page and reports an empty catalog.
pub fn check_page(page: &str) -> Result<(), UsdbError> {
    if let Some(at) = page.find(fixed::NOT_LOGGED_IN) {
        // But only when the site means it about the *page*. An ordinary song page carries the
        // same sentence inside its comment box, telling you to log in before commenting, and
        // treating that as a refusal makes every song unreadable to anybody browsing without
        // an account. Inside a form it is a label; outside one it is the answer.
        if !inside_a_form(page, at) {
            return Err(UsdbError::NotLoggedIn);
        }
    }
    if page.contains(fixed::NOT_FOUND) {
        return Err(UsdbError::NotFound);
    }
    Ok(())
}

/// Whether the text at `at` sits inside a `<form>`.
fn inside_a_form(page: &str, at: usize) -> bool {
    let before = &page[..at];
    let opened = before.rfind("<form");
    let closed = before.rfind("</form>");
    match (opened, closed) {
        (Some(opened), Some(closed)) => opened > closed,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Whether a page shows somebody logged in, and who.
pub fn logged_in_as(page: &str) -> Option<String> {
    if page.contains(fixed::PLEASE_LOGIN) {
        return None;
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = regex(
        &RE,
        r"(?s)<td class='row3' colspan='2'>\s*<span class='gen'>([^<]+) <b>([^<]+)</b>",
    );
    let found = re.captures(page)?;
    let greeting = found.get(1)?.as_str().trim();
    // Only a real welcome counts. "Welcome, Please login" is caught above, but a future
    // wording change should fail closed rather than report a session that does not exist.
    if !crate::strings::ALL
        .iter()
        .any(|labels| greeting.starts_with(labels.welcome))
    {
        return None;
    }
    Some(unescape(found.get(2)?.as_str()).trim().to_owned())
}

/// Stars out of five, counted from the images in a cell.
///
/// Counting pictures is not a choice: the number is nowhere else on the page. `star2.png` is
/// the empty star and must not match, which is why the slash is part of the needle.
pub fn rating(cell: &str) -> f32 {
    cell.matches("/star.png").count() as f32 + 0.5 * cell.matches("/half_star.png").count() as f32
}

/// Every song on one page of the catalog.
///
/// One regex over the whole row rather than an HTML parse. The reference does the same, and
/// for a reason worth keeping: the page is 470 KB of malformed markup per hundred songs, a
/// tolerant DOM parse of it costs more than the request did, and a regex that stops matching
/// is a loud failure rather than a subtly wrong tree.
pub fn catalog_page(page: &str) -> Vec<CatalogSong> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = regex(
        &RE,
        r#"(?s)<tr class="list_tr\d"\s+data-songid="(\d+)"\s+data-lastchange="(\d+)"[^>]*?>\s*"#
            .to_owned()
            .as_str(),
    );
    let mut songs = Vec::new();
    let starts: Vec<usize> = re.find_iter(page).map(|m| m.start()).collect();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(page.len());
        if let Some(song) = catalog_row(&page[*start..end]) {
            songs.push(song);
        }
    }
    songs
}

/// One `<tr>` of the catalog.
fn catalog_row(row: &str) -> Option<CatalogSong> {
    static HEAD: OnceLock<Regex> = OnceLock::new();
    let head = regex(&HEAD, r#"data-songid="(\d+)"\s+data-lastchange="(\d+)""#).captures(row)?;
    let id = SongId(head.get(1)?.as_str().parse().ok()?);
    let last_change = head.get(2)?.as_str().parse().unwrap_or(0);

    // The cells in order, which is how the site lays them out and the only thing it is
    // consistent about: sample, cover, artist, title, genre, year, edition, golden, language,
    // creator, rating, views, and a download link.
    static CELL: OnceLock<Regex> = OnceLock::new();
    let cells: Vec<&str> = regex(&CELL, r"(?s)<td[^>]*>(.*?)</td>")
        .captures_iter(row)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    if cells.len() < 12 {
        return None;
    }

    static SAMPLE: OnceLock<Regex> = OnceLock::new();
    let sample_url = regex(&SAMPLE, r#"<source src="([^"]+)""#)
        .captures(cells[0])
        .and_then(|c| c.get(1))
        .map(|m| unescape(m.as_str()));

    let text = |cell: &str| unescape(&strip_tags(cell)).trim().to_owned();
    let golden = text(cells[7]);
    Some(CatalogSong {
        id,
        last_change,
        artist: text(cells[2]),
        title: text(cells[3]),
        genre: text(cells[4]),
        year: text(cells[5]).parse().ok(),
        edition: text(cells[6]),
        // Compared against every language's word for yes, because the row is served in
        // whichever the account is set to and this cell is a word rather than an image.
        golden_notes: crate::strings::ALL
            .iter()
            .any(|labels| golden.eq_ignore_ascii_case(labels.yes)),
        language: text(cells[8]),
        creator: text(cells[9]),
        rating: rating(cells[10]),
        views: text(cells[11])
            .replace([' ', '.', ','], "")
            .parse()
            .unwrap_or(0),
        sample_url,
    })
}

/// The detail page of one song.
pub fn details(page: &str, id: SongId) -> Result<SongDetails, UsdbError> {
    check_page(page)?;
    let labels = Labels::detect(page);

    static PAIR: OnceLock<Regex> = OnceLock::new();
    let pairs = regex(&PAIR, r"(?s)<td[^>]*>(.*?)</td>\s*<td[^>]*>(.*?)</td>");
    let mut details = SongDetails {
        id,
        ..SongDetails::default()
    };

    // The first row of the table is the artist and the title, with no labels on them.
    static HEAD: OnceLock<Regex> = OnceLock::new();
    if let Some(found) = regex(
        &HEAD,
        r#"(?s)<tr class="list_head">\s*<td[^>]*>(.*?)</td>\s*<td[^>]*>(.*?)</td>"#,
    )
    .captures(page)
    {
        details.artist = clean(found.get(1).map_or("", |m| m.as_str()));
        details.title = clean(found.get(2).map_or("", |m| m.as_str()));
    }

    for found in pairs.captures_iter(page) {
        let label = clean(found.get(1).map_or("", |m| m.as_str()));
        let raw = found.get(2).map_or("", |m| m.as_str());
        let value = clean(raw);
        // Matched by label rather than by position: the site adds rows (an audio sample, a
        // team comment) between releases, and counting cells breaks the moment it does.
        match label.as_str() {
            "Cover" => {
                static IMG: OnceLock<Regex> = OnceLock::new();
                details.cover_url = regex(&IMG, r#"<img src="([^"]+)""#)
                    .captures(raw)
                    .and_then(|c| c.get(1))
                    .map(|m| unescape(m.as_str()))
                    // A song with no cover still gets an `<img>`, pointing at a placeholder.
                    // Taking that at face value downloads a picture of the words "no cover"
                    // and files it as the artwork.
                    .filter(|url| !url.to_lowercase().contains("nocover"));
            }
            l if l == labels.song_language => details.language = value,
            l if l == labels.year => details.year = value.parse().ok(),
            l if l == labels.genre => details.genre = value,
            l if l == labels.edition => details.edition = value,
            l if l == labels.bpm => details.bpm = value.replace(',', ".").parse().ok(),
            l if l == labels.gap => details.gap = value.replace(',', ".").parse().ok(),
            l if l == labels.golden_notes => details.golden_notes = says_yes(&value, labels),
            l if l == labels.songcheck => details.song_check = says_yes(&value, labels),
            l if l == labels.date => details.date = value,
            l if l == labels.uploaded_by => details.uploaded_by = value,
            l if l == labels.views => details.views = value.parse().unwrap_or(0),
            l if l == labels.rating => details.rating = rating(raw),
            _ => {}
        }
    }

    details.comment_videos = comment_videos(page);
    Ok(details)
}

fn says_yes(value: &str, labels: Labels) -> bool {
    value
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case(labels.yes))
}

/// Video links posted in the comments, in the order they appear.
///
/// The `#VIDEO` meta tag is the proper place for these, and most songs have one. When they do
/// not, a comment saying "here is the video" is what everybody actually uses, so the
/// downloader falls back to it — that fallback is the difference between a song that plays
/// and a song that is only lyrics.
pub fn comment_videos(page: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = regex(
        &RE,
        // Every shape USDB's comments contain. The `v=` one is not anchored to the start of
        // the query: a link copied from the site itself arrives as
        // `watch?feature=player_detailpage&v=...`, and requiring `watch?v=` misses it.
        r#"(?i)(?:youtu\.be/|youtube\.com/(?:embed/|v/)|youtube\.com/watch\?[^\s"'<>]*?v=)([A-Za-z0-9_-]{11})"#,
    );
    let mut found = Vec::new();
    for capture in re.captures_iter(page) {
        let Some(id) = capture.get(1) else { continue };
        let url = format!("https://www.youtube.com/watch?v={}", id.as_str());
        // The same video is usually linked twice on a page — once embedded, once as a link.
        if !found.contains(&url) {
            found.push(url);
        }
    }
    found
}

/// The song file itself, out of the `<textarea>` the download page puts it in.
pub fn song_txt(page: &str) -> Result<String, UsdbError> {
    check_page(page)?;
    static RE: OnceLock<Regex> = OnceLock::new();
    let found = regex(&RE, r"(?s)<textarea[^>]*>(.*?)</textarea>")
        .captures(page)
        .and_then(|c| c.get(1))
        .ok_or(UsdbError::Unexpected(
            "the download page had no text area in it",
        ))?;
    let text = unescape(found.as_str());
    if text.trim().is_empty() {
        return Err(UsdbError::Unexpected("the download page was empty"));
    }
    Ok(text)
}

/// Drop every tag, keeping the text between them.
fn strip_tags(html: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex(&RE, r"(?s)<[^>]*>")
        .replace_all(html, "")
        .into_owned()
}

fn clean(html: &str) -> String {
    unescape(&strip_tags(html)).trim().to_owned()
}

/// The five XML entities plus the numeric forms, which is what USDB emits.
///
/// Hand-written rather than a crate: this is the whole of what the site produces, and the
/// alternative is a dependency for five replacements.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|end| *end <= 10);
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            other => other
                .strip_prefix('#')
                .and_then(
                    |number| match number.strip_prefix('x').or(number.strip_prefix('X')) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => number.parse().ok(),
                    },
                )
                .and_then(char::from_u32),
        };
        match replacement {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}
