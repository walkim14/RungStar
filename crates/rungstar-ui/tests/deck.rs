//! The Steam Deck pass: every screen at the panel it will actually be played on.
//!
//! A Deck is 1280×800, which is 16:10. The design space is a thousand units tall and as wide
//! as the aspect makes it, so a Deck is **1600×1000 design units** and one design unit is
//! **0.8 physical pixels**. Everything here follows from that arithmetic, and the arithmetic
//! is the reason resolution independence was built the way it was: there is no Deck layout,
//! only the layout, checked at the Deck's shape.
//!
//! Two things are asserted. Nothing is drawn outside the window — a control half off the
//! bottom edge is unreachable on a handheld with no mouse — and no text is too small to read
//! at arm's length on a seven-inch screen.

mod common;

use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::Rect;
use rungstar_ui::theme::Theme;

/// The Deck's panel, in design units.
///
/// 1280 × 800 is 16:10; a thousand units tall makes it 1600 wide.
const DECK: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1600.0,
    h: 1000.0,
};

/// How many physical pixels one design unit is on a Deck.
const PIXELS_PER_UNIT: f32 = 800.0 / 1000.0;

/// The smallest text worth calling readable, in physical pixels.
///
/// A seven-inch panel held at arm's length. Below about twelve pixels a lower-case letter is
/// three pixels of stem and somebody is squinting at a party.
const LEGIBLE: f32 = 12.0;

/// Draw one screen and check it against the Deck.
fn check(name: &str, list: &DrawList) {
    assert!(list.is_balanced(), "{name} left a clip pushed");
    for command in list.commands() {
        match command {
            Command::Rect { rect, .. } | Command::Outline { rect, .. } => {
                assert!(
                    rect.x >= -1.0
                        && rect.y >= -1.0
                        && rect.right() <= DECK.w + 1.0
                        && rect.bottom() <= DECK.h + 1.0,
                    "{name}: something is off the Deck's screen at {rect:?}"
                );
            }
            Command::Text { text, style, .. } => {
                if text.trim().is_empty() {
                    continue;
                }
                let pixels = style.size * PIXELS_PER_UNIT;
                assert!(
                    pixels >= LEGIBLE,
                    "{name}: {text:?} is {pixels:.1} physical pixels on a Deck"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn every_screen_fits_a_steam_deck_and_is_readable_on_it() {
    let style = Theme::builtin().resolve_default();
    for (name, list) in common::every_screen(DECK, &style) {
        check(&name, &list);
    }
}

#[test]
fn the_smallest_text_in_the_theme_is_readable_on_a_deck() {
    // The check above only sees text that happens to be drawn. This one pins the scale itself,
    // so a theme that shrinks the base size cannot quietly take every hint below legibility.
    let style = Theme::builtin().resolve_default();
    let smallest = style.scaled_text(0.7) * PIXELS_PER_UNIT;
    assert!(
        smallest >= LEGIBLE,
        "the smallest text in the theme is {smallest:.1} pixels on a Deck"
    );
}
