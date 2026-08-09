//! The on-screen keyboard and the cursor rules every menu shares.

use rungstar_ui::keyboard::{Key, Keyboard, Page, COLUMNS};
use rungstar_ui::menu::{Cursor, Repeat};

/// Type a string by walking the grid to each character, the way a controller would.
fn type_with_dpad(keyboard: &mut Keyboard, text: &str) {
    for wanted in text.chars() {
        let target = keyboard
            .keys()
            .iter()
            .position(|k| *k == Key::Char(wanted))
            .unwrap_or_else(|| panic!("{wanted} is not on the {:?} page", keyboard.page()));
        // Walk there rather than jumping, so this exercises navigation as well as pressing.
        let mut guard = 0;
        while keyboard.cursor() != target {
            let (row, column) = Keyboard::position(keyboard.cursor());
            let (target_row, target_column) = Keyboard::position(target);
            if row != target_row {
                keyboard.navigate(0, if target_row > row { 1 } else { -1 });
            } else {
                keyboard.navigate(if target_column > column { 1 } else { -1 }, 0);
            }
            guard += 1;
            assert!(guard < 200, "could not reach {wanted}");
        }
        assert!(!keyboard.press(), "a character key finished editing");
    }
}

#[test]
fn a_controller_can_type_a_search_term() {
    // The whole reason this exists: searching 30,000 songs from a sofa. UltraStar Deluxe has
    // no on-screen keyboard at all, so this is not reachable there.
    let mut keyboard = Keyboard::new();
    type_with_dpad(&mut keyboard, "queen");
    assert_eq!(keyboard.text(), "queen");

    keyboard.apply(Key::Space);
    type_with_dpad(&mut keyboard, "bo");
    assert_eq!(keyboard.text(), "queen bo");

    keyboard.apply(Key::Backspace);
    assert_eq!(keyboard.text(), "queen b");
    keyboard.apply(Key::Clear);
    assert!(keyboard.is_empty());
}

#[test]
fn done_is_the_only_key_that_finishes_editing() {
    let mut keyboard = Keyboard::new();
    for key in keyboard.keys() {
        let finished = keyboard.apply(key);
        assert_eq!(
            finished,
            key == Key::Done,
            "{key:?} reported finishing = {finished}"
        );
    }
}

#[test]
fn navigation_wraps_within_the_row_and_the_column() {
    // A d-pad press that does nothing reads as an input that was missed, so every direction
    // from every key must land somewhere.
    let mut keyboard = Keyboard::new();
    let total = keyboard.keys().len();
    for start in 0..total {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let mut probe = Keyboard::new();
            for _ in 0..start {
                probe.navigate(1, 0);
            }
            let before = probe.cursor();
            probe.navigate(dx, dy);
            assert!(
                probe.cursor() < total,
                "navigated off the grid from {before}"
            );
        }
    }
    // Right from the end of a row comes back to its start.
    keyboard.navigate(-1, 0);
    let (row, column) = Keyboard::position(keyboard.cursor());
    assert_eq!(row, 0);
    assert_eq!(column, COLUMNS - 1);
}

#[test]
fn moving_down_a_short_final_row_lands_on_a_real_key() {
    // The last row is usually short. Sliding down a right-hand column must reach its last
    // key rather than falling into a gap.
    let mut keyboard = Keyboard::new();
    let total = keyboard.keys().len();
    for _ in 0..COLUMNS - 1 {
        keyboard.navigate(1, 0);
    }
    for _ in 0..keyboard.rows() * 2 {
        keyboard.navigate(0, 1);
        assert!(keyboard.cursor() < total);
    }
}

