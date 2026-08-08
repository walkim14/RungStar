//! Lyric search ranking.
//!
//! "Find the song that goes like this" is the feature nothing upstream has, and it only works
//! if the song you half-remember comes back *first*. Matching is the easy part — every song
//! that happens to contain five common words matches. Ordering them is the whole feature.

use std::fs;
use std::path::Path;

use rungstar_library::{scan, Database, ScanOptions, SearchField, SearchQuery, SortKey};

/// Write a song whose lyrics are the given lines, split into syllables the way real files are.
///
/// The syllable split matters: UltraStar stores "never" as `Ne` and `ver` on separate notes,
/// so a search only works because the indexer rejoins them. Writing the fixtures already
/// joined would test nothing.
fn write_song(root: &Path, artist: &str, title: &str, lines: &[&str]) {
    let dir = root.join(format!("{artist} - {title}"));
    fs::create_dir_all(&dir).expect("create song dir");
    fs::write(dir.join("song.mp3"), b"not really audio").expect("write audio");

    let mut text = format!("#TITLE:{title}\n#ARTIST:{artist}\n#MP3:song.mp3\n#BPM:300\n#GAP:0\n");
    let mut beat = 0;
    for line in lines {
        for word in line.split_whitespace() {
            // Split each word in two, as a real transcription does.
            let middle = word.len().div_ceil(2);
            let (head, tail) = word.split_at(middle);
            text.push_str(&format!(": {beat} 2 0 {head}\n"));
            beat += 2;
            if !tail.is_empty() {
                text.push_str(&format!(": {beat} 2 0 {tail}\n"));
                beat += 2;
            }
            // The trailing space belongs to the last syllable of the word.
            text.push_str(&format!(": {beat} 1 0 {}\n", " "));
            beat += 1;
        }
        text.push_str(&format!("- {beat}\n"));
        beat += 2;
    }
    text.push_str("E\n");
    fs::write(dir.join("song.txt"), text).expect("write song");
}

fn indexed(root: &Path) -> Database {
    let mut database = Database::in_memory().expect("open index");
    scan(&mut database, &ScanOptions::new([root.to_path_buf()])).expect("scan");
    database
}

fn titles(database: &Database, query: SearchQuery) -> Vec<String> {
    database
        .search(&query)
        .expect("search")
        .into_iter()
        .map(|s| s.title)
        .collect()
}

#[test]
fn syllables_are_rejoined_so_a_split_word_is_still_searchable() {
    let temp = tempfile::tempdir().unwrap();
    write_song(temp.path(), "Rick", "Give", &["never gonna give you up"]);
    let database = indexed(temp.path());

    // The file contains `ne` and `ver` on separate notes; the index must contain "never".
    let hits = titles(
        &database,
        SearchQuery::all().text("never").field(SearchField::Lyrics),
    );
    assert_eq!(hits, vec!["Give"], "a split word was not rejoined");
}

#[test]
fn a_phrase_outranks_the_same_words_scattered() {
    // This is the defect the phrase pass exists for: bm25 scores how many query terms a
    // document contains and how short it is, and has no notion of them being adjacent. Both
    // songs below contain all five words; only one of them is the song you meant.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(
        root,
        "Alpha",
        "Scattered",
        &[
            "up on the hill you wait",
            "gonna wander never far",
            "give me one more",
        ],
    );
    write_song(root, "Zulu", "Phrase", &["never gonna give you up"]);
    let database = indexed(root);

    let ranked = titles(
        &database,
        SearchQuery::all()
            .text("never gonna give you up")
            .field(SearchField::Lyrics)
            .sort(SortKey::Relevance, false),
    );
    assert_eq!(ranked.len(), 2, "both songs contain all five words");
    assert_eq!(
        ranked[0], "Phrase",
        "the song containing the actual line came second: {ranked:?}"
    );
}

#[test]
fn an_explicit_sort_is_respected_even_when_searching() {
    // Reordering a list the player explicitly asked to be alphabetical would be wrong,
    // however relevant the ranking thinks something is.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Alpha", "Scattered", &["up you never gonna give"]);
    write_song(root, "Zulu", "Phrase", &["never gonna give you up"]);
    let database = indexed(root);

    let alphabetical = titles(
        &database,
        SearchQuery::all()
            .text("never gonna give you up")
            .field(SearchField::Lyrics)
            .sort(SortKey::Title, false),
    );
    assert_eq!(alphabetical, vec!["Phrase", "Scattered"]);

    let reversed = titles(
        &database,
        SearchQuery::all()
            .text("never gonna give you up")
            .field(SearchField::Lyrics)
            .sort(SortKey::Title, true),
    );
    assert_eq!(reversed, vec!["Scattered", "Phrase"]);
}

#[test]
fn a_single_word_search_is_unaffected_by_the_phrase_pass() {
    // One word cannot be a phrase, so the second query must not run and must not change the
    // result set.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Alpha", "One", &["hello world"]);
    write_song(root, "Beta", "Two", &["hello there"]);
    let database = indexed(root);

    let mut hits = titles(
        &database,
        SearchQuery::all()
            .text("hello")
            .field(SearchField::Lyrics)
            .sort(SortKey::Relevance, false),
    );
    hits.sort();
    assert_eq!(hits, vec!["One", "Two"]);
}

#[test]
fn a_phrase_that_matches_nothing_still_returns_the_word_matches() {
    // The phrase pass narrows the ranking, never the result set. A line remembered slightly
    // wrong must still find the songs containing the words.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Alpha", "One", &["the quick brown fox jumps"]);
    let database = indexed(root);

    let hits = titles(
        &database,
        SearchQuery::all()
            .text("quick fox")
            .field(SearchField::Lyrics)
            .sort(SortKey::Relevance, false),
    );
    assert_eq!(
        hits,
        vec!["One"],
        "words present but not adjacent were dropped"
    );
}

#[test]
fn the_phrase_still_narrows_while_the_last_word_is_being_typed() {
    // Search runs on every keystroke, so the phrase has to match a partial final word or
    // ranking would flap between ranked and unranked as you type.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Alpha", "Scattered", &["you up and never gonna go"]);
    write_song(root, "Zulu", "Phrase", &["never gonna give you up"]);
    let database = indexed(root);

    for partial in ["never gon", "never gonna gi", "never gonna give y"] {
        let ranked = titles(
            &database,
            SearchQuery::all()
                .text(partial)
                .field(SearchField::Lyrics)
                .sort(SortKey::Relevance, false),
        );
        assert_eq!(
            ranked.first().map(String::as_str),
            Some("Phrase"),
            "typing `{partial}` ranked {ranked:?}"
        );
    }
}
