//! What the interface says happened, which is what a sound is played from.
//!
//! Asserting the *event* rather than the audio is the whole reason chimes are a separate thing
//! from sounds: this runs with no sound card, and it catches the failures that matter — a blip
//! for a press that did nothing, or silence for one that did something.

use rungstar_ui::browse::Browser;
use rungstar_ui::chime::{self, Chime};
use rungstar_ui::menus::{MainMenu, OptionsScreen};
use rungstar_ui::settings::Settings;
use rungstar_ui::songselect::Input;
use rungstar_ui::Cursor;

fn heard(act: impl FnOnce()) -> Vec<Chime> {
    chime::clear();
    act();
    chime::take()
}

#[test]
fn moving_the_cursor_is_heard_once() {
    assert_eq!(
        heard(|| {
            let mut cursor = Cursor::new(5);
            cursor.move_by(1);
        }),
        vec![Chime::Move]
    );
}

#[test]
fn a_press_that_moves_nothing_is_silent() {
    // The failure this exists for: holding a direction at the end of a list that does not wrap
    // would otherwise blip once a frame while nothing on screen changed, which reads as the
    // game being stuck rather than as the list having ended.
    assert!(heard(|| {
        let mut cursor = Cursor::new(4);
        cursor.last();
        for _ in 0..10 {
            cursor.move_clamped(1);
        }
    })
    .contains(&Chime::Move));

    let mut cursor = Cursor::new(4);
    cursor.last();
    assert!(
        heard(|| {
            for _ in 0..10 {
                cursor.move_clamped(1);
            }
        })
        .is_empty(),
        "a clamped cursor at the end chimed anyway"
    );
}

#[test]
fn a_cursor_over_one_item_never_chimes() {
    // A wrapping move on a single-item list returns to where it was. There is nothing to hear.
    assert!(heard(|| {
        let mut cursor = Cursor::new(1);
        cursor.move_by(1);
        cursor.move_by(-1);
    })
    .is_empty());
}

#[test]
fn an_empty_list_never_chimes() {
    assert!(heard(|| {
        let mut cursor = Cursor::new(0);
        cursor.move_by(1);
        cursor.first();
        cursor.last();
    })
    .is_empty());
}

#[test]
fn hovering_is_not_moving() {
    // Pointer code lands the cursor with `set`. A sound for the mouse passing over a list
    // would fire dozens of times crossing it, and the player did not press anything.
    assert!(heard(|| {
        let mut cursor = Cursor::new(10);
        for index in 0..10 {
            cursor.set(index);
        }
    })
    .is_empty());
}

#[test]
fn a_list_shrinking_under_the_cursor_is_not_movement() {
    // `set_count` moves the index when the item it was on stops existing — a search narrowing,
    // a scan finishing. That is the list changing, not the player navigating.
    assert!(heard(|| {
        let mut cursor = Cursor::new(50);
        cursor.set(40);
        cursor.set_count(3);
    })
    .is_empty());
}

#[test]
fn stepping_the_main_menu_is_heard() {
    let mut menu = MainMenu::new();
    assert_eq!(
        heard(|| {
            menu.handle(rungstar_ui::songselect::Input::Down);
        }),
        vec![Chime::Move]
    );
}

#[test]
fn skipping_an_unselectable_row_is_one_move_not_two() {
    // Options pages have heading rows the cursor steps over. Landing past one is a single
    // movement to the player, however many rows it crossed to get there.
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    screen.handle(Input::Confirm, &mut settings);
    assert_eq!(
        heard(|| {
            screen.handle(Input::Down, &mut settings);
        }),
        vec![Chime::Move]
    );
}

#[test]
fn scrolling_a_long_list_chimes_once_per_frame_however_far_it_went() {
    // A fast flick delivers several steps in one frame. The queue collapses them, because
    // eight copies of the same blip a millisecond apart is not eight sounds, it is a buzz.
    let mut browser = Browser::new();
    browser.set_count(30_000);
    assert_eq!(
        heard(|| {
            for _ in 0..8 {
                browser.move_by(1);
            }
        }),
        vec![Chime::Move]
    );
}