#[test]
fn every_page_is_reachable_and_the_controls_are_on_all_of_them() {
    // A player who switched to symbols for an apostrophe should not have to switch back to
    // press backspace.
    let mut keyboard = Keyboard::new();
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(keyboard.page());
        let keys = keyboard.keys();
        for control in [
            Key::Space,
            Key::Backspace,
            Key::Clear,
            Key::Done,
            Key::Shift,
        ] {
            assert!(
                keys.contains(&control),
                "{control:?} missing from {:?}",
                keyboard.page()
            );
        }
        keyboard.apply(Key::Shift);
    }
    assert_eq!(
        seen,
        vec![Page::Letters, Page::Capitals, Page::Symbols, Page::Accents]
    );
    assert_eq!(keyboard.page(), Page::Letters, "the pages must cycle");
}

#[test]
fn accented_characters_are_typable_so_a_european_library_is_searchable() {
    let mut keyboard = Keyboard::new();
    for _ in 0..3 {
        keyboard.apply(Key::Shift);
    }
    assert_eq!(keyboard.page(), Page::Accents);
    type_with_dpad(&mut keyboard, "öü");
    assert_eq!(keyboard.text(), "öü");
}

#[test]
fn switching_page_never_leaves_the_cursor_off_the_grid() {
    // The pages have different lengths, so a cursor at the end of the longest one must be
    // pulled back when a shorter page appears.
    let mut keyboard = Keyboard::new();
    for _ in 0..200 {
        keyboard.navigate(1, 0);
        keyboard.navigate(0, 1);
    }
    for _ in 0..6 {
        keyboard.apply(Key::Shift);
        assert!(keyboard.cursor() < keyboard.keys().len());
        // And the key under the cursor is a real one, not a panic waiting to happen.
        let _ = keyboard.selected();
    }
}

#[test]
fn the_field_has_a_limit_and_control_characters_are_ignored() {
    let mut keyboard = Keyboard::new().limit(5);
    for _ in 0..50 {
        keyboard.push('a');
    }
    assert_eq!(keyboard.text(), "aaaaa");

    let mut keyboard = Keyboard::new();
    keyboard.push('\n');
    keyboard.push('\t');
    keyboard.push('\u{7}');
    assert!(
        keyboard.is_empty(),
        "control characters must not enter the field"
    );
}

#[test]
fn existing_text_can_be_edited_and_is_truncated_to_the_limit() {
    let mut keyboard = Keyboard::with_text("Bohemian");
    assert_eq!(keyboard.text(), "Bohemian");
    keyboard.apply(Key::Backspace);
    assert_eq!(keyboard.text(), "Bohemia");

    let mut keyboard = Keyboard::new().limit(4);
    keyboard.set_text("far too long");
    assert_eq!(keyboard.text(), "far ");
}

#[test]
fn a_cursor_wraps_but_paging_stops_at_the_ends() {
    // Wrapping is right for a menu, where a dead press reads as a missed input. It is wrong
    // for paging through a long list, where reappearing at the top loses your place.
    let mut cursor = Cursor::new(5);
    cursor.move_by(-1);
    assert_eq!(cursor.index(), 4);
    cursor.move_by(1);
    assert_eq!(cursor.index(), 0);

    cursor.move_clamped(-1);
    assert_eq!(cursor.index(), 0);
    cursor.move_clamped(100);
    assert_eq!(cursor.index(), 4);
}

#[test]
fn a_cursor_over_nothing_survives_every_input() {
    let mut cursor = Cursor::new(0);
    assert!(cursor.is_empty());
    cursor.move_by(3);
    cursor.move_clamped(-3);
    cursor.set(10);
    cursor.last();
    assert_eq!(cursor.index(), 0);
}

#[test]
fn a_cursor_skips_rows_that_cannot_be_selected() {
    // Options pages have heading rows, and a cursor that lands on one appears to have
    // vanished.
    let selectable = |i: usize| i % 3 != 0;
    let mut cursor = Cursor::new(9);
    cursor.set(1);
    cursor.move_selectable(1, selectable);
    assert_eq!(cursor.index(), 2);
    cursor.move_selectable(1, selectable);
    assert_eq!(cursor.index(), 4, "row 3 is a heading and must be skipped");

    // Nothing selectable at all leaves the cursor where it was rather than looping forever.
    let mut cursor = Cursor::new(4);
    cursor.move_selectable(1, |_| false);
    assert_eq!(cursor.index(), 0);
}

