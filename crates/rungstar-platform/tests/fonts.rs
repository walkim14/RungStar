//! The bundled faces, and whether they can draw what a song library contains.
//!
//! Measured over 8,134 real songs, the text is 99.94% ASCII — and the remainder is 160,000
//! curly quotes, 28,000 accented letters, 200 Cyrillic characters, a few hundred CJK brackets
//! and a handful of Hangul. No single face worth shipping covers all of that, and the failure
//! mode of missing one is silent: an empty box where a letter should be, which is exactly how
//! the star glyphs in the USDB ratings went unnoticed until somebody looked at the screen.

use rungstar_platform::{Face, FontSet};

fn bundled(name: &str) -> Option<Face> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/fonts")
        .join(name);
    path.exists()
        .then(|| Face::load(&path).expect("a usable font"))
}

/// Everything a real library actually needs, by category.
///
/// Written out rather than read from a song folder, so the test runs on a machine with no
/// songs on it. The characters are the ones the measurement over the library turned up.
const NEEDED: &[(&str, &str)] = &[
    (
        "ASCII",
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
    ),
    ("ASCII punctuation", "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"),
    // 160,908 occurrences: this is what a downloaded song file is full of.
    (
        "typographic punctuation",
        "\u{2018}\u{2019}\u{201C}\u{201D}\u{2013}\u{2014}\u{2026}",
    ),
    // 27,868 occurrences.
    ("accented Latin", "áàâäãåæçéèêëíìîïñóòôöõøßúùûüýÁÄÖÜÉÑÇ"),
    ("Latin extended", "œŒšŠžŽ"),
    ("currency", "€£¥"),
];

/// What the chosen face is allowed to be missing, and the fallback has to carry.
const COVERAGE_ONLY: &[(&str, &str)] = &[
    ("Cyrillic", "абвгдежзийклмнопрстуфхцчшщъыьэюяАБВГДЕЖЗИЙ"),
    ("Greek", "ΛαβγδεΩ"),
];

#[test]
fn the_bundled_faces_are_there_and_load() {
    // These used to be dropped in at packaging time, which meant the game looked different on
    // every machine and nobody saw the shipped one until release.
    for name in [
        "RungStar-Regular.ttf",
        "RungStar-Bold.ttf",
        "RungStar-Lyrics.ttf",
        "RungStar-Fallback.ttf",
    ] {
        assert!(
            bundled(name).is_some(),
            "{name} is missing from assets/fonts"
        );
    }
}

#[test]
fn the_chosen_face_draws_everything_a_song_library_is_made_of() {
    let Some(face) = bundled("RungStar-Regular.ttf") else {
        return;
    };
    for (what, characters) in NEEDED {
        let missing: String = characters.chars().filter(|c| !face.has(*c)).collect();
        assert!(
            missing.is_empty(),
            "the chosen face cannot draw {what}: {missing:?}"
        );
    }
}

#[test]
fn every_weight_covers_the_same_characters() {
    // A heading or a lyric that silently loses its accents while the body text keeps them is
    // worse than one face used throughout.
    let Some(regular) = bundled("RungStar-Regular.ttf") else {
        return;
    };
    for name in ["RungStar-Bold.ttf", "RungStar-Lyrics.ttf"] {
        let other = bundled(name).expect("a bundled weight");
        for (what, characters) in NEEDED {
            for c in characters.chars() {
                assert_eq!(
                    regular.has(c),
                    other.has(c),
                    "{name} disagrees with the regular weight about {what}: {c:?}"
                );
            }
        }
    }
}

#[test]
fn the_fallback_covers_what_the_chosen_face_does_not() {
    // The whole point of having one: the face is chosen for how it looks, and the chain is
    // what makes that safe.
    let (Some(chosen), Some(fallback)) = (
        bundled("RungStar-Regular.ttf"),
        bundled("RungStar-Fallback.ttf"),
    ) else {
        return;
    };
    let mut needed_the_fallback = 0;
    for (what, characters) in COVERAGE_ONLY {
        for c in characters.chars() {
            assert!(
                chosen.has(c) || fallback.has(c),
                "nothing bundled can draw {what}: {c:?}"
            );
            if !chosen.has(c) {
                needed_the_fallback += 1;
            }
        }
    }
    assert!(
        needed_the_fallback > 0,
        "the fallback covered nothing the chosen face lacked, so it is dead weight"
    );
}

