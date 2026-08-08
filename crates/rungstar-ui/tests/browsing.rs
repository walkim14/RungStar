//! The three browse layouts. These are the tests that would otherwise be "look at it and see".

use std::collections::HashSet;

use rungstar_ui::browse::{Browser, Layout};
use rungstar_ui::geom::{Point, Rect};

/// The song list area on a Steam Deck, roughly: right two thirds of a 1600x1000 design space.
fn area() -> Rect {
    Rect::new(540.0, 120.0, 1000.0, 760.0)
}

fn settled(layout: Layout, count: usize) -> Browser {
    let mut browser = Browser::new();
    browser.layout = layout;
    browser.set_count(count);
    browser
}

#[test]
fn every_layout_puts_exactly_one_cursor_on_screen() {
    for layout in Layout::ALL {
        for count in [1, 2, 5, 50, 30_000] {
            let mut browser = settled(layout, count);
            browser.jump_to(count / 2);
            let placements = browser.placements(area());
            let selected: Vec<_> = placements.iter().filter(|p| p.selected).collect();
            assert_eq!(
                selected.len(),
                1,
                "{:?} with {count} songs put {} cursors on screen",
                layout,
                selected.len()
            );
            assert_eq!(selected[0].index, count / 2);
        }
    }
}

#[test]
fn no_song_is_shown_twice_however_short_the_list() {
    // A list shorter than the number of visible slots is the case that catches this: the
    // wrap-around fills the spare slots, and without a guard the same song appears three
    // times and clicking the second copy selects the first.
    for layout in Layout::ALL {
        for count in 1..12 {
            let mut browser = settled(layout, count);
            let placements = browser.placements(area());
            let mut seen = HashSet::new();
            for placement in &placements {
                assert!(
                    seen.insert(placement.index),
                    "{layout:?} showed song {} twice with {count} songs",
                    placement.index
                );
                assert!(placement.index < count);
            }
        }
    }
}

#[test]
fn the_cursor_stops_at_both_ends() {
    // The list is not a loop. Holding a direction has to arrive somewhere, and "am I back
    // where I started or is this a different song with the same name" is not a question a
    // song browser should raise.
    for layout in Layout::ALL {
        let mut browser = settled(layout, 10);
        assert_eq!(browser.cursor(), 0);
        browser.move_by(-1);
        assert_eq!(browser.cursor(), 0, "{layout:?} wrapped backwards");
        browser.move_by(9);
        assert_eq!(browser.cursor(), 9);
        browser.move_by(1);
        assert_eq!(browser.cursor(), 9, "{layout:?} wrapped forwards");
        // A page that runs off the end stops at the end rather than doing nothing.
        browser.move_by(-25);
        assert_eq!(browser.cursor(), 0);
    }
}

#[test]
fn nothing_from_the_far_end_is_drawn_beside_the_near_one() {
    // The visible slots past either end stay empty. Drawing the last song above the first is
    // exactly what makes a list feel endless, and it also means the song above the cursor is
    // not the one that sorts before it.
    for layout in Layout::ALL {
        let mut browser = settled(layout, 60);
        for placement in browser.placements(area()) {
            assert!(
                placement.index < 30,
                "{layout:?} drew song {} while at the top of the list",
                placement.index
            );
        }
        browser.jump_to(59);
        for placement in browser.placements(area()) {
            assert!(
                placement.index > 30,
                "{layout:?} drew song {} while at the bottom of the list",
                placement.index
            );
        }
    }
}

#[test]
fn a_jump_across_the_library_lands_rather_than_scrolling_to_it() {
    let mut browser = settled(Layout::List, 30_000);
    browser.move_by(-1);
    assert_eq!(browser.cursor(), 0, "there is nothing above the first song");
    assert!(
        !browser.animating(),
        "a step into the end of the list should not glide anywhere"
    );

    // A jump of twenty thousand rows may not scroll through twenty thousand rows. The view
    // lags by a bounded amount and eases in from there.
    browser.jump_to(0);
    browser.move_by(20_000);
    assert_eq!(browser.cursor(), 20_000);
    for _ in 0..120 {
        browser.tick(1.0 / 60.0);
    }
    assert!(!browser.animating(), "the view never caught up");
    let placements = browser.placements(area());
    let cursor = placements.iter().find(|p| p.selected).unwrap();
    assert!(
        (cursor.rect.center().y - area().center().y).abs() < 1.0,
        "cursor settled at {} instead of centred",
        cursor.rect.center().y
    );
}

#[test]
fn the_view_lags_the_cursor_and_then_catches_up() {
    let mut browser = settled(Layout::List, 100);
    browser.jump_to(50);
    let before = browser.placements(area());
    let centred = before.iter().find(|p| p.selected).unwrap().rect.center().y;

    browser.move_by(1);
    let during = browser.placements(area());
    let lagging = during.iter().find(|p| p.selected).unwrap().rect.center().y;
    assert!(
        lagging > centred,
        "the new cursor should still be below centre while the view catches up"
    );
    assert!(browser.animating());

    // Half a second is plenty for a single step.
    for _ in 0..30 {
        browser.tick(1.0 / 60.0);
    }
    assert!(!browser.animating(), "the view never settled");
    let after = browser.placements(area());
    let settled = after.iter().find(|p| p.selected).unwrap().rect.center().y;
    assert!(
        (settled - centred).abs() < 1.0,
        "settled at {settled}, not {centred}"
    );
}

