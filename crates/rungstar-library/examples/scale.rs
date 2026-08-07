//! Full-scale scan and search timings on a thirty thousand song library.
//!
//! Run once rather than sampled, because building the library dominates everything else:
//! `cargo run --release --example scale -p rungstar-library`
//!
//! Thirty thousand is roughly the size of USDB's catalogue, and people do download most of
//! it. The numbers that matter are the cold scan (paid once), the warm rescan (paid at every
//! launch) and search latency (paid at every keystroke).

use std::path::Path;
use std::time::Instant;

use rungstar_library::{scan, Database, ScanOptions, SearchQuery, SortKey};

const LIBRARY_SIZE: usize = 30_000;

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

fn build_library(root: &Path) {
    for index in 0..LIBRARY_SIZE {
        let artist = ARTISTS[index % ARTISTS.len()];
        let title = format!(
            "{} {} {index}",
            WORDS[index % WORDS.len()],
            WORDS[(index / 7) % WORDS.len()]
        );
        let dir = root
            .join(format!("Pack {:02}", index % 40))
            .join(format!("{artist} - {title}"));
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

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = f();
    println!(
        "{label:<28} {:>9.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    value
}

fn main() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path().to_path_buf();

    println!("building {LIBRARY_SIZE} songs on disk (this is the slow part)...");
    let started = Instant::now();
    build_library(&root);
    println!("built in {:.1} s\n", started.elapsed().as_secs_f64());

    let options = ScanOptions::new([root.clone()]);
    let mut database = Database::in_memory().expect("index");

    time("cold scan", || scan(&mut database, &options).expect("scan"));
    assert_eq!(database.count().expect("count"), LIBRARY_SIZE as i64);
    time("warm rescan (unchanged)", || {
        scan(&mut database, &options).expect("scan")
    });

    let mut verify = options.clone();
    verify.verify = true;
    time("full re-read (verify)", || {
        scan(&mut database, &verify).expect("scan")
    });

    println!();
    let cases: [(&str, SearchQuery); 5] = [
        ("search: prefix", SearchQuery::all().text("bea").limit(200)),
        (
            "search: two words",
            SearchQuery::all().text("queen love").limit(200),
        ),
        (
            "search: lyrics",
            SearchQuery::all().text("shadow winter").limit(200),
        ),
        (
            "search: fuzzy fallback",
            SearchQuery::all().text("beatls").limit(200),
        ),
        (
            "browse: by artist",
            SearchQuery::all().sort(SortKey::Artist, false).limit(200),
        ),
    ];
    for (label, query) in cases {
        let hits = time(label, || database.search(&query).expect("search"));
        println!("{:<28} {:>9} hits", "", hits.len());
    }
}