#[test]
fn a_chained_face_answers_for_the_whole_chain() {
    let (Some(chosen), Some(fallback)) = (
        bundled("RungStar-Regular.ttf"),
        bundled("RungStar-Fallback.ttf"),
    ) else {
        return;
    };
    let borrowed = 'д';
    assert!(!chosen.has(borrowed), "pick a character it really lacks");

    let chained = chosen.with_fallback(fallback);
    assert!(chained.has(borrowed));
    // And it measures as something rather than as nothing. A missing glyph costs a word that
    // is narrower than the letters in it, which is how a layout silently goes wrong.
    assert!(
        chained.width("д", 32.0) > 1.0,
        "a borrowed glyph has no advance width"
    );
}

#[test]
fn a_font_set_loads_with_no_theme_and_no_bundled_faces() {
    // A build from a source tree with an empty assets/fonts still has to start.
    assert!(FontSet::load(None, None, None).is_ok());
}

#[test]
fn an_invalid_theme_font_falls_back_in_every_role() {
    let temp = tempfile::tempdir().unwrap();
    let invalid = temp.path().join("broken.ttf");
    std::fs::write(&invalid, b"not a font").unwrap();

    let fonts = FontSet::load(Some(&invalid), Some(&invalid), Some(&invalid))
        .expect("an invalid theme face must not stop the game");
    for role in [
        rungstar_ui::draw::Font::Regular,
        rungstar_ui::draw::Font::Bold,
        rungstar_ui::draw::Font::Lyrics,
    ] {
        assert_ne!(fonts.face(role).source(), "broken.ttf");
    }
}

#[test]
fn the_shipped_chain_draws_every_script_a_library_uses() {
    // The set the loader itself probes for. Asserted end to end here — through the real
    // `FontSet::load`, on whatever this machine has — because the loader stopping early is
    // only correct if what it stopped at is actually enough.
    let fonts = FontSet::load(None, None, None).expect("a usable font set");
    let face = fonts.face(rungstar_ui::draw::Font::Regular);
    for c in ['\u{2019}', '\u{00F1}', '\u{0153}', '\u{20AC}'] {
        assert!(face.has(c), "the shipped chain cannot draw {c:?}");
    }
}

#[test]
fn the_chain_is_short_enough_to_be_worth_holding() {
    // Every face in a chain is a parsed megabyte held for the life of the process, and there
    // are three chains. An earlier version added every readable font on the machine — ten
    // faces per role, the chosen one among them twice — for coverage it mostly already had.
    let fonts = FontSet::load(None, None, None).expect("a usable font set");
    for role in [
        rungstar_ui::draw::Font::Regular,
        rungstar_ui::draw::Font::Bold,
        rungstar_ui::draw::Font::Lyrics,
    ] {
        let face = fonts.face(role);
        assert!(
            face.behind().len() <= 6,
            "{role:?} has {} faces behind it: {:?}",
            face.behind().len(),
            face.behind()
        );
        assert!(
            !face.behind().contains(&face.source().to_owned()),
            "{role:?} is its own fallback"
        );
    }
}

#[test]
fn a_face_says_what_it_is() {
    // `--check` prints this, and it is the only way a build machine can tell a release that
    // shipped its own fonts from one quietly borrowing the developer's.
    let Some(face) = bundled("RungStar-Regular.ttf") else {
        return;
    };
    assert_eq!(face.source(), "RungStar-Regular.ttf");
    assert!(face.behind().is_empty());

    let chained = face.with_fallback(bundled("RungStar-Fallback.ttf").expect("the fallback"));
    assert_eq!(chained.behind(), ["RungStar-Fallback.ttf"]);
}