#[test]
fn a_held_direction_repeats_after_a_pause_and_then_accelerates() {
    let mut repeat = Repeat::default();
    assert_eq!(repeat.press(), 1, "a press steps immediately");

    // Nothing during the initial pause, or a tap would scroll two items.
    assert_eq!(repeat.tick(0.1), 0);
    assert_eq!(repeat.tick(0.1), 0);
    // Then it starts.
    assert!(repeat.tick(0.2) >= 1);

    // And gets faster: the same slice of time yields more steps later than earlier.
    let early = repeat.tick(0.2);
    let late = repeat.tick(0.2);
    assert!(
        late >= early,
        "repeat did not accelerate: {early} then {late}"
    );
}

#[test]
fn a_stalled_frame_still_scrolls_the_right_distance_without_exploding() {
    // Dropping the extra steps would make a stutter also lose input; taking all of them
    // after a one-second stall would jump fifty items.
    let mut repeat = Repeat::default();
    repeat.press();
    let steps = repeat.tick(1.0);
    assert!(steps > 1, "a long frame should catch up");
    assert!(steps <= 32, "a long frame should not fire {steps} steps");
}

#[test]
fn every_printable_ascii_character_can_be_typed_from_a_controller() {
    // The on-screen keyboard was built for search, which is case-insensitive and uses about
    // twenty punctuation marks. A password is neither. Before this, a capital letter simply
    // could not be typed from a controller at all, which locks somebody out of their account
    // with nothing on screen to say why.
    //
    // Asserted over the whole printable range rather than over a chosen list, so the next
    // person with an unusual password does not find a new gap.
    let mut reachable: std::collections::HashSet<char> = std::collections::HashSet::new();
    let mut keyboard = Keyboard::new();
    for _ in 0..8 {
        for key in keyboard.keys() {
            match key {
                Key::Char(c) => {
                    reachable.insert(c);
                }
                Key::Space => {
                    reachable.insert(' ');
                }
                _ => {}
            }
        }
        keyboard.apply(Key::Shift);
    }

    let missing: Vec<char> = (0x20u8..0x7F)
        .map(char::from)
        .filter(|c| !reachable.contains(c))
        .collect();
    assert!(
        missing.is_empty(),
        "these cannot be typed from a controller: {missing:?}"
    );
}

#[test]
fn the_pages_come_round_rather_than_stopping() {
    // Four pages now: lower case, capitals, symbols, accents. A shift key that stops on the
    // last one strands whoever is on it.
    let mut keyboard = Keyboard::new();
    let first = keyboard.page();
    let mut seen = vec![first];
    for _ in 0..3 {
        keyboard.apply(Key::Shift);
        seen.push(keyboard.page());
    }
    assert_eq!(seen.len(), 4);
    keyboard.apply(Key::Shift);
    assert_eq!(keyboard.page(), first, "the pages did not come round");
}

#[test]
fn a_mixed_password_types_the_same_way_it_reads() {
    // Driven through the on-screen keyboard a key at a time, as a controller would, with the
    // page switched whenever the next character is on a different one.
    let mut keyboard = Keyboard::new().limit(64);
    let wanted = "aZ0*^&%_Xv";
    for want in wanted.chars() {
        let mut found = false;
        for _ in 0..4 {
            if let Some(index) = keyboard
                .keys()
                .iter()
                .position(|key| *key == Key::Char(want))
            {
                keyboard.set_cursor(index);
                keyboard.press();
                found = true;
                break;
            }
            keyboard.apply(Key::Shift);
        }
        assert!(found, "{want:?} is on no page of the keyboard");
    }
    assert_eq!(keyboard.text(), wanted);
}
