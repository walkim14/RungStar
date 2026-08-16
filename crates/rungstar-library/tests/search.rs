//! Narrowing a search by difficulty band.
//!
//! The heuristic difficulty is one number, but nobody browses by a number — the filter tree
//! offers bands, and ticking two of them has to mean "either", the way every other category
//! in it does. The bands are the five words the detail panel has always shown for a song, at
//! the same cut points, because a panel calling a song Moderate while the filter files it
//! under Hard is one number with two names. Songs are written straight into the index rather
//! than scanned, so the bands are pinned by the filter alone and not by whatever the note
//! heuristic happens to score a fixture at.

use rungstar_library::scan::ParsedSong;
use rungstar_library::{Database, DifficultyBand, Filters, SearchQuery};

/// A difficulty a given fraction of the way through a band.
///
/// These tests are about the filter, not about where the cut points sit — those are pinned
/// once, on their own, at the bottom of this file. Taking the fixture from the band itself
/// means a recut moves them with it rather than breaking three tests that were never about
/// the numbers.
fn inside(band: DifficultyBand, fraction: f64) -> f64 {
    let (low, high) = band.range();
    let low = if low.is_finite() { low } else { 0.0 };
    let high = if high.is_finite() { high } else { 1.0 };
    low + (high - low) * fraction
}

/// A song that exists only to sit at a known difficulty.
fn song(artist: &str, difficulty: f64) -> ParsedSong {
    ParsedSong {
        path: format!("C:/songs/{artist}/{artist}.txt"),
        artist: artist.to_owned(),
        title: "Song".to_owned(),
        // The sort keys the scanner would have filled, so the result order is the artist
        // order rather than whatever the rows were written in.
        artist_sort: artist.to_lowercase(),
        title_sort: "song".to_owned(),
        difficulty,
        ..Default::default()
    }
}

#[test]
fn two_difficulty_bands_return_the_songs_in_either_and_nothing_outside_both() {
    let mut database = Database::in_memory().expect("open index");
    database
        .upsert_songs(&[
            song("Gentle", inside(DifficultyBand::Gentle, 0.5)),
            song("Easy", inside(DifficultyBand::Easy, 0.5)),
            song("Hard", inside(DifficultyBand::Hard, 0.5)),
        ])
        .expect("write songs");

    let filters = Filters {
        difficulty: vec![DifficultyBand::Gentle, DifficultyBand::Hard],
        ..Default::default()
    };
    let hits = database
        .search(&SearchQuery::all().filters(filters))
        .expect("search");

    let artists: Vec<&str> = hits.iter().map(|song| song.artist.as_str()).collect();
    assert_eq!(artists, vec!["Gentle", "Hard"]);
}

#[test]
fn the_difficulty_facet_counts_each_band_easiest_first() {
    // Ordered by the scale and never by popularity: Easy/Hard/Medium reads as a mistake, the
    // same reason the decade list is a timeline rather than a chart.
    let mut database = Database::in_memory().expect("open index");
    database
        .upsert_songs(&[
            song("A", inside(DifficultyBand::Gentle, 0.5)),
            song("B", inside(DifficultyBand::Easy, 0.3)),
            song("C", inside(DifficultyBand::Easy, 0.7)),
            song("D", inside(DifficultyBand::Moderate, 0.5)),
            song("E", inside(DifficultyBand::Hard, 0.5)),
            song("F", inside(DifficultyBand::Brutal, 0.5)),
        ])
        .expect("write songs");

    let bands = database.facet("difficulty").expect("list bands");
    assert_eq!(
        bands,
        vec![
            ("gentle".to_owned(), 1),
            ("easy".to_owned(), 2),
            ("moderate".to_owned(), 1),
            ("hard".to_owned(), 1),
            ("brutal".to_owned(), 1),
        ]
    );
}

#[test]
fn a_band_no_song_is_in_is_not_offered() {
    // Ticking a filter that can only return nothing is a dead end, and every other category
    // in the tree lists what the library has rather than what it could have. This is what
    // keeps the top of the scale honest: measured across a real library only one song in
    // 8,159 is Brutal, so that row is simply absent from almost every library there is.
    let mut database = Database::in_memory().expect("open index");
    database
        .upsert_songs(&[
            song("A", inside(DifficultyBand::Gentle, 0.5)),
            song("B", inside(DifficultyBand::Easy, 0.5)),
        ])
        .expect("write songs");

    let bands: Vec<String> = database
        .facet("difficulty")
        .expect("list bands")
        .into_iter()
        .map(|(band, _)| band)
        .collect();
    assert_eq!(bands, vec!["gentle", "easy"]);
}

#[test]
fn the_words_divide_the_library_rather_than_the_scale() {
    // Cut at fifths of the scale — 0.2/0.4/0.6/0.8 — the words look reasonable and describe
    // nothing. Measured across a real 8,159-song library the heuristic runs 0.00 to 0.82 with
    // a mean of 0.31, so that mean lands in Easy, two thirds of every library shares that one
    // word, and Brutal is a single song. A band nearly everything is in is not a filter.
    //
    // These three points are where the library actually sits: its mean, its ninetieth
    // percentile, and its hardest songs.
    assert_eq!(
        DifficultyBand::of(0.31).label(),
        "Moderate",
        "the average song"
    );
    assert_eq!(
        DifficultyBand::of(0.45).label(),
        "Hard",
        "the ninetieth percentile"
    );
    assert_eq!(
        DifficultyBand::of(0.55).label(),
        "Brutal",
        "the hardest songs there are"
    );
}
