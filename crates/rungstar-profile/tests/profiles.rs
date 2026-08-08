//! Players, scores, statistics, and reading somebody's existing UltraStar database.

use rungstar_profile::stats::{Order, View};
use rungstar_profile::{import_ultrastar, song_key, Profiles, Score};

const NOW: i64 = 1_770_000_000;

fn store() -> Profiles {
    Profiles::in_memory().expect("open")
}

fn score(player_id: i64, artist: &str, title: &str, points: i32) -> Score {
    Score {
        player_id,
        artist: artist.to_owned(),
        title: title.to_owned(),
        difficulty: 1,
        points,
        notes: points - 500,
        golden: 200,
        line_bonus: 300,
        sung_at: NOW,
    }
}

#[test]
fn a_player_is_created_once_however_often_they_are_named() {
    // The importer meets the same name over and over, and two people called the same thing is
    // a mistake rather than a feature.
    let mut profiles = store();
    let first = profiles.ensure_player("Walki", NOW).unwrap();
    let again = profiles.ensure_player("Walki", NOW + 10).unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(profiles.players().unwrap().len(), 1);

    // Case and stray spaces are the same person. A separate profile because somebody typed a
    // capital differently is worse than none.
    let spaced = profiles.ensure_player("  walki ", NOW).unwrap();
    assert_eq!(spaced.id, first.id);
    assert_eq!(profiles.players().unwrap().len(), 1);
}

#[test]
fn players_get_different_colours_to_start_with() {
    let mut profiles = store();
    let colours: Vec<u8> = ["A", "B", "C", "D"]
        .iter()
        .map(|n| profiles.ensure_player(n, NOW).unwrap().colour)
        .collect();
    let unique: std::collections::HashSet<u8> = colours.iter().copied().collect();
    assert_eq!(
        unique.len(),
        colours.len(),
        "two players started the same colour"
    );
}

#[test]
fn a_name_cannot_be_taken_twice_by_renaming() {
    let mut profiles = store();
    let one = profiles.ensure_player("One", NOW).unwrap();
    let two = profiles.ensure_player("Two", NOW).unwrap();

    assert!(profiles.rename_player(two.id, "One").is_err());
    // Renaming to your own name, in different case, is not a clash.
    assert!(profiles.rename_player(one.id, "ONE").is_ok());
    assert!(
        profiles.rename_player(one.id, "").is_err(),
        "a name is required"
    );
}

#[test]
fn a_song_is_found_again_after_the_index_is_rebuilt() {
    // Scores are keyed on artist and title rather than on a library row, because song ids are
    // assigned by the scanner and change when the index is rebuilt. Keying on one would orphan
    // every score the first time anybody rebuilt it.
    assert_eq!(
        song_key("Queen", "Bohemian Rhapsody"),
        song_key("queen", "bohemian rhapsody")
    );
    assert_eq!(
        song_key(" Queen ", "Bohemian Rhapsody "),
        song_key("Queen", "Bohemian Rhapsody")
    );
    assert_ne!(
        song_key("Queen", "Radio Ga Ga"),
        song_key("Queen", "Bohemian Rhapsody")
    );

    let mut profiles = store();
    let player = profiles.ensure_player("Walki", NOW).unwrap();
    profiles
        .record(&score(player.id, "Queen", "Bohemian Rhapsody", 8500))
        .unwrap();

    // The same song, spelled as a different scan might have it.
    let found = profiles
        .best_for("queen", "BOHEMIAN RHAPSODY", None, 5)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].points, 8500);
}

