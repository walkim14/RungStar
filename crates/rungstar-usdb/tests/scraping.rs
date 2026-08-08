//! The USDB protocol, driven against saved pages.
//!
//! The fixtures are usdb_syncer's own, copied unmodified. Nothing here touches the network or
//! needs an account, which is the point: the whole protocol is finished and checked before
//! anybody logs in, and a site change shows up as a failing test rather than as an empty
//! catalog nobody notices.

use std::cell::RefCell;

use rungstar_usdb::client::{Order, Request, Transport};
use rungstar_usdb::parse;
use rungstar_usdb::rate::{backoff, Limiter, Rate};
use rungstar_usdb::strings::{Labels, Language};
use rungstar_usdb::{Catalog, Endpoint, Session, SongId, SyncReport, Usdb, UsdbError};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // USDB serves a mix of UTF-8 and Latin-1 and says neither. Lossy on purpose: a page with
    // one bad byte in a comment is still a page worth parsing.
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------- the catalog page

#[test]
fn the_song_list_parses_into_songs() {
    let songs = parse::catalog_page(&fixture("song_list.htm"));
    // The saved page holds three rows; most of its 470 KB is the filter dropdowns.
    assert_eq!(songs.len(), 3);

    let first = &songs[0];
    assert_eq!(first.id, SongId(57));
    assert_eq!(first.artist, "Albert Hammond");
    assert_eq!(first.title, "It Never Rains In Southern California");
    assert_eq!(first.genre, "Soft Rock");
    assert_eq!(first.year, Some(1972));
    assert_eq!(first.language, "English");
    assert_eq!(first.creator, "Canni");
    assert_eq!(first.views, 446);
    assert!(!first.golden_notes);
    assert_eq!(first.last_change, 1_204_046_489);
    assert!(
        first
            .sample_url
            .as_deref()
            .is_some_and(|url| url.contains("itunes")),
        "the preview URL was not found: {:?}",
        first.sample_url
    );
}