#[test]
fn a_long_scroll_does_not_build_an_unwindable_backlog() {
    // Holding a direction over a big library must not leave the animation running for
    // seconds after the input stops.
    let mut browser = settled(Layout::List, 30_000);
    for _ in 0..500 {
        browser.move_by(1);
    }
    // One second of animation must be enough to settle, however long the scroll was.
    for _ in 0..60 {
        browser.tick(1.0 / 60.0);
    }
    assert!(
        !browser.animating(),
        "the view is still unwinding a 500-item scroll"
    );
}

#[test]
fn the_animation_settles_at_the_same_speed_on_any_frame_rate() {
    // Framerate-dependent easing makes a 144 Hz display feel snappier than a 60 Hz one, which
    // is the kind of difference that gets blamed on the game feeling "off" on a Deck.
    let position = |fps: f32| {
        let mut browser = settled(Layout::List, 100);
        browser.jump_to(50);
        browser.move_by(1);
        let steps = (fps * 0.1) as usize;
        for _ in 0..steps {
            browser.tick(1.0 / fps);
        }
        browser
            .placements(area())
            .iter()
            .find(|p| p.selected)
            .unwrap()
            .rect
            .y
    };
    let at60 = position(60.0);
    let at144 = position(144.0);
    assert!(
        (at60 - at144).abs() < 2.0,
        "after 100 ms the cursor is at {at60} at 60 Hz but {at144} at 144 Hz"
    );
}

#[test]
fn chessboard_cells_do_not_overlap() {
    let mut browser = settled(Layout::Chessboard, 200);
    browser.jump_to(100);
    let placements = browser.placements(area());
    assert!(placements.len() > 4, "a grid should show several covers");
    for (i, a) in placements.iter().enumerate() {
        for b in placements.iter().skip(i + 1) {
            assert!(
                a.rect.intersect(&b.rect).is_none(),
                "cells {} and {} overlap",
                a.index,
                b.index
            );
        }
    }
}

#[test]
fn the_roulette_puts_the_cursor_in_front_and_largest() {
    let mut browser = settled(Layout::Roulette, 100);
    browser.jump_to(50);
    let placements = browser.placements(area());
    let cursor = placements.iter().find(|p| p.selected).unwrap();

    for other in placements.iter().filter(|p| !p.selected) {
        assert!(
            cursor.rect.w > other.rect.w,
            "song {} is drawn as large as the cursor",
            other.index
        );
        assert!(cursor.emphasis > other.emphasis);
    }
    // Centred horizontally, so the eye has a fixed place to look.
    assert!((cursor.rect.center().x - area().center().x).abs() < 1.0);
    // Drawn last, so a painter's-algorithm backend puts it on top without a depth buffer.
    assert!(placements.last().unwrap().selected);
}

#[test]
fn an_empty_list_draws_nothing_and_survives_every_input() {
    for layout in Layout::ALL {
        let mut browser = settled(layout, 0);
        assert!(browser.is_empty());
        assert!(browser.placements(area()).is_empty());
        // None of these should panic or move a cursor that does not exist.
        browser.move_by(1);
        browser.move_by(-100);
        browser.jump_to(50);
        browser.tick(0.016);
        assert_eq!(browser.cursor(), 0);
        assert!(browser.hit(area(), area().center()).is_none());
    }
}

#[test]
fn a_zero_sized_area_draws_nothing_rather_than_dividing_by_it() {
    // Happens for real: a window minimised to nothing, or a panel with no room left.
    for layout in Layout::ALL {
        let mut browser = settled(layout, 100);
        assert!(browser.placements(Rect::new(0.0, 0.0, 0.0, 0.0)).is_empty());
        assert!(browser
            .placements(Rect::new(0.0, 0.0, 100.0, 0.0))
            .is_empty());
        assert!(browser.page_size(Rect::new(0.0, 0.0, 0.0, 0.0)) >= 1);
    }
}

#[test]
fn shrinking_the_list_keeps_the_cursor_inside_it() {
    // Typing into the search box shrinks the list under the cursor, every keystroke.
    let mut browser = settled(Layout::List, 500);
    browser.jump_to(499);
    browser.set_count(10);
    assert_eq!(browser.cursor(), 9);
    browser.set_count(0);
    assert_eq!(browser.cursor(), 0);
    assert!(browser.placements(area()).is_empty());
}

#[test]
fn clicking_selects_what_is_under_the_pointer() {
    for layout in Layout::ALL {
        let mut browser = settled(layout, 100);
        browser.jump_to(50);
        let placements = browser.placements(area());
        for placement in &placements {
            let hit = browser.hit(area(), placement.rect.center());
            // Roulette covers overlap, so the centre of a back cover can belong to a front
            // one. What must hold is that something is hit, and that it is a real song.
            let hit = hit.expect("a placement's own centre must hit something");
            assert!(hit < 100);
        }
        // Outside the area, nothing.
        assert!(browser.hit(area(), Point::new(-50.0, -50.0)).is_none());
    }
}

#[test]
fn page_size_reflects_what_is_actually_on_screen() {
    for layout in Layout::ALL {
        let browser = settled(layout, 1000);
        let page = browser.page_size(area());
        assert!(page >= 1, "{layout:?} pages by zero");
        assert!(
            page <= 64,
            "{layout:?} pages by {page}, which is not a screenful"
        );
    }
}

#[test]
fn switching_layout_keeps_your_place() {
    // The cursor, the filter and the scroll position do not belong to the layout, so changing
    // how the list looks must not change where you are in it.
    let mut browser = settled(Layout::List, 1000);
    browser.jump_to(417);
    for layout in Layout::ALL {
        browser.layout = layout;
        assert_eq!(browser.cursor(), 417);
        let placements = browser.placements(area());
        assert!(placements.iter().any(|p| p.index == 417 && p.selected));
    }
}
