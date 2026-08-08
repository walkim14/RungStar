//! Scan a real song folder and report what happened.
//!
//! `cargo run --release --example index -p rungstar-library -- <folder> [--verify]`
//!
//! The synthetic 30,000 song benchmark measures throughput on files this crate generated,
//! which by construction are all well formed. A real library is the only way to find out what
//! the parser actually meets, so this reports the failures as loudly as the timings.

use std::path::PathBuf;
use std::time::Instant;

use rungstar_library::{scan, Database, ScanOptions, SearchField, SearchQuery, SortKey};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next().map(PathBuf::from) else {
        eprintln!("usage: index <song folder> [--verify]");
        std::process::exit(2);
    };
    let verify = std::env::args().any(|a| a == "--verify");

    let database_path = std::env::temp_dir().join("rungstar-index-example.db");
    let _ = std::fs::remove_file(&database_path);
    let mut database = Database::open(&database_path).expect("open index");

    let mut options = ScanOptions::new([root.clone()]);
    options.verify = verify;

    println!("scanning {}", root.display());
    let started = Instant::now();
    let report = scan(&mut database, &options).expect("scan");
    let cold = started.elapsed();

    println!();
    println!("indexed          {}", report.total_indexed());
    println!("  added          {}", report.added);
    println!("  updated        {}", report.updated);
    println!("  unchanged      {}", report.unchanged);
    println!("  failed         {}", report.failed);
    println!("  removed        {}", report.removed);
    println!("cold scan        {:.2} s", cold.as_secs_f64());
    if report.total_indexed() > 0 {
        println!(
            "  per song       {:.2} ms",
            cold.as_secs_f64() * 1000.0 / report.total_indexed() as f64
        );
    }

    let started = Instant::now();
    let second = scan(&mut database, &options).expect("rescan");
    println!(
        "warm rescan      {:.2} s ({} unchanged)",
        started.elapsed().as_secs_f64(),
        second.unchanged
    );

    let time = |label: &str, query: SearchQuery| {
        let started = Instant::now();
        let hits = database.search(&query).expect("search");
        println!(
            "{label:<17}{:>7.1} ms   {} hits",
            started.elapsed().as_secs_f64() * 1000.0,
            hits.len()
        );
        hits
    };

    println!();
    time("browse all", SearchQuery::all().limit(200));
    time("prefix `bea`", SearchQuery::all().text("bea").limit(200));
    time("two words", SearchQuery::all().text("love you").limit(200));
    let lyrics = time(
        "lyric, plain rank",
        SearchQuery::all()
            .text("never gonna give you up")
            .field(SearchField::Lyrics)
            .sort(SortKey::Relevance, false)
            .limit(20),
    );
    for song in lyrics.iter().take(3) {
        println!("                  \u{2192} {}", song.display_name());
    }
    let lyrics = time(
        "lyric, phrase",
        SearchQuery::all()
            .text("never gonna give you up")
            .field(SearchField::Lyrics)
            .sort(SortKey::Relevance, false)
            .limit(20),
    );
    for song in lyrics.iter().take(3) {
        println!("                  \u{2192} {}", song.display_name());
    }
    let fuzzy = time(
        "misspelling",
        SearchQuery::all().text("beatls yestrday").limit(20),
    );
    for song in fuzzy.iter().take(3) {
        println!("                  \u{2192} {}", song.display_name());
    }
    time(
        "by difficulty",
        SearchQuery::all()
            .sort(SortKey::Difficulty, true)
            .limit(200),
    );

    // A song with no audio file cannot be sung or previewed, and the failure is silent —
    // worth counting, because the header naming a file that is not there is the single most
    // common defect in a real library.
    let all = database
        .search(&SearchQuery::all().limit(100_000))
        .expect("search");
    let playable = all.iter().filter(|s| s.is_playable()).count();
    let with_video = all.iter().filter(|s| s.video_file.is_some()).count();
    let with_cover = all.iter().filter(|s| s.cover_file.is_some()).count();
    println!();
    println!("playable   {playable} of {}", all.len());
    println!("has video  {with_video}");
    println!("has cover  {with_cover}");
    println!("duets      {}", all.iter().filter(|s| s.is_duet).count());
    for song in all.iter().filter(|s| !s.is_playable()).take(3) {
        println!("  no audio: {}", song.path.display());
    }
    for song in all.iter().filter(|s| s.is_duet).take(3) {
        println!("  duet: {}", song.path.display());
    }

    println!();
    for column in ["language", "genre", "edition"] {
        match database.facet(column) {
            Ok(values) if !values.is_empty() => {
                let shown: Vec<String> = values
                    .iter()
                    .take(6)
                    .map(|(name, count)| format!("{name} ({count})"))
                    .collect();
                println!(
                    "{column:<10}{} distinct: {}",
                    values.len(),
                    shown.join(", ")
                );
            }
            _ => println!("{column:<10}none recorded"),
        }
    }

    let _ = std::fs::remove_file(&database_path);
}
