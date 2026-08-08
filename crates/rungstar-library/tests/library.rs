//! Scanning and searching a real directory of song files.

use std::fs;
use std::path::{Path, PathBuf};

use rungstar_library::{scan, Database, ScanOptions, SearchField, SearchQuery, SortKey};

/// Write a playable song, with an audio file beside it so it counts as playable.
fn write_song(root: &Path, folder: &str, artist: &str, title: &str, extra: &str) -> PathBuf {
    let dir = root.join(folder).join(format!("{artist} - {title}"));
    fs::create_dir_all(&dir).expect("create song dir");
    let audio = format!("{artist} - {title}.mp3");
    fs::write(dir.join(&audio), b"not really audio").expect("write audio");

    let path = dir.join(format!("{artist} - {title}.txt"));
    let text = format!(
        "#TITLE:{title}\n#ARTIST:{artist}\n#MP3:{audio}\n#BPM:300\n#GAP:0\n{extra}\
         : 0 4 0 Hel\n: 6 4 2 lo \n- 12\n: 14 4 5 world\n: 20 4 0 again\nE\n"
    );
    fs::write(&path, text).expect("write song");
    path
}

fn library(root: &Path) -> Database {
    let mut database = Database::in_memory().expect("open index");
    scan(&mut database, &ScanOptions::new([root.to_path_buf()])).expect("scan");
    database
}

#[test]
fn a_scan_finds_songs_and_a_rescan_skips_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "Alpha", "One", "");
    write_song(root, "Pop", "Beta", "Two", "");
    write_song(root, "Rock", "Gamma", "Three", "");

    let mut database = Database::in_memory().unwrap();
    let options = ScanOptions::new([root.to_path_buf()]);

    let first = scan(&mut database, &options).unwrap();
    assert_eq!(first.added, 3);
    assert_eq!(first.unchanged, 0);
    assert_eq!(database.count().unwrap(), 3);

    // Nothing has changed, so nothing should be read again.
    let second = scan(&mut database, &options).unwrap();
    assert_eq!(second.added, 0);
    assert_eq!(second.updated, 0);
    assert_eq!(second.unchanged, 3);
}

#[test]
fn a_deleted_song_leaves_the_index() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doomed = write_song(root, "Pop", "Alpha", "One", "");
    write_song(root, "Pop", "Beta", "Two", "");

    let mut database = Database::in_memory().unwrap();
    let options = ScanOptions::new([root.to_path_buf()]);
    scan(&mut database, &options).unwrap();

    fs::remove_file(&doomed).unwrap();
    let report = scan(&mut database, &options).unwrap();
    assert_eq!(report.removed, 1);
    assert_eq!(database.count().unwrap(), 1);
}

#[test]
fn the_folder_category_is_the_directory_below_the_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Eurovision", "Alpha", "One", "");
    let database = library(root);

    let songs = database.search(&SearchQuery::all()).unwrap();
    assert_eq!(songs[0].folder, "Eurovision");
}

#[test]
fn songs_are_searchable_by_artist_and_title() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "The Beatles", "Hey Jude", "");
    write_song(root, "Pop", "Queen", "Bohemian Rhapsody", "");
    let database = library(root);

    let hits = database
        .search(&SearchQuery::all().text("beatles"))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Hey Jude");

    // Prefixes match as you type, and word order does not matter.
    let hits = database
        .search(&SearchQuery::all().text("rhap bohem"))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].artist, "Queen");
}

#[test]
fn accents_do_not_have_to_be_typed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "Björk", "Jóga", "");
    let database = library(root);

    let hits = database.search(&SearchQuery::all().text("bjork")).unwrap();
    assert_eq!(hits.len(), 1, "diacritics should be folded away");
}

#[test]
fn lyrics_are_searchable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "Alpha", "One", "");
    let database = library(root);

    // Nothing in the metadata says "world"; it only appears in the lyrics.
    let query = SearchQuery::all().text("world").field(SearchField::Lyrics);
    assert_eq!(database.search(&query).unwrap().len(), 1);
}

#[test]
fn a_misspelling_still_finds_the_song() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "The Beatles", "Hey Jude", "");
    write_song(root, "Pop", "Queen", "Bohemian Rhapsody", "");
    let database = library(root);

    let hits = database
        .search(&SearchQuery::all().text("beatls jud"))
        .unwrap();
    assert_eq!(hits.len(), 1, "should fall back to similarity");
    assert_eq!(hits[0].title, "Hey Jude");

    // Something genuinely absent still returns nothing.
    assert!(database
        .search(&SearchQuery::all().text("zzzzqqqq"))
        .unwrap()
        .is_empty());
}

#[test]
fn results_can_be_sorted_and_the_order_is_stable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "Zebra", "Song", "#YEAR:1999\n");
    write_song(root, "Pop", "Ärzte", "Song", "#YEAR:1985\n");
    write_song(root, "Pop", "Alpha", "Song", "");
    let database = library(root);

    let by_artist = database
        .search(&SearchQuery::all().sort(SortKey::Artist, false))
        .unwrap();
    let names: Vec<&str> = by_artist.iter().map(|s| s.artist.as_str()).collect();
    assert_eq!(
        names,
        vec!["Alpha", "Ärzte", "Zebra"],
        "accents sort as their base letter"
    );

    // A song with no year belongs at the end of a year sort, not the top.
    let by_year = database
        .search(&SearchQuery::all().sort(SortKey::Year, false))
        .unwrap();
    assert_eq!(by_year[0].year, Some(1985));
    assert_eq!(by_year.last().unwrap().year, None);
}