#[test]
fn a_songs_table_is_highest_first_and_kept_per_difficulty() {
    // A score on Easy is not comparable with one on Hard, and putting them in one table would
    // make Easy always win.
    let mut profiles = store();
    let a = profiles.ensure_player("A", NOW).unwrap();
    let b = profiles.ensure_player("B", NOW).unwrap();

    for (player, points, difficulty) in [(a.id, 7000, 2u8), (b.id, 9500, 0), (a.id, 8000, 2)] {
        let mut entry = score(player, "Queen", "Radio Ga Ga", points);
        entry.difficulty = difficulty;
        profiles.record(&entry).unwrap();
    }

    let all = profiles.best_for("Queen", "Radio Ga Ga", None, 5).unwrap();
    assert_eq!(all.len(), 3);
    assert!(all[0].points >= all[1].points && all[1].points >= all[2].points);

    let hard = profiles
        .best_for("Queen", "Radio Ga Ga", Some(2), 5)
        .unwrap();
    assert_eq!(hard.len(), 2, "Easy leaked into the Hard table");
    assert_eq!(hard[0].points, 8000);

    // A top five is five, however many were sung.
    for n in 0..10 {
        profiles
            .record(&score(a.id, "Queen", "Radio Ga Ga", 100 * n))
            .unwrap();
    }
    assert_eq!(
        profiles
            .best_for("Queen", "Radio Ga Ga", None, 5)
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn singers_are_ranked_by_average_not_by_total() {
    // Ranking by total ranks by who sang most. One lucky song should not put somebody top of
    // a list they have no business being on either, which is why the average is the key and
    // the count is shown beside it.
    let mut profiles = store();
    let steady = profiles.ensure_player("Steady", NOW).unwrap();
    let lucky = profiles.ensure_player("Lucky", NOW).unwrap();

    for _ in 0..10 {
        profiles
            .record(&score(steady.id, "Queen", "One Vision", 7000))
            .unwrap();
    }
    profiles
        .record(&score(lucky.id, "Queen", "One Vision", 9500))
        .unwrap();

    let ranked = profiles.best_singers(Order::Best, 10).unwrap();
    assert_eq!(ranked[0].name, "Lucky", "the higher average should lead");
    assert_eq!(ranked[0].songs, 1);
    assert_eq!(ranked[1].name, "Steady");
    assert_eq!(ranked[1].songs, 10);
    assert_eq!(ranked[1].average, 7000);

    // Worst first is the same list reversed, which is genuinely funny at a party.
    let worst = profiles.best_singers(Order::Worst, 10).unwrap();
    assert_eq!(worst[0].name, "Steady");
}

#[test]
fn the_same_song_spelled_two_ways_counts_once() {
    let mut profiles = store();
    let player = profiles.ensure_player("A", NOW).unwrap();
    profiles
        .record(&score(player.id, "Queen", "One Vision", 7000))
        .unwrap();
    profiles
        .record(&score(player.id, "queen", "one vision", 7100))
        .unwrap();
    profiles
        .record(&score(player.id, "ABBA", "Waterloo", 6000))
        .unwrap();

    let songs = profiles.most_sung_songs(Order::Best, 10).unwrap();
    assert_eq!(songs.len(), 2, "one song was counted as two: {songs:?}");
    assert_eq!(songs[0].times, 2);
    assert_eq!(songs[0].best, 7100);

    let artists = profiles.most_sung_artists(Order::Best, 10).unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0].artist.to_lowercase(), "queen");
    assert_eq!(artists[0].times, 2);
}

#[test]
fn deleting_a_player_takes_their_scores_with_them() {
    // Which is what deleting a profile means. Statistics are computed from the rows that
    // remain rather than kept as totals, so nothing is left claiming they sang.
    let mut profiles = store();
    let keep = profiles.ensure_player("Keep", NOW).unwrap();
    let go = profiles.ensure_player("Go", NOW).unwrap();
    profiles
        .record(&score(keep.id, "Queen", "One Vision", 7000))
        .unwrap();
    profiles
        .record(&score(go.id, "Queen", "One Vision", 9000))
        .unwrap();
    assert_eq!(profiles.score_count().unwrap(), 2);

    profiles.remove_player(go.id).unwrap();
    assert_eq!(
        profiles.score_count().unwrap(),
        1,
        "their scores stayed behind"
    );
    assert_eq!(
        profiles
            .best_for("Queen", "One Vision", None, 5)
            .unwrap()
            .len(),
        1
    );
    assert!(profiles
        .best_singers(Order::Best, 10)
        .unwrap()
        .iter()
        .all(|s| s.name != "Go"));
}

#[test]
fn a_favourite_is_per_player_and_toggles() {
    let mut profiles = store();
    let a = profiles.ensure_player("A", NOW).unwrap();
    let b = profiles.ensure_player("B", NOW).unwrap();

    assert!(!profiles.is_favourite(a.id, "Queen", "One Vision").unwrap());
    assert!(profiles
        .toggle_favourite(a.id, "Queen", "One Vision")
        .unwrap());
    assert!(profiles.is_favourite(a.id, "Queen", "One Vision").unwrap());
    // Somebody else's taste is their own.
    assert!(!profiles.is_favourite(b.id, "Queen", "One Vision").unwrap());

    assert!(!profiles
        .toggle_favourite(a.id, "Queen", "One Vision")
        .unwrap());
    assert!(!profiles.is_favourite(a.id, "Queen", "One Vision").unwrap());

    profiles.toggle_favourite(a.id, "ABBA", "Waterloo").unwrap();
    profiles.toggle_favourite(b.id, "ABBA", "Waterloo").unwrap();
    assert_eq!(profiles.favourite_keys(Some(a.id)).unwrap().len(), 1);
    assert_eq!(
        profiles.favourite_keys(None).unwrap().len(),
        1,
        "counted twice"
    );
}

#[test]
fn the_statistics_views_name_their_own_columns() {
    // A table of numbers with unlabelled columns is a puzzle.
    for view in View::ALL {
        let (left, right) = view.columns();
        assert!(!view.title().is_empty());
        assert!(!left.is_empty() && !right.is_empty());
    }
    assert_eq!(View::Scores.next().next().next().next(), View::Scores);
    assert_eq!(View::Scores.previous(), View::Artists);
}

/// One row as an `Ultrastar.db` holds it: singer, artist, title, difficulty, score, date.
type UsRow<'a> = (&'a str, &'a str, &'a str, i64, i64, Option<i64>);

