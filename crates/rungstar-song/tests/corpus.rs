//! Behavioural conformance against the reference corpus.
//!
//! The fixtures come from usdb_syncer's test suite (GPL-3.0-only, see `NOTICE.md`) and are
//! used unmodified. They encode two decades of accumulated real-world weirdness, so matching
//! them byte for byte is the strongest compatibility signal available without a song library.
//!
//! * `normalized/` — already canonical: parsing then writing must be the identity.
//! * `deviant/`    — `*_in.txt` parsed and written must equal `*_out.txt`, with no fixes.
//! * `invalid/`    — must fail to parse.

use std::fs;
use std::path::{Path, PathBuf};

use rungstar_song::SongTxt;

fn fixtures(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(sub)
}

/// Read a fixture, collapsing line endings the way Python's universal-newline reader does.
///
/// Two fixtures are deliberately stored with CRLF (see their `.gitattributes`) to exercise
/// the reader; the expected output is still LF.
fn read_normalized(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    String::from_utf8(bytes)
        .unwrap_or_else(|e| panic!("{} is not UTF-8: {e}", path.display()))
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn txt_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("listing {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures found in {}", dir.display());
    paths
}

#[test]
fn normalized_files_round_trip_unchanged() {
    for path in txt_files(&fixtures("normalized")) {
        let contents = read_normalized(&path);
        let song = SongTxt::parse_str(&contents)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
        assert_eq!(
            song.to_string(),
            contents,
            "round trip changed {}",
            path.display()
        );
    }
}

#[test]
fn deviant_files_normalise_to_their_expected_output() {
    for path in txt_files(&fixtures("deviant")) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix("_in.txt") else {
            continue;
        };
        let expected_path = path.with_file_name(format!("{stem}_out.txt"));

        let input = read_normalized(&path);
        let expected = read_normalized(&expected_path);
        let song = SongTxt::parse_str(&input)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
        assert_eq!(
            song.to_string(),
            expected,
            "mismatch for {}",
            path.display()
        );
    }
}

#[test]
fn invalid_files_are_rejected() {
    for path in txt_files(&fixtures("invalid")) {
        let contents = read_normalized(&path);
        assert!(
            SongTxt::parse_str(&contents).is_err(),
            "{} parsed but should have been rejected",
            path.display()
        );
    }
}

#[test]
fn duet_tracks_are_separated() {
    let contents = read_normalized(&fixtures("normalized").join("duet.txt"));
    let song = SongTxt::parse_str(&contents).unwrap();
    assert!(song.is_duet());
    let track_2 = song.tracks.track_2.as_ref().expect("second track");
    // P1 sings three lines, P2 two; the split point is the `P2` marker.
    assert_eq!(song.tracks.track_1.len(), 3);
    assert_eq!(track_2.len(), 2);
    assert_eq!(song.tracks.track_1[0].notes.len(), 2);
    assert_eq!(track_2[0].start(), 4);
    assert_eq!(song.headers.p1.as_deref(), Some("P1"));
    assert_eq!(song.headers.p2.as_deref(), Some("P2"));
}

#[test]
fn junk_is_skipped_but_reported() {
    let contents = read_normalized(&fixtures("deviant").join("with_junk_in.txt"));
    let parsed = SongTxt::parse_str_verbose(&contents).unwrap();
    assert!(
        parsed.warnings.len() >= 5,
        "expected the junk lines to be reported, got {:?}",
        parsed.warnings.as_slice()
    );
}

#[test]
fn fix_files_match_their_expected_output() {
    // The reference test suite pins exactly these options, so this compares like with like.
    let options = rungstar_song::FixOptions::usdx_style();
    let mut failures = Vec::new();

    for path in txt_files(&fixtures("fixes")) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix("_in.txt") else {
            continue;
        };
        let expected_path = path.with_file_name(format!("{stem}_out.txt"));

        let input = read_normalized(&path);
        let expected = read_normalized(&expected_path);
        let mut song = SongTxt::parse_str(&input)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
        let mut warnings = rungstar_song::Warnings::new();
        song.fix(&options, &mut warnings);

        let actual = song.to_string();
        if actual != expected {
            failures.push(format!(
                "--- {stem} ---\nexpected:\n{expected}\n\nactual:\n{actual}\n"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} fixture(s) differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