#[test]
fn a_rating_is_counted_from_the_pictures_and_the_empty_star_does_not_count() {
    // The number is nowhere else on the page. `star2.png` is the empty one, and matching it
    // by accident would rate every song five.
    assert_eq!(parse::rating(r#"<img src="images/star2.png">"#), 0.0);
    assert_eq!(
        parse::rating(r#"<img src="images/star.png"> <img src="images/star2.png">"#),
        1.0
    );
    assert_eq!(
        parse::rating(r#"<img src="images/star.png"><img src="images/half_star.png">"#),
        1.5
    );

    // And across the real page, nothing is rated above five.
    for song in parse::catalog_page(&fixture("song_list.htm")) {
        assert!(
            (0.0..=5.0).contains(&song.rating),
            "{} is rated {}",
            song.title,
            song.rating
        );
    }
}

#[test]
fn every_row_of_a_real_page_has_an_artist_and_a_title() {
    // The failure this guards against is a regex that matches the row but slips a cell, which
    // shows up as a catalog of blank titles rather than as an error.
    let songs = parse::catalog_page(&fixture("song_list.htm"));
    for song in &songs {
        assert!(!song.artist.is_empty(), "{:?} has no artist", song.id);
        assert!(!song.title.is_empty(), "{:?} has no title", song.id);
        assert!(song.id.0 > 0);
        assert!(song.last_change > 0, "{:?} has no edit time", song.id);
    }
    // Ids are unique: a duplicated row means the split between rows is wrong.
    let mut ids: Vec<i64> = songs.iter().map(|s| s.id.0).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "a row was parsed twice");
}

#[test]
fn markup_that_is_not_a_song_list_yields_nothing_rather_than_rubbish() {
    assert!(parse::catalog_page("<html><body>nothing here</body></html>").is_empty());
    assert!(parse::catalog_page("").is_empty());
}

// ---------------------------------------------------------------- the detail page

#[test]
fn a_song_page_parses_into_details() {
    let page = fixture("song_page_with_embedded_video.htm");
    let details = parse::details(&page, SongId(26152)).expect("a good page");
    assert_eq!(details.artist, "Revolverheld");
    assert_eq!(details.language, "German");
    assert_eq!(details.year, Some(2013));
    assert_eq!(details.genre, "Pop");
    assert_eq!(details.bpm, Some(276.17));
    assert_eq!(details.gap, Some(120000.0));
    assert!(details.golden_notes, "golden notes said Yes");
    assert!(!details.song_check, "songcheck said No");
    assert_eq!(details.uploaded_by, "bohning");
    assert_eq!(details.views, 27);
    assert_eq!(details.rating, 5.0);
    assert_eq!(details.date, "10.10.22 - 19:47");
    assert_eq!(details.cover_url.as_deref(), Some("data/cover/26152.jpg"));
}

#[test]
fn a_song_page_with_no_cover_says_so_rather_than_guessing_one() {
    let page = fixture("song_page_without_comments_or_cover.htm");
    let details = parse::details(&page, SongId(1)).expect("a good page");
    // The page does have an `<img>`: it points at a placeholder. Taking that at face value
    // downloads a picture of the words "no cover" and files it as the artwork.
    assert_eq!(details.cover_url, None);
    // The rest of the page still parses. Half a page is worth more than none.
    assert!(!details.artist.is_empty());
}

#[test]
fn a_video_linked_in_the_comments_is_found() {
    // Most songs name their video in the #VIDEO meta tag. When they do not, a comment saying
    // "here is the video" is what everybody uses, and that fallback is the difference between
    // a song that plays and a song that is only lyrics.
    for name in [
        "song_page_with_embedded_video.htm",
        "song_page_with_unembedded_video.htm",
    ] {
        let videos = parse::comment_videos(&fixture(name));
        assert!(!videos.is_empty(), "{name} had no video link");
        for url in &videos {
            assert!(url.starts_with("https://www.youtube.com/watch?v="), "{url}");
            assert_eq!(url.len(), 43, "{url} is not an eleven-character id");
        }
        // The same video is usually linked twice, embedded and as a link.
        let mut unique = videos.clone();
        unique.dedup();
        assert_eq!(unique.len(), videos.len(), "the same video came back twice");
    }
}

// ---------------------------------------------------------------- the note file

#[test]
fn the_note_file_comes_out_of_the_text_area() {
    let text = parse::song_txt(&fixture("txt_page.htm")).expect("a good page");
    assert!(
        text.contains("#TITLE:"),
        "not a song file: {:?}",
        &text[..80]
    );
    assert!(text.contains("#ARTIST:"));
    assert!(text.contains("#BPM:"));
    // Entities have to be undone or the lyrics arrive full of &amp;.
    assert!(!text.contains("&amp;"), "the text was not unescaped");
    assert!(!text.contains("&lt;"));
    // And it must be a song the parser proper accepts, which is the real test of this.
    assert!(
        text.lines()
            .any(|line| line.starts_with(':') || line.starts_with('*')),
        "no note lines came out"
    );
}

#[test]
fn a_page_with_no_text_area_is_an_error_rather_than_an_empty_song() {
    let error = parse::song_txt("<html>nothing</html>").unwrap_err();
    assert!(matches!(error, UsdbError::Unexpected(_)));
}

// ---------------------------------------------------------------- errors in 200s

#[test]
fn an_error_dressed_as_a_two_hundred_is_still_an_error() {
    // The single most important thing about this site: it answers a request for a private
    // page, or for a song that does not exist, with a perfectly ordinary 200 and an
    // explanation in the body. A client that trusts status codes reports an empty catalog.
    let private = "<html>You are not logged in. Login to use this function.</html>";
    assert!(matches!(
        parse::check_page(private),
        Err(UsdbError::NotLoggedIn)
    ));
    assert!(matches!(
        parse::song_txt(private),
        Err(UsdbError::NotLoggedIn)
    ));
    assert!(matches!(
        parse::details(private, SongId(1)),
        Err(UsdbError::NotLoggedIn)
    ));

    let missing = "<html>Datensatz nicht gefunden</html>";
    assert!(matches!(
        parse::check_page(missing),
        Err(UsdbError::NotFound)
    ));

    // A real page is not mistaken for either.
    assert!(parse::check_page(&fixture("txt_page.htm")).is_ok());
}

// ---------------------------------------------------------------- who is logged in

#[test]
fn the_welcome_banner_says_who_is_logged_in() {
    assert_eq!(
        parse::logged_in_as(&fixture("song_list.htm")).as_deref(),
        Some("Ultorex")
    );
    // And a logged-out page says nobody, rather than reporting a session that does not exist.
    assert_eq!(
        parse::logged_in_as(&fixture("song_page_with_embedded_video.htm")),
        None
    );
    assert_eq!(parse::logged_in_as("<html></html>"), None);
}

#[test]
fn the_page_language_is_detected_rather_than_configured() {
    // The site answers in whatever the account is set to, and every label lookup depends on
    // getting this right. Configuring it once is how a scraper returns nothing for the half
    // of its users whose account is German.
    assert_eq!(
        Labels::detect("<b>Willkommen</b>").language,
        Language::German
    );
    assert_eq!(
        Labels::detect("<b>Bienvenue</b>").language,
        Language::French
    );
    assert_eq!(Labels::detect("<b>Welcome</b>").language, Language::English);
    // An unrecognised page falls back rather than failing: the numbers on it do not depend
    // on the language at all.
    assert_eq!(Labels::detect("<b>...</b>").language, Language::English);
}

#[test]
fn a_german_detail_page_parses_by_its_own_labels() {
    // Built rather than saved, because the fixtures are all English and this is exactly the
    // case that the reference's own users hit.
    let page = "<html><b>Willkommen</b><table>\
        <tr class=\"list_head\"><td>Nena</td><td>99 Luftballons</td></tr>\
        <tr><td>Sprache</td><td>German</td></tr>\
        <tr><td>Jahr</td><td>1983</td></tr>\
        <tr><td>Goldene Noten</td><td>Ja <img src=\"images/yes_small.png\"></td></tr>\
        <tr><td>Aufrufe</td><td>1234</td></tr>\
        </table></html>";
    let details = parse::details(page, SongId(7)).expect("a good page");
    assert_eq!(details.artist, "Nena");
    assert_eq!(details.title, "99 Luftballons");
    assert_eq!(details.language, "German");
    assert_eq!(details.year, Some(1983));
    assert!(details.golden_notes, "Ja was not understood as yes");
    assert_eq!(details.views, 1234);
}

// ---------------------------------------------------------------- entities

#[test]
fn entities_are_undone_including_the_numeric_ones() {
    assert_eq!(parse::unescape("Salt &amp; Pepa"), "Salt & Pepa");
    assert_eq!(parse::unescape("&lt;b&gt;"), "<b>");
    assert_eq!(parse::unescape("Don&#39;t"), "Don't");
    assert_eq!(parse::unescape("caf&#233;"), "café");
    assert_eq!(parse::unescape("&#x41;"), "A");
    // Something that only looks like an entity is left alone rather than eaten.
    assert_eq!(parse::unescape("R&B"), "R&B");
    assert_eq!(parse::unescape("100% & rising"), "100% & rising");
    assert_eq!(parse::unescape("&notanentity;"), "&notanentity;");
    assert_eq!(parse::unescape("plain"), "plain");
}

// ---------------------------------------------------------------- requests

#[test]
fn every_endpoint_builds_the_request_usdb_expects() {
    let list = Endpoint::List {
        start: 200,
        order: Order::LastChange,
    }
    .request();
    assert!(list.post, "the catalog is a POST");
    assert!(list.params.contains(&("link".into(), "list".into())));
    assert!(list.body.contains(&("start".into(), "200".into())));
    assert!(list.body.contains(&("limit".into(), "100".into())));
    assert!(list.body.contains(&("details".into(), "1".into())));
    // Newest first, which is what makes an incremental sync possible at all.
    assert!(list.body.contains(&("ud".into(), "desc".into())));
    assert!(list.body.contains(&("order".into(), "lastchange".into())));

    let detail = Endpoint::Detail(SongId(42)).request();
    assert!(!detail.post);
    assert!(detail.params.contains(&("id".into(), "42".into())));

    // The note file needs `wd=1` or the page returns a download prompt instead of the text.
    let txt = Endpoint::Txt(SongId(42)).request();
    assert!(txt.post);
    assert!(txt.body.contains(&("wd".into(), "1".into())));
    assert!(txt.params.contains(&("link".into(), "gettxt".into())));

    let login = Endpoint::Login {
        user: "somebody".into(),
        password: "hunter2".into(),
    }
    .request();
    assert!(login.post);
    assert!(login.body.contains(&("login".into(), "Login".into())));
}

// ---------------------------------------------------------------- rate limiting

#[test]
fn a_burst_goes_straight_through_and_a_crawl_is_paced() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let mut limiter = Limiter::new(Rate {
        per_second: 2.0,
        burst: 4.0,
    });
    // Four in hand: opening a song and fetching its text must not wait.
    for _ in 0..4 {
        assert_eq!(limiter.take(start), Duration::ZERO);
    }
    // The fifth waits, because a sustained crawl is what the limit is for.
    let wait = limiter.take(start);
    assert!(wait > Duration::ZERO, "the bucket never emptied");
    assert!(wait <= Duration::from_millis(600), "{wait:?} is too long");

    // And it refills.
    let later = start + Duration::from_secs(10);
    assert_eq!(limiter.take(later), Duration::ZERO);
}

#[test]
fn backoff_grows_is_capped_and_is_jittered() {
    use std::time::Duration;
    let base = Duration::from_millis(500);
    let cap = Duration::from_secs(30);
    let first = backoff(0, base, cap, 500);
    let second = backoff(1, base, cap, 500);
    let third = backoff(2, base, cap, 500);
    assert!(second > first && third > second, "it did not grow");
    assert!(backoff(20, base, cap, 500) <= cap + cap / 2, "uncapped");

    // Jittered, or every client that saw the same outage retries at the same instant and the
    // server gets the spike twice.
    let a = backoff(3, base, cap, 10);
    let b = backoff(3, base, cap, 990);
    assert_ne!(a, b, "the retry delay is not jittered");
}

// ---------------------------------------------------------------- the session

/// A transport that hands back canned pages and counts what was asked for.
struct Canned {
    pages: RefCell<Vec<Result<String, UsdbError>>>,
    asked: RefCell<Vec<Request>>,
}

impl Canned {
    fn new(pages: Vec<Result<String, UsdbError>>) -> Self {
        Self {
            pages: RefCell::new(pages),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl Transport for Canned {
    fn fetch(&self, request: &Request) -> Result<String, UsdbError> {
        self.asked.borrow_mut().push(request.clone());
        let mut pages = self.pages.borrow_mut();
        if pages.is_empty() {
            return Ok(String::new());
        }
        pages.remove(0)
    }
}

fn fast() -> Rate {
    // No pacing in tests: the limiter has its own, and a paced test is a slow one.
    Rate {
        per_second: 1_000_000.0,
        burst: 1_000_000.0,
    }
}

#[test]
fn a_session_notices_who_it_is_from_any_page() {
    let transport = Canned::new(vec![Ok(fixture("song_list.htm"))]);
    let mut usdb = Usdb::with_rate(transport, fast());
    assert_eq!(usdb.session(), Session::Anonymous);
    usdb.catalog_page(0, Order::LastChange).unwrap();
    assert_eq!(usdb.session(), Session::LoggedIn("Ultorex".to_owned()));
}

#[test]
fn a_refused_login_is_not_retried() {
    // Repeating a request that was refused for a reason that will not change is how an
    // account gets locked.
    let transport = Canned::new(vec![Ok(
        "<html>Login or Password invalid, please try again.</html>".to_owned(),
    )]);
    let mut usdb = Usdb::with_rate(transport, fast());
    let error = usdb
        .log_in(&rungstar_usdb::Credentials {
            user: "somebody".into(),
            password: "wrong".into(),
        })
        .unwrap_err();
    assert!(matches!(error, UsdbError::BadCredentials));
}

/// A page of exactly one hundred songs, which is what a full catalog page looks like.
///
/// Built by repeating the fixture's rows with fresh ids, because the saved page holds three
/// and a crawl only continues past a page that came back full.
fn full_page() -> String {
    let page = fixture("song_list.htm");
    let start = page.find("<tr class=\"list_tr").expect("a row");
    // Exactly one row: from the first to the next, or the whole tail if there is no next.
    let length = page[start + 4..]
        .find("<tr class=\"list_tr")
        .map_or(page.len() - start, |at| at + 4);
    let row = &page[start..start + length];
    let mut out = String::from("<html>");
    for id in 0..100 {
        out.push_str(&row.replace(
            "data-songid=\"57\"",
            &format!("data-songid=\"{}\"", 1000 + id),
        ));
    }
    out
}

#[test]
fn a_crawl_stops_when_the_caller_says_so() {
    // Three hundred requests is a long time to be uninterruptible, and it is also how an
    // incremental sync stops at the songs it already has.
    let page = full_page();
    let transport = Canned::new(vec![Ok(page.clone()), Ok(page.clone()), Ok(page)]);
    let mut usdb = Usdb::with_rate(transport, fast());
    let mut pages = 0;
    usdb.catalog(Order::LastChange, |page| {
        assert_eq!(page.len(), 100, "the synthesised page is not full");
        pages += 1;
        pages < 2
    })
    .unwrap();
    assert_eq!(pages, 2, "the crawl did not stop when asked");
}

#[test]
fn a_crawl_stops_on_its_own_at_a_short_page() {
    // USDB gives no total, so a page with fewer songs than the limit is how the end is known.
    let transport = Canned::new(vec![Ok(full_page()), Ok(fixture("song_list.htm"))]);
    let mut usdb = Usdb::with_rate(transport, fast());
    let mut pages = 0;
    let total = usdb
        .catalog(Order::LastChange, |_| {
            pages += 1;
            true
        })
        .unwrap();
    assert_eq!(pages, 2);
    assert_eq!(total, 103);
}

// ---------------------------------------------------------------- the catalog

fn song(id: i64, artist: &str, title: &str, changed: i64) -> rungstar_usdb::CatalogSong {
    rungstar_usdb::CatalogSong {
        id: SongId(id),
        last_change: changed,
        artist: artist.into(),
        title: title.into(),
        genre: String::new(),
        year: None,
        edition: String::new(),
        language: String::new(),
        creator: String::new(),
        golden_notes: false,
        rating: 0.0,
        views: 0,
        sample_url: None,
    }
}

#[test]
fn absorbing_a_page_counts_what_was_new() {
    let mut catalog = Catalog::new();
    let mut report = SyncReport::default();
    catalog.absorb(
        &[song(1, "A", "One", 100), song(2, "B", "Two", 200)],
        &mut report,
    );
    assert_eq!((report.added, report.updated, report.unchanged), (2, 0, 0));
    assert_eq!(catalog.high_water(), 200);

    // The same songs again change nothing.
    let mut report = SyncReport::default();
    catalog.absorb(
        &[song(1, "A", "One", 100), song(2, "B", "Two", 200)],
        &mut report,
    );
    assert_eq!((report.added, report.updated, report.unchanged), (0, 0, 2));

    // An edit does.
    let mut report = SyncReport::default();
    catalog.absorb(&[song(1, "A", "One, revised", 300)], &mut report);
    assert_eq!((report.added, report.updated), (0, 1));
    assert_eq!(catalog.get(SongId(1)).unwrap().title, "One, revised");
    assert_eq!(catalog.high_water(), 300);
}

#[test]
fn a_sync_stops_once_a_whole_page_is_older_than_what_is_held() {
    // A whole page rather than one song: USDB's edit time has one-second resolution, so
    // several songs edited in the same second can straddle a page boundary and stopping on
    // the first old one would miss the rest.
    let catalog = Catalog::new();
    let page = [song(1, "A", "One", 100), song(2, "B", "Two", 90)];
    assert!(catalog.caught_up(&page, 100), "all of it is old");
    assert!(!catalog.caught_up(&page, 95), "one of them is newer");
    assert!(
        !catalog.caught_up(&[], 100),
        "an empty page decides nothing"
    );
}

#[test]
fn the_catalog_searches_by_artist_and_title_ignoring_accents() {
    let mut catalog = Catalog::new();
    let mut report = SyncReport::default();
    catalog.absorb(
        &[
            song(1, "Björk", "Army Of Me", 1),
            song(2, "Abba", "Waterloo", 2),
            song(3, "Blur", "Song 2", 3),
        ],
        &mut report,
    );
    assert_eq!(catalog.search("bjork").len(), 1, "accents are not folded");
    assert_eq!(catalog.search("BJÖRK").len(), 1);
    assert_eq!(catalog.search("abba waterloo").len(), 1);
    assert_eq!(
        catalog.search("waterloo abba").len(),
        1,
        "the word order should not matter"
    );
    assert_eq!(catalog.search("").len(), 3);
    assert_eq!(catalog.search("nothing here").len(), 0);
}

#[test]
fn a_catalog_survives_being_saved_and_read_back() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usdb").join("catalog.json");

    let mut catalog = Catalog::new();
    let mut report = SyncReport::default();
    let mut original = song(57, "Albert Hammond", "It Never Rains", 1_204_046_489);
    original.rating = 3.5;
    original.year = Some(1972);
    original.golden_notes = true;
    catalog.absorb(&[original.clone()], &mut report);
    catalog.save(&path).unwrap();

    let read_back = Catalog::load(&path).unwrap();
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back.get(SongId(57)), Some(&original));
    assert_eq!(read_back.high_water(), 1_204_046_489);
}

#[test]
fn a_missing_catalog_is_a_first_run_rather_than_a_failure() {
    let directory = tempfile::tempdir().unwrap();
    let catalog = Catalog::load(&directory.path().join("nothing.json")).unwrap();
    assert!(catalog.is_empty());
}

#[test]
fn a_real_page_syncs_into_a_searchable_catalog() {
    // The whole path, end to end, with no network: fetch, parse, absorb, search.
    let transport = Canned::new(vec![Ok(fixture("song_list.htm"))]);
    let mut usdb = Usdb::with_rate(transport, fast());
    let mut catalog = Catalog::new();
    let mut report = SyncReport::default();
    let page = usdb.catalog_page(0, Order::LastChange).unwrap();
    catalog.absorb(&page, &mut report);

    assert_eq!(report.added, 3);
    assert_eq!(catalog.len(), report.added);
    let found = catalog.search("hammond");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "It Never Rains In Southern California");
}