#[test]
fn filters_narrow_the_results() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(
        root,
        "Pop",
        "Alpha",
        "One",
        "#LANGUAGE:English\n#YEAR:1985\n",
    );
    write_song(
        root,
        "Rock",
        "Beta",
        "Two",
        "#LANGUAGE:German\n#YEAR:1994\n",
    );
    let database = library(root);

    let mut query = SearchQuery::all();
    query.filters.languages = vec!["German".to_owned()];
    assert_eq!(database.search(&query).unwrap().len(), 1);

    let mut query = SearchQuery::all();
    query.filters.decades = vec![1980];
    let hits = database.search(&query).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].year, Some(1985));

    let mut query = SearchQuery::all();
    query.filters.folders = vec!["Rock".to_owned()];
    assert_eq!(database.search(&query).unwrap().len(), 1);
}

#[test]
fn facets_report_what_is_available() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "Alpha", "One", "#LANGUAGE:English\n");
    write_song(root, "Pop", "Beta", "Two", "#LANGUAGE:English\n");
    write_song(root, "Rock", "Gamma", "Three", "#LANGUAGE:German\n");
    let database = library(root);

    let languages = database.facet("language").unwrap();
    assert_eq!(
        languages,
        vec![("English".to_owned(), 2), ("German".to_owned(), 1)]
    );
    // An unknown column is refused rather than interpolated into SQL.
    assert!(database.facet("id; DROP TABLE song").unwrap().is_empty());
}

#[test]
fn decades_are_a_facet_even_though_no_column_holds_one() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(
        root,
        "Pop",
        "Alpha",
        "One",
        "#YEAR:1984
",
    );
    write_song(
        root,
        "Pop",
        "Beta",
        "Two",
        "#YEAR:1989
",
    );
    write_song(
        root,
        "Pop",
        "Gamma",
        "Three",
        "#YEAR:1972
",
    );
    write_song(root, "Pop", "Delta", "Four", "");
    let database = library(root);

    // Newest first, because a decade list is a timeline; ordering it by popularity, as the
    // other facets are, would make it unreadable. A song with no year is in no decade.
    let decades = database.facet("decade").unwrap();
    assert_eq!(
        decades,
        vec![("1980".to_owned(), 2), ("1970".to_owned(), 1)]
    );

    // And the count is the number filtering by it actually returns.
    let mut query = SearchQuery::all();
    query.filters.decades = vec![1980];
    assert_eq!(database.search(&query).unwrap().len(), 2);
}

#[test]
fn play_counts_survive_a_rescan() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "Alpha", "One", "");

    let mut database = Database::in_memory().unwrap();
    scan(&mut database, &ScanOptions::new([root.to_path_buf()])).unwrap();

    let id = database.search(&SearchQuery::all()).unwrap()[0].id;
    database.record_play(id).unwrap();
    database.record_play(id).unwrap();

    // Force a re-read, which must not clobber the player's history.
    let mut forced = ScanOptions::new([root.to_path_buf()]);
    forced.verify = true;
    scan(&mut database, &forced).unwrap();

    assert_eq!(database.song(id).unwrap().unwrap().times_played, 2);
}

#[test]
fn a_broken_song_is_reported_and_left_out() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Pop", "Alpha", "One", "");
    let broken = root.join("Pop").join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("broken.txt"), "this is not a song file at all").unwrap();

    let mut database = Database::in_memory().unwrap();
    let report = scan(&mut database, &ScanOptions::new([root.to_path_buf()])).unwrap();
    assert_eq!(report.added, 1);
    assert_eq!(report.failed, 1);
    assert_eq!(database.count().unwrap(), 1);
}

#[test]
fn metadata_and_media_presence_are_recorded() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(
        root,
        "Pop",
        "Alpha",
        "One",
        "#GENRE:Rock\n#EDITION:Party\n#CREATOR:someone\n",
    );
    let database = library(root);

    let song = &database.search(&SearchQuery::all()).unwrap()[0];
    assert_eq!(song.genre.as_deref(), Some("Rock"));
    assert_eq!(song.edition.as_deref(), Some("Party"));
    assert!(song.audio_file.is_some(), "the mp3 beside it exists");
    assert!(song.video_file.is_none(), "no video was written");
    assert!(song.is_playable());
    assert_eq!(song.note_count, 4);
    assert!(song.duration_secs > 0.0);
    assert!((0.0..=1.0).contains(&song.difficulty));
}

#[test]
fn an_edited_song_is_re_read() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let path = write_song(root, "Pop", "Alpha", "One", "");

    let mut database = Database::in_memory().unwrap();
    let options = ScanOptions::new([root.to_path_buf()]);
    scan(&mut database, &options).unwrap();

    // Rewrite with a different title and a different length, so size changes too.
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("#TITLE:One", "#TITLE:One Renamed");
    fs::write(&path, text).unwrap();

    let report = scan(&mut database, &options).unwrap();
    assert_eq!(report.updated, 1);
    assert_eq!(
        database.search(&SearchQuery::all()).unwrap()[0].title,
        "One Renamed"
    );
}
