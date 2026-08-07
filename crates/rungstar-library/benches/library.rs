//! Scan and search throughput on a library the size of a serious collection.
//!
//! This measures search latency, which is what every keystroke in the browser pays. It uses a
//! modest library because criterion re-runs each case many times and building song files
//! dominates otherwise. For full-scale scan and search figures on a thirty thousand song
//! collection, run .

use std::path::{Path, PathBuf};

use criterion::{criterion_group, criterion_main, Criterion};
use rungstar_library::{scan, Database, ScanOptions, SearchQuery, SortKey};

const LIBRARY_SIZE: usize = 4_000;

const ARTISTS: [&str; 16] = [
    "The Beatles",
    "Queen",
    "Björk",
    "ABBA",
    "Nirvana",
    "Die Ärzte",
    "Daft Punk",
    "Radiohead",
    "Beyoncé",
    "Metallica",
    "Céline Dion",
    "Blur",
    "Oasis",
    "Prince",
    "Madonna",
    "Muse",
];
const WORDS: [&str; 12] = [
    "love", "night", "fire", "heart", "dream", "shadow", "river", "gold", "winter", "sun", "road",
    "silence",
];

/// Build a synthetic library on disk. Slow, so it happens once and is reused.
fn build_library(root: &Path) {
    for index in 0..LIBRARY_SIZE {
        let artist = ARTISTS[index % ARTISTS.len()];
        let title = format!(
            "{} {} {index}",
            WORDS[index % WORDS.len()],
            WORDS[(index / 7) % WORDS.len()]
        );
        let folder = format!("Pack {:02}", index % 40);
        let dir = root.join(&folder).join(format!("{artist} - {title}"));
        std::fs::create_dir_all(&dir).expect("create song dir");

        let audio = format!("{artist} - {title}.mp3");
        std::fs::write(dir.join(&audio), b"x").expect("write audio");

        let mut text = format!(
            "#TITLE:{title}\n#ARTIST:{artist}\n#MP3:{audio}\n#BPM:{}\n#GAP:{}\n\
             #LANGUAGE:{}\n#GENRE:{}\n#YEAR:{}\n",
            240 + (index % 200),
            index % 5_000,
            if index % 3 == 0 { "English" } else { "German" },
            if index % 2 == 0 { "Pop" } else { "Rock" },
            1960 + (index % 65),
        );
        // Forty notes is a short song, but it exercises the parser and the lyric index.
        for note in 0..40 {
            let beat = note * 6;
            let word = WORDS[(index + note as usize) % WORDS.len()];
            text.push_str(&format!(": {beat} 4 {} {word} \n", note % 13));
            if note % 8 == 7 {
                text.push_str(&format!("- {}\n", beat + 5));
            }
        }
        text.push_str("E\n");
        std::fs::write(dir.join(format!("{artist} - {title}.txt")), text).expect("write song");
    }
}

fn scanned(root: &Path) -> Database {
    let mut database = Database::in_memory().expect("index");
    scan(&mut database, &ScanOptions::new([root.to_path_buf()])).expect("scan");
    database
}

fn benchmarks(c: &mut Criterion) {
    let temp = tempfile::tempdir().expect("temp dir");
    let root: PathBuf = temp.path().to_path_buf();
    eprintln!("building a {LIBRARY_SIZE} song library, this takes a moment...");
    build_library(&root);

    let database = scanned(&root);
    assert_eq!(database.count().expect("count"), LIBRARY_SIZE as i64);

    let mut search_group = c.benchmark_group("search");
    search_group.bench_function("prefix", |b| {
        let query = SearchQuery::all().text("bea").limit(200);
        b.iter(|| std::hint::black_box(database.search(&query).expect("search")));
    });
    search_group.bench_function("two_words", |b| {
        let query = SearchQuery::all().text("queen love").limit(200);
        b.iter(|| std::hint::black_box(database.search(&query).expect("search")));
    });
    search_group.bench_function("lyrics", |b| {
        let query = SearchQuery::all().text("shadow winter").limit(200);
        b.iter(|| std::hint::black_box(database.search(&query).expect("search")));
    });
    search_group.bench_function("fuzzy_miss", |b| {
        // No index hit, so this falls through to edit distance over the whole library.
        let query = SearchQuery::all().text("beatls").limit(200);
        b.iter(|| std::hint::black_box(database.search(&query).expect("search")));
    });
    search_group.bench_function("browse_by_artist", |b| {
        let query = SearchQuery::all().sort(SortKey::Artist, false).limit(200);
        b.iter(|| std::hint::black_box(database.search(&query).expect("search")));
    });
    search_group.finish();
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