/// Write an `Ultrastar.db` the way UltraStar Deluxe does.
fn write_ultrastar(path: &std::path::Path, rows: &[UsRow<'_>]) {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE us_songs (
             ID INTEGER PRIMARY KEY, Artist TEXT NOT NULL, Title TEXT NOT NULL,
             TimesPlayed INTEGER NOT NULL, Rating INTEGER NULL);
         CREATE TABLE us_scores (
             SongID INTEGER NOT NULL, Difficulty INTEGER NOT NULL, Player TEXT NOT NULL,
             Score INTEGER NOT NULL, Date INTEGER NULL);",
    )
    .unwrap();

    let mut songs: Vec<(String, String)> = Vec::new();
    for (_, artist, title, _, _, _) in rows {
        let pair = ((*artist).to_owned(), (*title).to_owned());
        if !songs.contains(&pair) {
            songs.push(pair);
        }
    }
    for (index, (artist, title)) in songs.iter().enumerate() {
        db.execute(
            "INSERT INTO us_songs (ID, Artist, Title, TimesPlayed) VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![index as i64 + 1, artist, title],
        )
        .unwrap();
    }
    for (player, artist, title, difficulty, points, date) in rows {
        let id = songs
            .iter()
            .position(|(a, t)| a == artist && t == title)
            .unwrap() as i64
            + 1;
        db.execute(
            "INSERT INTO us_scores (SongID, Difficulty, Player, Score, Date)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, difficulty, player, points, date],
        )
        .unwrap();
    }
}

#[test]
fn an_existing_ultrastar_database_imports() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("Ultrastar.db");
    write_ultrastar(
        &path,
        &[
            (
                "Walki",
                "Queen",
                "Bohemian Rhapsody",
                1,
                8500,
                Some(1_600_000_000),
            ),
            ("Walki", "ABBA", "Waterloo", 0, 7200, Some(1_600_000_100)),
            ("Anna", "Queen", "Bohemian Rhapsody", 2, 9100, None),
        ],
    );

    let mut profiles = store();
    let report = import_ultrastar(&mut profiles, &path, NOW).unwrap();
    assert_eq!(report.scores_added, 3);
    assert_eq!(report.players_added, 2);
    assert_eq!(report.orphaned, 0);

    // The scores are there, keyed the same way as a score earned here.
    let table = profiles
        .best_for("Queen", "Bohemian Rhapsody", None, 5)
        .unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(table[0].player, "Anna");
    assert_eq!(table[0].points, 9100);
    assert!(report.summary().contains("3 scores"));
}

#[test]
fn importing_twice_does_not_double_anybodys_history() {
    // The obvious way to get this wrong is a fallback date of "now", which makes every row
    // look new on the second run.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("Ultrastar.db");
    write_ultrastar(
        &path,
        &[
            ("Walki", "Queen", "One Vision", 1, 8000, Some(1_600_000_000)),
            ("Walki", "Queen", "One Vision", 1, 8000, None),
        ],
    );

    let mut profiles = store();
    let first = import_ultrastar(&mut profiles, &path, NOW).unwrap();
    assert_eq!(first.scores_added, 2);

    let second = import_ultrastar(&mut profiles, &path, NOW + 5000).unwrap();
    assert_eq!(second.scores_added, 0, "the second import duplicated rows");
    assert_eq!(second.scores_skipped, 2);
    assert_eq!(profiles.score_count().unwrap(), 2);
}

#[test]
fn an_import_keeps_scores_earned_here() {
    // Replacing the table would be simpler and would silently delete somebody's evening.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("Ultrastar.db");
    write_ultrastar(&path, &[("Walki", "Queen", "One Vision", 1, 8000, Some(1))]);

    let mut profiles = store();
    let mine = profiles.ensure_player("Walki", NOW).unwrap();
    profiles
        .record(&score(mine.id, "Queen", "One Vision", 9999))
        .unwrap();

    import_ultrastar(&mut profiles, &path, NOW).unwrap();
    let table = profiles.best_for("Queen", "One Vision", None, 5).unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(table[0].points, 9999, "the score earned here was lost");
}

#[test]
fn a_score_with_no_song_row_is_counted_rather_than_dropped() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("Ultrastar.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE us_songs (ID INTEGER PRIMARY KEY, Artist TEXT, Title TEXT);
         CREATE TABLE us_scores (SongID INTEGER, Difficulty INTEGER, Player TEXT,
                                 Score INTEGER, Date INTEGER);
         INSERT INTO us_scores VALUES (99, 1, 'Walki', 8000, 1);",
    )
    .unwrap();
    drop(db);

    let mut profiles = store();
    let report = import_ultrastar(&mut profiles, &path, NOW).unwrap();
    assert_eq!(report.scores_added, 0);
    assert_eq!(report.orphaned, 1, "a score with no song vanished silently");
    assert!(report.summary().contains("no song"));
}

#[test]
fn a_database_that_is_not_ultrastars_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("something-else.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE notes (id INTEGER);")
        .unwrap();

    let mut profiles = store();
    let error = import_ultrastar(&mut profiles, &path, NOW).unwrap_err();
    assert!(
        error.to_string().contains("us_scores"),
        "the reason should name what was missing: {error}"
    );
}
