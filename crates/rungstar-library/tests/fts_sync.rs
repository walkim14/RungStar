//! The search index must not drift from the song table.
//!
//! A scan skips the FTS delete for rows it is inserting for the first time, because on a cold
//! scan every row is new and interleaving a delete after each insert forces FTS5 to flush its
//! pending-terms buffer thirty thousand times — the difference between a four second scan and
//! an eighty second one. That optimisation is only correct while "new" really means "this path
//! has no index entry", so these tests pin the two ways it could stop being true: an edited
//! song leaving its old text behind, and a deleted-then-recreated path inheriting a stale row.

use std::fs;
use std::path::{Path, PathBuf};

use rungstar_library::{scan, Database, ScanOptions, SearchField, SearchQuery};

fn write_song(root: &Path, artist: &str, title: &str, lyric: &str) -> PathBuf {
    let dir = root.join(format!("{artist} - {title}"));
    fs::create_dir_all(&dir).expect("create song dir");
    let audio = format!("{artist} - {title}.mp3");
    fs::write(dir.join(&audio), b"not really audio").expect("write audio");

    let path = dir.join(format!("{artist} - {title}.txt"));
    let text = format!(
        "#TITLE:{title}\n#ARTIST:{artist}\n#MP3:{audio}\n#BPM:300\n#GAP:0\n\
         : 0 4 0 {lyric}\n- 12\n: 14 4 5 world\nE\n"
    );
    fs::write(&path, text).expect("write song");
    path
}

fn hits(database: &Database, text: &str, field: SearchField) -> usize {
    database
        .search(&SearchQuery::all().text(text).field(field))
        .expect("search")
        .len()
}

#[test]
fn editing_a_song_replaces_its_index_entry_rather_than_adding_one() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let path = write_song(root, "Alpha", "One", "zaphod");

    let mut database = Database::in_memory().unwrap();
    let options = ScanOptions::new([root.to_path_buf()]);
    scan(&mut database, &options).unwrap();
    assert_eq!(hits(&database, "zaphod", SearchField::Lyrics), 1);

    // Same path, different content. The row is stale, so the index entry must be cleared.
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("zaphod", "trillian"),
    )
    .unwrap();
    let report = scan(&mut database, &options).unwrap();
    assert_eq!(report.updated, 1);

    // The new text is findable...
    assert_eq!(hits(&database, "trillian", SearchField::Lyrics), 1);
    // ...and, the part the delete is responsible for, the old text is not.
    assert_eq!(hits(&database, "zaphod", SearchField::Lyrics), 0);
    assert_eq!(database.count().unwrap(), 1);
}

#[test]
fn a_recreated_path_does_not_inherit_the_old_index_entry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let path = write_song(root, "Alpha", "One", "zaphod");

    let mut database = Database::in_memory().unwrap();
    let options = ScanOptions::new([root.to_path_buf()]);
    scan(&mut database, &options).unwrap();

    // Delete the song and scan, so the row goes away and its id becomes free to reuse.
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
    scan(&mut database, &options).unwrap();
    assert_eq!(database.count().unwrap(), 0);
    assert_eq!(hits(&database, "zaphod", SearchField::Lyrics), 0);

    // Write a different song. It is genuinely new, so its insert skips the delete — which is
    // only safe because removing the old row took its index entry with it.
    write_song(root, "Alpha", "One", "trillian");
    let report = scan(&mut database, &options).unwrap();
    assert_eq!(report.added, 1);
    assert_eq!(hits(&database, "trillian", SearchField::Lyrics), 1);
    assert_eq!(hits(&database, "zaphod", SearchField::Lyrics), 0);
}

#[test]
fn a_verifying_rescan_leaves_exactly_one_entry_per_song() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_song(root, "Alpha", "One", "zaphod");
    write_song(root, "Beta", "Two", "ford");

    let mut database = Database::in_memory().unwrap();
    let options = ScanOptions::new([root.to_path_buf()]);
    scan(&mut database, &options).unwrap();

    // `verify` re-reads everything, so every row takes the update path three times over. If
    // the delete were skipped there, each pass would add another copy of the same song.
    for _ in 0..3 {
        let mut verifying = ScanOptions::new([root.to_path_buf()]);
        verifying.verify = true;
        scan(&mut database, &verifying).unwrap();
    }

    assert_eq!(database.count().unwrap(), 2);
    assert_eq!(hits(&database, "zaphod", SearchField::Lyrics), 1);
    assert_eq!(hits(&database, "ford", SearchField::Lyrics), 1);
    assert_eq!(hits(&database, "Alpha", SearchField::Artist), 1);
}
