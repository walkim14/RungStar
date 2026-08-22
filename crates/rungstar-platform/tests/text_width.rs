//! The layout estimate has to be an upper bound on what the font actually draws.
//!
//! `rungstar-ui` has no font in it, so a screen that needs a width before there are pixels asks
//! `approx_text_width`. The sing screen lays a line's syllables edge to edge from those numbers
//! and centres each one in the box it was given — so a syllable whose real ink is wider than its
//! estimate spills half the difference into the syllable on each side, and the words are drawn
//! through each other. Fast lines show it first because they hold the most syllables.
//!
//! Measured across seven faces, the old flat average of 0.55 em per character was not a "slight
//! over-estimate": `'W'` is 1.13 em and `'m'` is 1.06, so "Wo" was given 0.73 of the room it
//! needed. This asserts the direction of the error rather than its size — over is a gap, under
//! is a collision.

use rungstar_platform::font::Face;
use rungstar_ui::draw::approx_text_width;

/// Faces to check against, whichever of them this machine has.
fn faces() -> Vec<(String, Face)> {
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\verdanab.ttf",
            r"C:\Windows\Fonts\tahomabd.ttf",
        ]
    } else {
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ]
    };
    candidates
        .iter()
        .filter_map(|path| {
            Face::load(std::path::Path::new(path))
                .ok()
                .map(|face| ((*path).to_owned(), face))
        })
        .collect()
}

/// Syllables of the kind a note file actually holds, wide ones first.
const SYLLABLES: &[&str] = [
    "Wo", "Wa", "Whoa", "WWW", "Mmm", "mmm", "My", "Mm", "Wow", "AWW", "Wan", "wom", "man", "Ne",
    "ver ", "gon", "na ", "give", "you ", "up", "I'm", "the ", "and ", "a ", "'s", "it", "lil",
    "OH", "NO", "MAMMA", "MIA", "Dan", "cing", "Queen", "Wa", "ter", "loo",
]
.as_slice();

#[test]
fn no_syllable_is_wider_than_the_room_the_layout_gives_it() {
    let faces = faces();
    assert!(
        !faces.is_empty(),
        "no font on this machine to check against; the estimate went unverified"
    );

    let mut worst: Option<(f32, String)> = None;
    for (name, face) in &faces {
        for size in [24.0f32, 48.0, 64.0] {
            for syllable in SYLLABLES {
                let real = face.width(syllable, size);
                let given = approx_text_width(syllable, size);
                let ratio = real / given.max(0.001);
                if worst.as_ref().is_none_or(|(w, _)| ratio > *w) {
                    worst = Some((
                        ratio,
                        format!(
                            "{syllable:?} at {size} in {name}: needs {real:.1}, given {given:.1}"
                        ),
                    ));
                }
            }
        }
    }
    let (ratio, what) = worst.expect("nothing was measured");
    assert!(
        ratio <= 1.0,
        "a syllable is drawn wider than its box, so it runs into its neighbours -- {what}"
    );
}
