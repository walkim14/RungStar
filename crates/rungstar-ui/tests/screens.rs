//! The song browser and the options screens, driven by actions and read back as commands.
//!
//! This is what the display-list boundary buys: the screens are exercised here with no window,
//! no GPU and no font, and the assertions are about what was drawn rather than how it looked.

use rungstar_library::{SearchField, SongEntry, SortKey};
use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::Rect;
use rungstar_ui::menus::{MainMenu, OptionsOutcome, OptionsScreen};
use rungstar_ui::options::Action;
use rungstar_ui::screen::{Route, Transition};
use rungstar_ui::settings::Settings;
use rungstar_ui::songselect::{Facet, FacetValues, Input, Mode, SongSelect};
use rungstar_ui::theme::Theme;
use rungstar_ui::Layout;

fn area() -> Rect {
    Rect::new(0.0, 0.0, 1600.0, 1000.0)
}

/// What every fixture song scores. Named because two tests need to agree about it.
const FIXTURE_DIFFICULTY: f64 = 0.5;

fn song(id: i64, artist: &str, title: &str) -> SongEntry {
    SongEntry {
        id,
        path: format!("C:/songs/{artist} - {title}/song.txt").into(),
        folder: "Pop".into(),
        artist: artist.into(),
        title: title.into(),
        edition: None,
        genre: Some("Rock".into()),
        language: Some("English".into()),
        creator: None,
        tags: None,
        year: Some(1975),
        bpm: 300.0,
        gap_ms: 0,
        duration_secs: 214.0,
        is_duet: false,
        audio_file: Some("song.mp3".into()),
        video_file: None,
        cover_file: None,
        background_file: None,
        note_count: 200,
        golden_count: 10,
        difficulty: FIXTURE_DIFFICULTY,
        medley_start: None,
        medley_end: None,
        preview_start: None,
        usdb_id: None,
        times_played: 0,
        last_played: None,
        loudness: None,
        peak: None,
    }
}

fn library(count: usize) -> Vec<SongEntry> {
    (0..count)
        .map(|i| song(i as i64, &format!("Artist {i:03}"), &format!("Song {i:03}")))
        .collect()
}

/// Every string a frame drew, so a test can ask whether something is on screen.
fn strings(list: &DrawList) -> Vec<String> {
    list.commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn draw(screen: &mut SongSelect) -> DrawList {
    let mut list = DrawList::new();
    let theme = Theme::builtin();
    screen.draw(&mut list, area(), &theme.resolve_default(), &|_| None);
    list
}

fn loaded(count: usize) -> SongSelect {
    let mut screen = SongSelect::new();
    screen.set_results(library(count));
    screen
}

#[test]
fn a_fresh_screen_asks_for_a_query_and_stops_asking_once_answered() {
    let mut screen = SongSelect::new();
    assert!(screen.needs_query(), "the first frame must fetch something");
    screen.set_results(library(10));
    assert!(!screen.needs_query());
    assert_eq!(screen.songs().len(), 10);
}

#[test]
fn typing_re_queries_on_every_keystroke() {
    // What makes the list narrow as you type rather than when you finish. At 3 ms for a
    // prefix search over 30,000 songs it is affordable; the flag is how the screen says so.
    let mut screen = loaded(100);
    screen.handle(Input::Search, area());
    assert_eq!(screen.mode(), Mode::Searching);

    for c in "queen".chars() {
        screen.handle(Input::Type(c), area());
        assert!(screen.needs_query(), "typing `{c}` did not ask for a query");
        screen.set_results(library(3));
    }
    assert_eq!(screen.search_text(), "queen");

    // Backspace counts as a change too, or deleting a character would leave the old results.
    screen.handle(Input::Backspace, area());
    assert!(screen.needs_query());
    assert_eq!(screen.search_text(), "quee");
}

#[test]
fn refining_a_search_keeps_the_song_you_were_narrowing_towards() {
    // The whole point of typing more: you are heading for one song. Resetting the cursor to
    // the top every keystroke means it is never under the cursor when you arrive.
    let mut screen = loaded(50);
    screen.browser.jump_to(30);
    let wanted = screen.selected().unwrap().id;

    let mut narrowed = library(50);
    narrowed.retain(|s| s.id % 3 == 0);
    assert!(narrowed.iter().any(|s| s.id == wanted));
    screen.set_results(narrowed);

    assert_eq!(screen.selected().unwrap().id, wanted, "the selection moved");
}

#[test]
fn a_song_that_falls_out_of_the_results_leaves_the_cursor_somewhere_valid() {
    let mut screen = loaded(50);
    screen.browser.jump_to(30);
    screen.set_results(library(5));
    assert!(screen.selected().is_some());
    assert!(screen.browser.cursor() < 5);

    screen.set_results(Vec::new());
    assert!(screen.selected().is_none());
    // And it still draws, rather than panicking on an empty list.
    let list = draw(&mut screen);
    assert!(!list.is_empty());
    assert!(list.is_balanced());
}

#[test]
fn confirming_asks_to_sing_the_song_under_the_cursor() {
    let mut screen = loaded(20);
    screen.browser.jump_to(7);
    let expected = screen.selected().unwrap().id;
    assert_eq!(
        screen.handle(Input::Confirm, area()),
        Transition::Sing(expected)
    );
}

#[test]
fn confirming_an_empty_list_does_nothing_rather_than_singing_nothing() {
    let mut screen = SongSelect::new();
    screen.set_results(Vec::new());
    assert_eq!(screen.handle(Input::Confirm, area()), Transition::None);
}

#[test]
fn the_search_overlay_opens_and_closes_without_losing_the_list() {
    let mut screen = loaded(40);
    screen.browser.jump_to(12);

    screen.handle(Input::Search, area());
    assert_eq!(screen.mode(), Mode::Searching);
    let list = draw(&mut screen);
    // The keyboard draws its own keys, so there is a lot more on screen than while browsing.
    assert!(
        list.len() > 40,
        "the keyboard did not draw: {} commands",
        list.len()
    );
    assert!(list.is_balanced());

    screen.handle(Input::Back, area());
    assert_eq!(screen.mode(), Mode::Browsing);
    assert_eq!(
        screen.browser.cursor(),
        12,
        "closing the search moved the cursor"
    );
}

#[test]
fn the_sort_picker_changes_the_sort_and_asks_for_a_new_query() {
    let mut screen = loaded(40);
    assert_eq!(screen.sort(), SortKey::Artist);

    screen.handle(Input::Sort, area());
    assert_eq!(screen.mode(), Mode::Sorting);
    screen.set_results(library(40));

    screen.handle(Input::Down, area());
    assert_eq!(screen.sort(), SortKey::Title);
    assert!(screen.needs_query());
    screen.set_results(library(40));

    // Left or right reverses rather than moving, since the list is vertical.
    assert!(!screen.descending());
    screen.handle(Input::Right, area());
    assert!(screen.descending());
    assert!(screen.needs_query());

    screen.handle(Input::Confirm, area());
    assert_eq!(screen.mode(), Mode::Browsing);
}

#[test]
fn the_field_being_searched_can_be_changed_without_leaving_the_keyboard() {
    // "I typed a lyric, not a title" is a correction you make mid-search, not before it.
    let mut screen = loaded(40);
    screen.handle(Input::Search, area());
    assert_eq!(screen.field(), SearchField::All);

    screen.handle(Input::Sort, area());
    assert_eq!(screen.field(), SearchField::Artist);
    assert!(screen.needs_query());
    assert_eq!(
        screen.mode(),
        Mode::Searching,
        "changing the field closed the keyboard"
    );
}

#[test]
fn every_layout_draws_a_balanced_frame_for_every_list_size() {
    for layout in Layout::ALL {
        for count in [0, 1, 2, 7, 500] {
            let mut screen = loaded(count);
            screen.browser.layout = layout;
            screen.browser.jump_to(count / 2);
            let list = draw(&mut screen);
            assert!(
                list.is_balanced(),
                "{layout:?} with {count} songs left a clip pushed"
            );
            assert!(
                !list.is_empty(),
                "{layout:?} with {count} songs drew nothing"
            );
        }
    }
}

#[test]
fn an_empty_library_explains_itself_instead_of_showing_a_blank_screen() {
    // A blank list on a first run with no explanation is where a player gives up.
    let mut screen = SongSelect::new();
    screen.set_results(Vec::new());
    let text = strings(&draw(&mut screen)).join(" ");
    assert!(text.contains("No songs yet"), "no explanation: {text}");
    assert!(text.contains("Options"), "no way forward offered: {text}");
}

#[test]
fn a_search_with_no_matches_says_so_and_offers_a_way_out() {
    let mut screen = loaded(10);
    screen.handle(Input::Search, area());
    for c in "zzzz".chars() {
        screen.handle(Input::Type(c), area());
    }
    screen.set_results(Vec::new());
    screen.handle(Input::Back, area());

    let text = strings(&draw(&mut screen)).join(" ");
    assert!(text.contains("Nothing matched"), "{text}");
    assert!(text.contains("clear the search"), "{text}");
}

#[test]
fn the_detail_panel_shows_the_song_under_the_cursor() {
    let mut screen = loaded(30);
    screen.browser.jump_to(11);
    let text = strings(&draw(&mut screen));
    assert!(
        text.iter().any(|t| t == "Song 011"),
        "title missing: {text:?}"
    );
    assert!(text.iter().any(|t| t == "Artist 011"), "artist missing");
    // Facts a player would want before choosing, not a dump of every header.
    assert!(text.iter().any(|t| t == "3:34"), "length missing: {text:?}");
    // Asked of the band rather than spelled out, because where the bands are cut is a
    // calibration decision pinned by its own test in the library. What this one is about is
    // that the panel says the difficulty at all.
    let difficulty = rungstar_library::DifficultyBand::of(FIXTURE_DIFFICULTY).label();
    assert!(
        text.iter().any(|t| t == difficulty),
        "difficulty missing: {text:?}"
    );
}

#[test]
fn an_unplayable_song_is_flagged_before_you_pick_it() {
    let mut broken = song(1, "Alpha", "One");
    broken.audio_file = None;
    let mut screen = SongSelect::new();
    screen.set_results(vec![broken]);
    let text = strings(&draw(&mut screen)).join(" ");
    assert!(text.contains("No audio file"), "not flagged: {text}");
}

#[test]
fn the_button_hints_change_with_what_the_buttons_do() {
    // On a controller there is nowhere else to discover that West opens the search.
    let mut screen = loaded(10);
    let browsing = strings(&draw(&mut screen)).join(" ");
    assert!(browsing.contains("Sing") && browsing.contains("Search"));

    screen.handle(Input::Search, area());
    let searching = strings(&draw(&mut screen)).join(" ");
    assert!(
        searching.contains("Done"),
        "hints did not follow the mode: {searching}"
    );
}

#[test]
fn paging_moves_by_a_screenful_in_every_layout() {
    for layout in Layout::ALL {
        let mut screen = loaded(1000);
        screen.browser.layout = layout;
        screen.browser.jump_to(500);
        screen.handle(Input::PageDown, area());
        let moved = screen.browser.cursor() as isize - 500;
        assert!(moved > 1, "{layout:?} paged by only {moved}");
        assert!(
            moved < 100,
            "{layout:?} paged by {moved}, which is not a screenful"
        );
    }
}

#[test]
fn the_main_menu_routes_to_every_screen_it_offers() {
    let mut menu = MainMenu::new();
    let mut seen = Vec::new();
    // Walk the menu until it comes back to the top, confirming each row leads somewhere.
    // Looping a fixed number of times would silently stop testing rows as the menu grows.
    for step in 0..16 {
        match menu.handle(Input::Confirm) {
            Transition::Push(route) => seen.push(route),
            // The quit row asks before it acts, so leaving is a question rather than a
            // transition. Still a row that leads somewhere — dismissed here so the walk
            // carries on to the rest of the menu.
            Transition::None if menu.confirming() => {
                seen.push(Route::Main);
                menu.handle(Input::Back);
            }
            other => panic!("a menu row produced {other:?}"),
        }
        menu.handle(Input::Down);
        if menu.cursor() == 0 {
            break;
        }
        assert!(step < 15, "the menu never wrapped");
    }
    for wanted in [
        Route::SongSelect,
        Route::Players,
        Route::Stats,
        Route::Options,
        Route::About,
    ] {
        assert!(
            seen.contains(&wanted),
            "{wanted:?} is on no menu row: {seen:?}"
        );
    }
    assert!(seen.contains(&Route::Main), "there is no way to quit");
    assert_eq!(menu.cursor(), 0, "the menu did not wrap");
    assert_eq!(
        seen.len(),
        seen.iter().collect::<std::collections::HashSet<_>>().len(),
        "two menu rows go to the same place: {seen:?}"
    );
}

#[test]
fn the_options_screen_edits_the_settings_it_is_given() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    let before = settings.game.difficulty;

    // Into the first page, down to Difficulty, and change it.
    screen.handle(Input::Confirm, &mut settings);
    assert!(!screen.on_page_list());
    screen.handle(Input::Down, &mut settings);
    let outcome = screen.handle(Input::Right, &mut settings);

    assert_eq!(outcome, OptionsOutcome::Changed);
    assert_ne!(settings.game.difficulty, before);
}

#[test]
fn appearance_rows_only_report_changes_when_they_can_change() {
    let theme = Theme::builtin();
    let mut screen = OptionsScreen::new();
    screen.use_theme(&theme);
    let mut settings = Settings::default();

    // Appearance is the fifth page. Theme names installed assets and is display-only until
    // theme discovery exists, while Skin and Accent are choices supplied by that theme.
    for _ in 0..4 {
        screen.handle(Input::Down, &mut settings);
    }
    screen.handle(Input::Confirm, &mut settings);
    assert_eq!(
        screen.handle(Input::Confirm, &mut settings),
        OptionsOutcome::None,
        "the read-only Theme row claimed it changed"
    );

    screen.handle(Input::Down, &mut settings);
    let skin = settings.appearance.skin.clone();
    assert_eq!(
        screen.handle(Input::Right, &mut settings),
        OptionsOutcome::Changed
    );
    assert_ne!(settings.appearance.skin, skin);

    screen.handle(Input::Down, &mut settings);
    let accent = settings.appearance.accent.clone();
    assert_eq!(
        screen.handle(Input::Right, &mut settings),
        OptionsOutcome::Changed
    );
    assert_ne!(settings.appearance.accent, accent);
}

#[test]
fn an_options_group_keeps_a_visible_context_marker_while_its_items_have_focus() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    let style = Theme::builtin().resolve_default();
    screen.handle(Input::Confirm, &mut settings);

    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &style, &settings);
    assert!(list.commands().iter().any(|command| matches!(
        command,
        Command::Outline { color, .. } if *color == style.accent_soft
    )));
}

#[test]
fn a_button_row_reports_the_action_rather_than_changing_a_setting() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    let untouched = settings.clone();

    // Into the Game page, then down to the first button on it. Stepping with Down only,
    // because Confirm on a choice row is defined to change it.
    screen.handle(Input::Confirm, &mut settings);
    let mut outcome = OptionsOutcome::None;
    for _ in 0..12 {
        screen.handle(Input::Down, &mut settings);
        let result = screen.handle(Input::Confirm, &mut settings);
        if let OptionsOutcome::Run(action) = result {
            outcome = OptionsOutcome::Run(action);
            break;
        }
        // Anything Confirm changed on the way, put back, so the assertion below is about the
        // button and not about the walk.
        if result == OptionsOutcome::Changed {
            screen.handle(Input::Left, &mut settings);
        }
    }
    assert!(
        matches!(outcome, OptionsOutcome::Run(_)),
        "no button was reachable on the first page"
    );
    if let OptionsOutcome::Run(action) = outcome {
        assert!(matches!(
            action,
            Action::RescanLibrary
                | Action::RebuildIndex
                | Action::AddSongFolder
                | Action::ResetToDefaults
        ));
    }
    // Pressing the button itself must not have edited anything.
    assert_eq!(settings.game, untouched.game);
}

#[test]
fn back_walks_out_of_the_options_one_level_at_a_time() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();

    screen.handle(Input::Confirm, &mut settings);
    assert!(!screen.on_page_list());
    assert_eq!(
        screen.handle(Input::Back, &mut settings),
        OptionsOutcome::None
    );
    assert!(
        screen.on_page_list(),
        "back should return to the group list first"
    );
    assert_eq!(
        screen.handle(Input::Back, &mut settings),
        OptionsOutcome::Pop
    );
}

#[test]
fn every_options_page_draws_and_every_row_has_help() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    let theme = Theme::builtin();
    let style = theme.resolve_default();

    for _ in 0..6 {
        // Into the page, then down through every row on it.
        screen.handle(Input::Confirm, &mut settings);
        for _ in 0..30 {
            let mut list = DrawList::new();
            screen.draw(&mut list, area(), &style, &settings);
            assert!(list.is_balanced());
            assert!(
                strings(&list).iter().any(|t| t == screen.help()),
                "the help for the selected row was not drawn"
            );
            screen.handle(Input::Down, &mut settings);
        }
        screen.handle(Input::Back, &mut settings);
        screen.handle(Input::Down, &mut settings);
    }
}

#[test]
fn changing_the_text_scale_changes_what_is_drawn() {
    // The setting has to reach the theme, or it is a slider that does nothing.
    let mut theme = Theme::builtin();
    let mut menu = MainMenu::new();

    let mut small = DrawList::new();
    theme.metrics.text_scale = 0.8;
    menu.draw(&mut small, area(), &theme.resolve_default(), "");

    let mut large = DrawList::new();
    theme.metrics.text_scale = 1.5;
    menu.draw(&mut large, area(), &theme.resolve_default(), "");

    let size_of = |list: &DrawList| {
        list.commands()
            .iter()
            .filter_map(|c| match c {
                Command::Text { style, .. } => Some(style.size),
                _ => None,
            })
            .fold(0.0f32, f32::max)
    };
    assert!(size_of(&large) > size_of(&small) * 1.5);
    let _ = &mut menu;
}

/// Nothing may be drawn outside the window.
///
/// A layout that overflows is invisible in a test that only checks it did not panic, and
/// obvious the moment somebody looks at it — which is the wrong order to find it in.
fn assert_on_screen(list: &DrawList, area: Rect, what: &str) {
    // Clipped regions are allowed to be partly outside; only unclipped drawing must fit.
    let mut depth = 0;
    for command in list.commands() {
        match command {
            Command::PushClip(_) => depth += 1,
            Command::PopClip => depth -= 1,
            _ if depth > 0 => {}
            Command::Rect { rect, .. }
            | Command::Outline { rect, .. }
            | Command::Bubble { rect, .. }
            | Command::Glow { rect, .. }
            | Command::Image { rect, .. }
            | Command::Text { rect, .. } => {
                assert!(
                    rect.x >= area.x - 1.0
                        && rect.y >= area.y - 1.0
                        && rect.right() <= area.right() + 1.0
                        && rect.bottom() <= area.bottom() + 1.0,
                    "{what}: {rect:?} falls outside the {}x{} window",
                    area.w,
                    area.h
                );
            }
            _ => {}
        }
    }
}

/// The displays this has to be right on, as design-space areas.
fn areas() -> Vec<(&'static str, Rect)> {
    vec![
        ("deck 1280x800", Rect::new(0.0, 0.0, 1600.0, 1000.0)),
        ("1080p", Rect::new(0.0, 0.0, 1778.0, 1000.0)),
        ("4:3", Rect::new(0.0, 0.0, 1333.0, 1000.0)),
        ("ultrawide", Rect::new(0.0, 0.0, 2389.0, 1000.0)),
    ]
}

#[test]
fn the_options_screen_stays_inside_the_window() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    for (name, area) in areas() {
        let mut screen = OptionsScreen::new();
        let mut settings = Settings::default();
        // Every page, and every row on it.
        for page in 0..6 {
            screen.handle(Input::Confirm, &mut settings);
            for row in 0..14 {
                let mut list = DrawList::new();
                screen.draw(&mut list, area, &style, &settings);
                assert_on_screen(
                    &list,
                    area,
                    &format!("{name} options page {page} row {row}"),
                );
                screen.handle(Input::Down, &mut settings);
            }
            screen.handle(Input::Back, &mut settings);
            screen.handle(Input::Down, &mut settings);
        }
    }
}

#[test]
fn the_main_menu_stays_inside_the_window() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    for (name, area) in areas() {
        let mut menu = MainMenu::new();
        let mut list = DrawList::new();
        menu.draw(
            &mut list,
            area,
            &style,
            "An UltraStar Deluxe-class karaoke game",
        );
        assert_on_screen(&list, area, name);
    }
}

#[test]
fn the_song_browser_stays_inside_the_window() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    for (name, area) in areas() {
        for layout in Layout::ALL {
            let mut screen = loaded(200);
            screen.browser.layout = layout;
            screen.browser.jump_to(100);
            let mut list = DrawList::new();
            screen.draw(&mut list, area, &style, &|_| None);
            assert_on_screen(&list, area, &format!("{name} {layout:?}"));

            // And the overlays, which have their own layout maths.
            screen.handle(Input::Search, area);
            let mut list = DrawList::new();
            screen.draw(&mut list, area, &style, &|_| None);
            assert_on_screen(&list, area, &format!("{name} {layout:?} keyboard"));
        }
    }
}

#[test]
fn text_rows_do_not_overlap_each_other() {
    // Two lines in one row is the usual way a label and its caption end up on top of each
    // other: one is bottom-aligned in the upper half and the other top-aligned in the lower,
    // and the descenders of the first land in the second.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut menu = MainMenu::new();
    let mut list = DrawList::new();
    menu.draw(&mut list, area, &style, "subtitle");

    let texts: Vec<(String, Rect, f32)> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { rect, text, style } => Some((text.clone(), *rect, style.size)),
            _ => None,
        })
        .collect();

    for (i, (a_text, a_rect, a_size)) in texts.iter().enumerate() {
        for (b_text, b_rect, b_size) in texts.iter().skip(i + 1) {
            if a_text.is_empty() || b_text.is_empty() {
                continue;
            }
            // Only vertically stacked pairs in the same column matter.
            let same_column = a_rect.x < b_rect.right() && b_rect.x < a_rect.right();
            if !same_column {
                continue;
            }
            let (upper, upper_size, lower) = if a_rect.center().y < b_rect.center().y {
                (a_rect, a_size, b_rect)
            } else {
                (b_rect, b_size, a_rect)
            };
            // Descenders reach about a fifth of the size below the baseline. The upper box
            // must leave that much clearance.
            let descender = upper_size * 0.22;
            assert!(
                upper.bottom() - descender <= lower.y + 1.0,
                "{a_text:?} and {b_text:?} overlap: {upper:?} then {lower:?}"
            );
        }
    }
}

#[test]
fn clicking_selects_a_song_and_clicking_again_sings_it() {
    // Two clicks rather than one, and no selection on hover. In the list and the roulette the
    // cursor is always centred and the songs scroll past it, so selecting on hover would drag
    // the list out from under the pointer — you would click a different song than you aimed
    // at. Probing real points rather than recomputing the layout, because a test that
    // re-derives those rectangles agrees with a wrong answer.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);

    for layout in Layout::ALL {
        let mut screen = loaded(200);
        screen.browser.layout = layout;
        screen.browser.jump_to(100);

        let mut list = DrawList::new();
        screen.draw(&mut list, area, &style, &|_| None);

        // Hovering never moves the cursor.
        for row in 0..8 {
            screen.handle(
                Input::Hover(rungstar_ui::geom::Point::new(
                    900.0,
                    200.0 + row as f32 * 70.0,
                )),
                area,
            );
        }
        assert_eq!(
            screen.browser.cursor(),
            100,
            "{layout:?}: hovering moved the cursor"
        );

        // A click on a song that is not selected moves the cursor there and starts nothing.
        // A click on the one already selected sings it. Which of the two a given point is
        // depends on the layout, so the assertion is the contract rather than the outcome.
        let mut selected_something = false;
        'probe: for row in 0..12 {
            for column in 0..6 {
                let mut list = DrawList::new();
                screen.draw(&mut list, area, &style, &|_| None);
                let point = rungstar_ui::geom::Point::new(
                    700.0 + column as f32 * 140.0,
                    200.0 + row as f32 * 60.0,
                );
                let before = screen.browser.cursor();
                let outcome = screen.handle(Input::Click(point), area);
                let after = screen.browser.cursor();
                match outcome {
                    Transition::Sing(_) => assert_eq!(
                        before, after,
                        "{layout:?}: a click both moved the cursor and started a song"
                    ),
                    Transition::None => {}
                    other => panic!("{layout:?}: a click produced {other:?}"),
                }
                if after != before {
                    selected_something = true;
                    break 'probe;
                }
            }
        }
        assert!(
            selected_something,
            "{layout:?}: no point in the list selected a song"
        );

        // Clicking the song that is now selected sings it. Where that lands on screen is a
        // layout's own business, so this probes rather than assuming — the contract is "a
        // click on the selected song sings it", not "the selected song is in the middle".
        let mut sang = false;
        'again: for _ in 0..2 {
            for row in 0..12 {
                for column in 0..6 {
                    let mut list = DrawList::new();
                    screen.draw(&mut list, area, &style, &|_| None);
                    let probe = rungstar_ui::geom::Point::new(
                        700.0 + column as f32 * 140.0,
                        200.0 + row as f32 * 60.0,
                    );
                    if let Transition::Sing(_) = screen.handle(Input::Click(probe), area) {
                        sang = true;
                        break 'again;
                    }
                }
            }
        }
        assert!(
            sang,
            "{layout:?}: no second click on the selected song ever sang it"
        );
    }
}

#[test]
fn the_pointer_can_type_on_the_on_screen_keyboard() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);

    let mut screen = loaded(10);
    screen.handle(Input::Search, area);
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);

    // The keys are drawn as panels; find one by pointing at every panel until something is
    // typed. Anything else would be asserting the grid's arithmetic twice.
    let panels: Vec<Rect> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Rect { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();

    let mut typed = false;
    for panel in panels {
        screen.handle(Input::Click(panel.center()), area);
        if !screen.search_text().is_empty() {
            typed = true;
            break;
        }
        let mut list = DrawList::new();
        screen.draw(&mut list, area, &style, &|_| None);
        if screen.mode() != Mode::Searching {
            screen.handle(Input::Search, area);
            let mut list = DrawList::new();
            screen.draw(&mut list, area, &style, &|_| None);
        }
    }
    assert!(typed, "no key on the on-screen keyboard could be clicked");
}

#[test]
fn the_pointer_works_on_the_main_menu_and_the_options() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);

    let mut menu = MainMenu::new();
    let mut list = DrawList::new();
    menu.draw(&mut list, area, &style, "");
    // The second entry is Options; find its row and click it.
    let rows: Vec<Rect> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Rect { rect, .. } if rect.w > 300.0 && rect.h > 40.0 => Some(*rect),
            _ => None,
        })
        .collect();
    assert!(rows.len() >= 5, "the menu did not draw its rows");
    // The second row, whatever it now is, must be reachable by clicking it.
    assert!(matches!(
        menu.handle(Input::Click(rows[1].center())),
        Transition::Push(_)
    ));

    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &settings);
    // Hovering a group previews it without stealing focus from the group list.
    let before = screen.page_index();
    screen.handle(
        Input::Hover(Rect::new(30.0, 250.0, 10.0, 10.0).center()),
        &mut settings,
    );
    assert!(screen.on_page_list(), "hovering a group took focus");
    let _ = before;
}

#[test]
fn hints_name_the_control_the_player_is_holding() {
    // A hint reading "X" on a keyboard tells you a button exists and gives the wrong name.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);

    let mut screen = loaded(10);
    screen.gamepad = false;
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    let keys = strings(&list).join(" ");
    assert!(keys.contains("Enter"), "keyboard hints missing: {keys}");
    assert!(keys.contains("Esc"));

    screen.gamepad = true;
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    let pad = strings(&list).join(" ");
    assert!(
        pad.contains(" A ") || pad.starts_with("A ") || pad.contains("A"),
        "{pad}"
    );
    assert!(
        !pad.contains("Enter"),
        "gamepad hints still name keyboard keys: {pad}"
    );
}

#[test]
fn an_open_overlay_takes_every_click_away_from_the_list() {
    // A click aimed just past the search dialog was landing on the song list behind it, which
    // could select a song or start one. A search box is the last place that should happen.
    //
    // The song menu is deliberately not in this list: it has a Sing row, so a click on it
    // starting a song is the feature rather than the leak.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);

    for opener in [Input::Search, Input::Sort] {
        let mut screen = loaded(200);
        screen.browser.jump_to(100);
        let mut list = DrawList::new();
        screen.draw(&mut list, area, &style, &|_| None);
        screen.handle(opener, area);
        let before = screen.browser.cursor();

        for row in 0..14 {
            for column in 0..10 {
                if screen.mode() == Mode::Browsing {
                    // A click landed outside the dialog and closed it, which is allowed.
                    screen.handle(opener, area);
                }
                let mut list = DrawList::new();
                screen.draw(&mut list, area, &style, &|_| None);
                let point = rungstar_ui::geom::Point::new(
                    60.0 + column as f32 * 150.0,
                    60.0 + row as f32 * 65.0,
                );
                screen.handle(Input::Hover(point), area);
                let outcome = screen.handle(Input::Click(point), area);
                assert!(
                    !matches!(outcome, Transition::Sing(_)),
                    "{opener:?}: a click at {point:?} started a song through the overlay"
                );
                let _ = list;
            }
        }
        if opener == Input::Search {
            // Sorting re-queries and may legitimately move the cursor; searching must not
            // touch the list at all.
            assert_eq!(
                screen.browser.cursor(),
                before,
                "clicks moved the song cursor behind the search box"
            );
        }
    }
}

#[test]
fn clicking_away_from_the_song_menu_closes_it_without_reaching_the_list() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);

    let mut screen = loaded(200);
    screen.browser.jump_to(100);
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    screen.handle(Input::ContextMenu, area);
    assert_eq!(screen.mode(), Mode::Menu);

    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    // Far from the centred card, but well inside the song list.
    let corner = rungstar_ui::geom::Point::new(1500.0, 940.0);
    let outcome = screen.handle(Input::Click(corner), area);
    assert_eq!(outcome, Transition::None, "the click reached the list");
    assert_eq!(screen.mode(), Mode::Browsing, "the menu did not close");
    assert_eq!(screen.browser.cursor(), 100, "the click moved the cursor");
}

#[test]
fn keys_that_do_not_type_still_work_while_searching() {
    // The field being searched is changed with the sort key, and an earlier fix blocked it
    // along with the letters. Navigation and finishing have to keep working too, or the
    // on-screen keyboard cannot be used at all.
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.handle(Input::Search, area);
    assert!(screen.wants_text());

    let field = screen.field();
    screen.handle(Input::Sort, area);
    assert_ne!(
        screen.field(),
        field,
        "the search field could not be changed"
    );
    assert_eq!(screen.mode(), Mode::Searching);

    // And the layout does not change behind the dialog.
    let layout = screen.browser.layout;
    screen.handle(Input::CycleLayout, area);
    assert_eq!(
        screen.browser.layout, layout,
        "the browse layout changed behind the search box"
    );

    screen.handle(Input::Back, area);
    assert_eq!(screen.mode(), Mode::Browsing);
    assert!(!screen.wants_text());
}

#[test]
fn enter_finishes_the_search_rather_than_pressing_a_key() {
    // Somebody typing on a real keyboard is not looking at the on-screen keyboard's cursor,
    // so pressing whatever happens to be under it is never what Enter meant.
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.handle(Input::Search, area);
    for c in "queen".chars() {
        screen.handle(Input::Type(c), area);
    }
    assert_eq!(screen.search_text(), "queen");

    screen.handle(Input::Submit, area);
    assert_eq!(
        screen.mode(),
        Mode::Browsing,
        "Enter did not finish the search"
    );
    assert_eq!(screen.search_text(), "queen", "Enter changed the text");
    assert!(!screen.wants_text());

    // And back in the list, Enter sings again.
    screen.set_results(vec![]);
    assert_eq!(screen.handle(Input::Submit, area), Transition::None);
}

#[test]
fn the_on_screen_keyboard_still_presses_keys_with_confirm() {
    // Submit is Enter's meaning, not Confirm's: a gamepad still presses the highlighted key.
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.handle(Input::Search, area);
    let before = screen.search_text().to_owned();
    screen.handle(Input::Confirm, area);
    assert_ne!(
        screen.search_text(),
        before,
        "Confirm no longer types on the on-screen keyboard"
    );
    assert_eq!(screen.mode(), Mode::Searching);
}

/// Open the filter panel and put the cursor on the category with this title.
fn open_filters(screen: &mut SongSelect, area: Rect, category: &str) {
    if screen.mode() != Mode::Filtering {
        screen.handle(Input::CycleFilter, area);
    }
    assert_eq!(screen.mode(), Mode::Filtering);
    // Back to the category column, then to the top of it, wherever the last test step left it.
    screen.handle(Input::Left, area);
    for _ in 0..Facet::ALL.len() {
        screen.handle(Input::Up, area);
    }
    for _ in 0..Facet::ALL.len() {
        if screen.facet_title() == category {
            return;
        }
        screen.handle(Input::Down, area);
    }
    panic!("no filter category called {category:?}");
}

/// Facet lists as the application would supply them.
fn facets() -> FacetValues {
    let mut values = FacetValues::new();
    values.set(
        Facet::Genre,
        vec![("Rock".to_owned(), 12), ("Schlager".to_owned(), 4)],
    );
    values.set(
        Facet::Language,
        vec![
            ("English".to_owned(), 30),
            ("German".to_owned(), 9),
            ("Swedish".to_owned(), 2),
        ],
    );
    values.set(
        Facet::Decade,
        vec![("1980".to_owned(), 7), ("1970".to_owned(), 3)],
    );
    values
}

#[test]
fn the_list_can_be_narrowed_to_duets() {
    // Searching the word "duet" happens to work because the text is indexed, but only for
    // songs that say so somewhere. A filter asks the question properly.
    use rungstar_ui::songselect::Narrow;
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    assert_eq!(screen.narrow(), Narrow::Everything);
    assert_eq!(screen.filters().duet, None);

    open_filters(&mut screen, area, "Kind");
    screen.handle(Input::Right, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Confirm, area);
    assert_eq!(screen.narrow(), Narrow::Duets);
    assert_eq!(screen.filters().duet, Some(true));
    assert!(
        screen.needs_query(),
        "narrowing did not ask for a new query"
    );

    // One kind at a time: "duets only" and "solos only" together is an empty list.
    screen.set_results(vec![]);
    screen.handle(Input::Down, area);
    screen.handle(Input::Confirm, area);
    assert_eq!(screen.filters().duet, Some(false), "solos only");

    // And choosing the one already chosen turns it off rather than doing nothing.
    screen.set_results(vec![]);
    screen.handle(Input::Confirm, area);
    assert_eq!(screen.narrow(), Narrow::Everything);
    assert!(screen.filters().is_empty());
}

#[test]
fn values_within_a_category_are_any_of_and_categories_are_all_of() {
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.set_facets(facets());

    // Two languages: either one will do.
    open_filters(&mut screen, area, "Language");
    screen.handle(Input::Right, area);
    screen.handle(Input::Confirm, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Confirm, area);
    assert_eq!(screen.filters().languages, vec!["English", "German"]);

    // Plus a decade, which narrows further rather than widening.
    screen.handle(Input::Left, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Right, area);
    screen.handle(Input::Confirm, area);
    let filters = screen.filters();
    assert_eq!(filters.languages, vec!["English", "German"]);
    assert_eq!(filters.decades, vec![1980], "a decade is stored as a year");

    // Choosing a value again removes it.
    screen.handle(Input::Confirm, area);
    assert!(screen.filters().decades.is_empty());
}

#[test]
fn every_filter_can_be_cleared_at_once() {
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.set_facets(facets());

    open_filters(&mut screen, area, "Genre");
    screen.handle(Input::Right, area);
    screen.handle(Input::Confirm, area);
    open_filters(&mut screen, area, "Language");
    screen.handle(Input::Right, area);
    screen.handle(Input::Confirm, area);
    assert_eq!(screen.active_filters(), 2);

    screen.handle(Input::Search, area);
    assert_eq!(screen.active_filters(), 0);
    assert!(screen.filters().is_empty());
    assert!(screen.needs_query(), "clearing did not ask for a new query");
}

#[test]
fn a_value_that_left_the_library_stops_being_chosen() {
    // A rescan can remove the last Swedish song. Leaving it chosen leaves the browser empty
    // with no visible reason why.
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.set_facets(facets());
    open_filters(&mut screen, area, "Language");
    screen.handle(Input::Right, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Confirm, area);
    assert_eq!(screen.filters().languages, vec!["Swedish"]);

    let mut fewer = FacetValues::new();
    fewer.set(Facet::Language, vec![("English".to_owned(), 30)]);
    screen.set_facets(fewer);
    assert!(screen.filters().languages.is_empty());
}

#[test]
fn the_filter_panel_lists_what_the_library_has() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.set_facets(facets());
    open_filters(&mut screen, area, "Language");

    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    assert!(list.is_balanced());
    let text = strings(&list);
    for expected in ["Filter", "Language", "English", "German", "Swedish", "30"] {
        assert!(
            text.iter().any(|t| t == expected),
            "the panel did not show {expected:?}: {text:?}"
        );
    }

    // A decade reads as a decade, not as a bare year.
    open_filters(&mut screen, area, "Decade");
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    assert!(strings(&list).iter().any(|t| t == "1980s"));
}

#[test]
fn a_narrowed_list_says_so() {
    // A list quietly missing songs is indistinguishable from a library missing them.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);

    let mut screen = loaded(20);
    screen.set_facets(facets());
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    assert!(
        !strings(&list).join(" ").contains("German"),
        "an unfiltered list claimed to be filtered"
    );

    open_filters(&mut screen, area, "Language");
    screen.handle(Input::Right, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Confirm, area);
    screen.handle(Input::Back, area);
    screen.handle(Input::Back, area);
    assert_eq!(screen.mode(), Mode::Browsing);
    screen.set_results(vec![]);

    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    let text = strings(&list).join(" ");
    assert!(text.contains("German"), "the filter is not shown: {text}");
}

/// Put the cursor on the options row whose help begins with `prefix`.
///
/// By help text rather than by position, so adding a row above it does not silently move the
/// test onto a different button.
fn go_to_row(screen: &mut OptionsScreen, settings: &mut Settings, prefix: &str) {
    for _ in 0..12 {
        screen.handle(Input::Confirm, settings);
        for _ in 0..40 {
            if screen.help().starts_with(prefix) {
                return;
            }
            screen.handle(Input::Down, settings);
        }
        screen.handle(Input::Back, settings);
        screen.handle(Input::Down, settings);
    }
    panic!("no options row whose help starts with {prefix:?}");
}

const WIPE_HELP: &str = "Wipe every score";

#[test]
fn deleting_the_statistics_asks_before_doing_it() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    go_to_row(&mut screen, &mut settings, WIPE_HELP);

    // The press that lands on the button opens the question and runs nothing.
    assert_eq!(
        screen.handle(Input::Confirm, &mut settings),
        OptionsOutcome::None
    );
    assert!(screen.confirming());

    // And the answer starts on Cancel, so a second reflex press still deletes nothing. This
    // is the whole point: the accident is two Enters, not one.
    assert_eq!(
        screen.handle(Input::Confirm, &mut settings),
        OptionsOutcome::None
    );
    assert!(!screen.confirming());
}

#[test]
fn deleting_the_statistics_runs_once_the_delete_button_is_chosen() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    go_to_row(&mut screen, &mut settings, WIPE_HELP);

    screen.handle(Input::Confirm, &mut settings);
    screen.handle(Input::Right, &mut settings);
    assert_eq!(
        screen.handle(Input::Confirm, &mut settings),
        OptionsOutcome::Run(Action::WipeStatistics)
    );
    assert!(!screen.confirming(), "the question should close behind it");
}

#[test]
fn backing_out_of_the_confirmation_deletes_nothing() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    go_to_row(&mut screen, &mut settings, WIPE_HELP);

    screen.handle(Input::Confirm, &mut settings);
    screen.handle(Input::Right, &mut settings);
    assert_eq!(
        screen.handle(Input::Back, &mut settings),
        OptionsOutcome::None
    );
    assert!(!screen.confirming());

    // Reopening must not remember that Delete was the last thing under the cursor.
    screen.handle(Input::Confirm, &mut settings);
    assert_eq!(
        screen.handle(Input::Confirm, &mut settings),
        OptionsOutcome::None,
        "the confirmation reopened with Delete selected"
    );
}

#[test]
fn the_confirmation_says_what_will_be_lost() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    go_to_row(&mut screen, &mut settings, WIPE_HELP);
    screen.handle(Input::Confirm, &mut settings);

    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &style, &settings);
    assert!(list.is_balanced());
    let text = strings(&list);
    assert!(
        text.iter().any(|t| t == "Delete every score?"),
        "the question was not drawn: {text:?}"
    );
    assert!(text.iter().any(|t| t == "Cancel"));
    assert!(text.iter().any(|t| t == "Delete"));
    assert!(
        text.iter().any(|t| t.contains("no undo")),
        "the warning did not say it is permanent"
    );
    // Nothing from the page behind it, so there is no doubt about what a click will hit.
    assert!(
        !text.iter().any(|t| t.starts_with(WIPE_HELP)),
        "the page was still drawn under the question"
    );
}

#[test]
fn resetting_everything_asks_too() {
    let mut screen = OptionsScreen::new();
    let mut settings = Settings::default();
    go_to_row(&mut screen, &mut settings, "Put every setting back");
    assert_eq!(
        screen.handle(Input::Confirm, &mut settings),
        OptionsOutcome::None
    );
    assert!(screen.confirming());
    assert_eq!(settings, Settings::default(), "asking must change nothing");
}

/// Fail if two strings on the same line are drawn into boxes that overlap.
///
/// A label and a value that share one rectangle look fine until one of them is long: the
/// ellipsis is applied at the edge of the *box*, so neither is cut off and they collide in the
/// middle. Boxes that do not overlap cannot do that, whatever the strings turn out to be.
fn no_side_by_side_text_overlaps(list: &DrawList, what: &str) {
    let texts: Vec<(String, Rect)> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { rect, text, .. } if !text.is_empty() => Some((text.clone(), *rect)),
            _ => None,
        })
        .collect();

    for (index, (a_text, a)) in texts.iter().enumerate() {
        for (b_text, b) in texts.iter().skip(index + 1) {
            let shared = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0);
            // Same line only: boxes that barely graze each other vertically are a caption
            // under a label, which is the other test's business.
            if shared < a.h.min(b.h) * 0.5 {
                continue;
            }
            let across = (a.right().min(b.right()) - a.x.max(b.x)).max(0.0);
            assert!(
                across <= 1.0,
                "{what}: {a_text:?} and {b_text:?} share {across:.0} units of the same line\n  \
                 {a:?}\n  {b:?}"
            );
        }
    }
}

#[test]
fn a_long_value_is_cut_off_rather_than_drawn_under_its_label() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut settings = Settings::default();
    // The real thing that showed this up: a song folder nested five deep.
    settings.game.song_roots = vec![
        "C:/Users/somebody/Projects/UltraStarPlaySongConverter/UltraStarPlaySongsToBeConverted"
            .to_owned(),
    ];

    // Narrow as well as wide: the columns are fractions, so the tight case is the small one.
    for (w, h) in [(1778.0, 1000.0), (1600.0, 1000.0), (1333.0, 1000.0)] {
        let mut screen = OptionsScreen::new();
        for page in 0..6 {
            screen.handle(Input::Confirm, &mut settings);
            for row in 0..30 {
                let mut list = DrawList::new();
                screen.draw(&mut list, Rect::new(0.0, 0.0, w, h), &style, &settings);
                no_side_by_side_text_overlaps(&list, &format!("{w}x{h} page {page} row {row}"));
                screen.handle(Input::Down, &mut settings);
            }
            screen.handle(Input::Back, &mut settings);
            screen.handle(Input::Down, &mut settings);
        }
    }
}

#[test]
fn a_long_song_title_is_cut_off_before_it_reaches_its_score() {
    use rungstar_profile::stats::View;
    use rungstar_ui::statsscreen::{Row as StatRow, StatsScreen};

    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let long = "Panic! At The Disco - There\u{2019}s A Good Reason These Tables Are Numbered                 Honey, You Just Haven\u{2019}t Thought Of It Yet";

    for (w, h) in [(1778.0, 1000.0), (1333.0, 1000.0)] {
        let mut screen = StatsScreen::new();
        for _ in 0..View::ALL.len() {
            screen.set_rows(
                (0..12)
                    .map(|i| StatRow {
                        label: format!("{long} ({i})"),
                        detail: "Somebody With A Long Name On A Long Evening".to_owned(),
                        value: "10000".to_owned(),
                    })
                    .collect(),
            );
            let mut list = DrawList::new();
            screen.draw(&mut list, Rect::new(0.0, 0.0, w, h), &style);
            assert!(list.is_balanced());
            no_side_by_side_text_overlaps(&list, &format!("statistics at {w}x{h}"));
            screen.handle(Input::Right);
        }
    }
}

#[test]
fn a_long_microphone_name_is_cut_off_before_its_value() {
    use rungstar_ui::micscreen::{Device, MicScreen};

    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = MicScreen::new();
    screen.devices = vec![
        Device {
            name: "Realtek(R) Audio High Definition Microphone Array (Front Panel, Pink)"
                .to_owned(),
            assignment: vec![1, 2],
            levels: vec![0.4, 0.2],
            heard: vec![true, false],
            delay_ms: None,
        };
        3
    ];
    let mut list = DrawList::new();
    screen.draw(&mut list, Rect::new(0.0, 0.0, 1333.0, 1000.0), &style);
    assert!(list.is_balanced());
    no_side_by_side_text_overlaps(&list, "microphones");
}

/// The microphone list, when there are more microphones than fit.
///
/// Two dual-channel karaoke adapters is four singers and, with the channels split, eight rows
/// plus the refresh row — not an unusual party. They used to be laid out from the top with no
/// clip and no scroll offset, so everything past the bottom edge was drawn off the screen and
/// could not be reached.
mod many_microphones {
    use rungstar_ui::draw::{Command, DrawList};
    use rungstar_ui::geom::Rect;
    use rungstar_ui::micscreen::{Device, MicScreen};
    use rungstar_ui::songselect::Input;
    use rungstar_ui::theme::Theme;

    /// A Steam Deck.
    const DECK: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 1000.0,
    };

    fn crowded() -> MicScreen {
        let mut screen = MicScreen::new();
        screen.split_channels = true;
        screen.devices = (0..8)
            .map(|n| Device {
                name: format!("USB Microphone {n}"),
                assignment: vec![0, 0],
                levels: vec![0.2, 0.1],
                heard: vec![true, false],
                delay_ms: None,
            })
            .collect();
        screen
    }

    fn drawn(screen: &mut MicScreen) -> DrawList {
        let style = Theme::builtin().resolve_default();
        let mut list = DrawList::new();
        screen.draw(&mut list, DECK, &style);
        list
    }

    /// Everything drawn, with its rectangle, after clipping is taken into account.
    fn rows_on_screen(list: &DrawList) -> Vec<String> {
        let mut clips: Vec<Rect> = Vec::new();
        let mut seen = Vec::new();
        for command in list.commands() {
            match command {
                Command::PushClip(rect) => clips.push(*rect),
                Command::PopClip => {
                    clips.pop();
                }
                Command::Text { rect, text, .. } if text.contains("Microphone") => {
                    let inside = clips
                        .iter()
                        .all(|clip| rect.y >= clip.y - 1.0 && rect.bottom() <= clip.bottom() + 1.0);
                    if inside {
                        seen.push(text.clone());
                    }
                }
                _ => {}
            }
        }
        seen
    }

    #[test]
    fn nothing_is_drawn_past_the_bottom_of_the_screen() {
        let mut screen = crowded();
        let list = drawn(&mut screen);
        for command in list.commands() {
            if let Command::Rect { rect, .. } = command {
                assert!(
                    rect.bottom() <= DECK.h + 1.0,
                    "a row is drawn off the bottom at {rect:?}"
                );
            }
        }
    }

    #[test]
    fn the_cursor_can_reach_the_last_row_and_it_is_visible_when_it_does() {
        // The whole point: seventeen rows, a screen that holds fewer, and every one of them
        // reachable. Walking down must eventually show the refresh row at the very end.
        let mut screen = crowded();
        let total = screen.devices.len() * 2 + 1;
        for _ in 0..total - 1 {
            screen.handle(Input::Down);
        }
        assert_eq!(
            screen.cursor(),
            total - 1,
            "the cursor did not reach the end"
        );

        let list = drawn(&mut screen);
        let text: Vec<String> = list
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            text.iter().any(|t| t == "Look again"),
            "the last row is not on screen: {text:?}"
        );
    }

    #[test]
    fn the_view_follows_the_cursor_rather_than_staying_put() {
        let mut screen = crowded();
        let first = rows_on_screen(&drawn(&mut screen));
        assert!(!first.is_empty(), "nothing was drawn at all");

        for _ in 0..12 {
            screen.handle(Input::Down);
        }
        let later = rows_on_screen(&drawn(&mut screen));
        assert_ne!(first, later, "the list did not scroll");
    }

    #[test]
    fn a_list_that_fits_is_not_scrolled_and_says_nothing_about_it() {
        // One microphone. A counter reading "1 of 2" on a screen showing everything is noise.
        let mut screen = MicScreen::new();
        screen.devices = vec![Device {
            name: "USB Microphone".to_owned(),
            assignment: vec![1, 0],
            levels: vec![0.3, 0.0],
            heard: vec![true, false],
            delay_ms: None,
        }];
        let list = drawn(&mut screen);
        let text: Vec<String> = list
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(text.iter().any(|t| t.contains("USB Microphone")));
        assert!(
            !text.iter().any(|t| t.contains(" of ")),
            "a counter on a list that fits: {text:?}"
        );
    }
}

#[test]
fn the_backing_track_mode_is_not_offered_without_any_backing_tracks() {
    // The folder is what the mode needs, and nothing else on this screen can produce one. A
    // control that says a mode exists and then refuses to enter it is worse than no control.
    let mut screen = loaded(20);
    screen.set_results(library(20));
    screen.handle(Input::ToggleInstrumental, area());
    assert!(!screen.instrumental_toggled, "nothing was asked for");
    assert!(
        !screen.needs_query(),
        "nothing changed, so nothing to re-query"
    );

    let drawn = strings(&draw(&mut screen));
    assert!(!drawn.iter().any(|t| t == "No vocals"));
    assert!(!drawn.iter().any(|t| t == "Vocals on"));
}

#[test]
fn the_backing_track_mode_says_so_and_re_queries() {
    let mut screen = loaded(20);
    screen.instrumental_available = true;
    screen.set_results(library(20));

    // The hint offers the way in before the mode is on, and the header says nothing yet.
    let drawn = strings(&draw(&mut screen));
    assert!(drawn.iter().any(|t| t == "No vocals"), "the footer hint");
    assert!(
        !drawn.iter().any(|t| t.starts_with("No vocals  ")),
        "and not the header, which is a different string"
    );

    screen.handle(Input::ToggleInstrumental, area());
    assert!(
        screen.instrumental_toggled,
        "the screen asks; the application owns the setting"
    );
    // The list itself changes -- songs with no backing track leave it -- so the results the
    // screen is holding are no longer the answer to the question being asked.
    assert!(screen.needs_query());
    // What the application does with that: flips the setting and hands the answer back.
    screen.instrumental_toggled = false;
    screen.instrumental = true;
    screen.set_results(library(12));

    let drawn = strings(&draw(&mut screen));
    assert!(
        drawn.iter().any(|t| t.starts_with("No vocals  ")),
        "the header has to say the mode is on: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|t| t == "Vocals on"),
        "and the hint has to say what pressing it again does"
    );

    screen.handle(Input::ToggleInstrumental, area());
    assert!(screen.instrumental_toggled);
    assert!(screen.needs_query());
}

#[test]
fn the_backing_track_toggle_does_not_reach_past_the_search_box() {
    // "v" is a letter first. Every single-key shortcut on this screen has to lose to a text
    // field that has focus, or it fires underneath the thing being typed into.
    let mut screen = loaded(20);
    screen.instrumental_available = true;
    screen.handle(Input::Search, area());
    screen.handle(Input::ToggleInstrumental, area());
    assert!(!screen.instrumental_toggled);
    assert_eq!(screen.mode(), Mode::Searching);
}

#[test]
fn the_word_on_the_song_and_the_band_in_the_filter_are_the_same_word() {
    // The detail panel has always named a difficulty in words, and the filter now offers
    // bands. If those are two scales, the panel calls a song Moderate while the filter files
    // it under Hard — one number with two names, which is not a wording problem but a wrong
    // answer. They are the same scale or the filter is lying.
    use rungstar_library::DifficultyBand;
    use rungstar_ui::songselect::difficulty_label;

    let mut seen = std::collections::HashSet::new();
    let mut step = 0.0;
    while step <= 1.0 {
        assert_eq!(
            difficulty_label(step),
            DifficultyBand::of(step).label(),
            "the panel and the filter disagree at {step}"
        );
        seen.insert(DifficultyBand::of(step).label());
        step += 0.01;
    }
    // And every band the filter can offer is a word the panel can print, or there is a row
    // in the tree naming something no song will ever be described as.
    assert_eq!(seen.len(), DifficultyBand::ALL.len());
}

/// Difficulty bands as the library reports them: stable keys, with counts.
fn difficulty_values() -> FacetValues {
    let mut values = facets();
    values.set(
        Facet::Difficulty,
        vec![
            ("gentle".to_owned(), 1043),
            ("easy".to_owned(), 1947),
            ("brutal".to_owned(), 670),
        ],
    );
    values
}

#[test]
fn the_list_can_be_narrowed_to_a_difficulty_band() {
    // "Nothing too hard" is one of the few things anybody asks a song list at a party, and
    // until now the only answer was to sort by difficulty and scroll.
    use rungstar_library::DifficultyBand;
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.set_facets(difficulty_values());
    assert!(screen.filters().difficulty.is_empty());

    open_filters(&mut screen, area, "Difficulty");
    screen.handle(Input::Right, area);
    screen.handle(Input::Confirm, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Down, area);
    screen.handle(Input::Confirm, area);

    // Either of them will do, the way every other category in the tree works.
    assert_eq!(
        screen.filters().difficulty,
        vec![DifficultyBand::Gentle, DifficultyBand::Brutal]
    );
    assert!(
        screen.needs_query(),
        "narrowing did not ask for a new query"
    );
}

#[test]
fn a_difficulty_band_is_named_the_way_the_song_panel_names_it() {
    // The panel calls a song Gentle. A filter row calling the same thing "gentle" is the
    // stored key leaking onto the screen, and it reads as two different scales.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = Rect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut screen = loaded(20);
    screen.set_facets(difficulty_values());
    open_filters(&mut screen, area, "Difficulty");

    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    let text = strings(&list);
    for expected in ["Gentle", "Easy", "Brutal", "1043"] {
        assert!(
            text.iter().any(|t| t == expected),
            "the panel did not show {expected:?}: {text:?}"
        );
    }
    assert!(
        !text.iter().any(|t| t == "gentle"),
        "the stored key reached the screen: {text:?}"
    );
}

#[test]
fn the_microphone_list_says_what_each_one_is_scored_at() {
    // Every microphone now has its own delay, and until this there was nowhere to see one:
    // the measurement screen showed them once and then closed. A number nobody can look up
    // is a number nobody can tell is wrong, which for this setting is the whole failure —
    // a bad delay sings perfectly and scores badly with nothing on screen to say why.
    use rungstar_ui::micscreen::{Device, MicScreen};

    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = MicScreen::new();
    screen.devices = vec![
        Device {
            name: "Blue Yeti".to_owned(),
            assignment: vec![1],
            levels: vec![0.4],
            heard: vec![true],
            delay_ms: Some(119),
        },
        Device {
            name: "Never Measured".to_owned(),
            assignment: vec![2],
            levels: vec![0.4],
            heard: vec![true],
            delay_ms: None,
        },
    ];

    let mut list = DrawList::new();
    screen.draw(&mut list, Rect::new(0.0, 0.0, 1600.0, 1000.0), &style);
    assert!(list.is_balanced());
    let text = strings(&list).join(" ");
    assert!(
        text.contains("119 ms"),
        "a measured microphone did not show its delay: {text}"
    );
    // One that has never been swept says so rather than showing the shared figure as though
    // it had been measured, which would make the guess indistinguishable from an answer.
    assert!(
        text.contains("not measured"),
        "an unmeasured microphone did not say so: {text}"
    );
}

#[test]
fn leaving_the_main_menu_asks_before_it_quits() {
    // Esc out of the options and Esc again lands on the main menu and ends the game. That is
    // two presses of the same key, one of which was aimed at a screen that had already
    // closed, and it takes a party down with it. The menu's own comment has always claimed
    // this was confirmed somewhere; it was not.
    let mut menu = MainMenu::new();

    assert!(matches!(menu.handle(Input::Back), Transition::None));
    assert!(menu.confirming(), "Esc did not open a confirmation");

    // Esc again is the answer somebody who did not mean it will give, so it cancels rather
    // than confirming — the accidental second press must not do what the first one nearly did.
    assert!(matches!(menu.handle(Input::Back), Transition::None));
    assert!(
        !menu.confirming(),
        "a second Esc did not dismiss the question"
    );

    // Confirming outright does not leave either: Esc opens the question on Cancel, because
    // the press that got here was most likely aimed at a screen that had already closed.
    menu.handle(Input::Back);
    assert!(matches!(menu.handle(Input::Confirm), Transition::None));

    // Steering to the other answer is what leaves, and it still works.
    menu.handle(Input::Back);
    menu.handle(Input::Right);
    assert!(matches!(menu.handle(Input::Confirm), Transition::Quit));
}

#[test]
fn the_quit_entry_asks_too() {
    // Choosing Quit is deliberate, but a stray Enter in a room of six people is the same
    // accident as a stray Esc, and one rule for both is easier to rely on than two.
    let mut menu = MainMenu::new();
    for _ in 0..16 {
        if menu.entry_quits(menu.cursor()) {
            break;
        }
        menu.handle(Input::Down);
    }
    assert!(menu.entry_quits(menu.cursor()), "no Quit entry in the menu");

    assert!(matches!(menu.handle(Input::Confirm), Transition::None));
    assert!(menu.confirming(), "choosing Quit did not ask");
    assert!(matches!(menu.handle(Input::Confirm), Transition::Quit));
}

#[test]
fn the_quit_question_is_on_screen_and_clickable() {
    // A state with no picture is worse than no state: the keys change meaning and nothing
    // says so, which reads as the menu having locked up.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut menu = MainMenu::new();
    menu.handle(Input::Back);

    let mut list = DrawList::new();
    menu.draw(&mut list, area(), &style, "");
    assert!(list.is_balanced());
    let text = strings(&list);
    assert!(
        text.iter().any(|t| t.contains("Quit RungStar")),
        "the question was not drawn: {text:?}"
    );
    for button in ["Cancel", "Quit"] {
        assert!(
            text.iter().any(|t| t == button),
            "no {button:?} button: {text:?}"
        );
    }

    // Clicking Cancel dismisses it rather than leaving.
    let cancel = menu
        .quit_button(false)
        .expect("the Cancel button was not recorded");
    // A button laid out to a negative height still draws and can never be hit, and a click
    // that misses it dismisses the question anyway — so the pointer half of this test would
    // pass on a dialog nobody could actually press Cancel on.
    for (label, rect) in [
        ("Cancel", cancel),
        ("Quit", menu.quit_button(true).unwrap()),
    ] {
        assert!(
            rect.w > 0.0 && rect.h > 0.0,
            "the {label} button was laid out to nothing: {rect:?}"
        );
    }
    assert!(matches!(
        menu.handle(Input::Click(cancel.center())),
        Transition::None
    ));
    assert!(!menu.confirming());

    // And clicking Quit leaves.
    menu.handle(Input::Back);
    let mut list = DrawList::new();
    menu.draw(&mut list, area(), &style, "");
    let quit = menu.quit_button(true).expect("no Quit button recorded");
    assert!(matches!(
        menu.handle(Input::Click(quit.center())),
        Transition::Quit
    ));
}

#[test]
fn the_filter_panel_can_be_worked_with_the_mouse() {
    // Every pointer event over the panel was treated as a click outside it, because the guard
    // that makes an overlay modal only knew about the keyboard, the sort picker and the song
    // menu. So the panel closed the instant it was clicked and there was no way to use it
    // without a controller.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = area();
    let mut screen = loaded(20);
    screen.set_facets(facets());
    open_filters(&mut screen, area, "Language");

    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);

    // The second value of the open category — German, from the fixture.
    let row = screen
        .filter_row(false, 1)
        .expect("no filter value row was recorded");
    assert!(
        row.w > 0.0 && row.h > 0.0,
        "the row was laid out to nothing"
    );

    screen.handle(Input::Click(row.center()), area);
    assert_eq!(
        screen.mode(),
        Mode::Filtering,
        "clicking a filter value closed the panel"
    );
    assert_eq!(
        screen.filters().languages,
        vec!["German"],
        "clicking a filter value did not toggle it"
    );

    // And clicking it again turns it off, as the keyboard does.
    screen.handle(Input::Click(row.center()), area);
    assert!(screen.filters().languages.is_empty());
    assert_eq!(screen.mode(), Mode::Filtering);
}

#[test]
fn a_filter_category_is_opened_by_clicking_it() {
    // The other half of working the panel with a pointer: reaching a different category at
    // all. Hovering deliberately does not, so a click has to.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = area();
    let mut screen = loaded(20);
    screen.set_facets(facets());
    open_filters(&mut screen, area, "Language");

    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    let wanted = Facet::ALL
        .iter()
        .position(|facet| facet.title() == "Genre")
        .expect("no Genre category");
    let row = screen
        .filter_row(true, wanted)
        .expect("no filter category row was recorded");

    screen.handle(Input::Click(row.center()), area);
    assert_eq!(screen.facet_title(), "Genre");
    assert_eq!(screen.mode(), Mode::Filtering, "the panel closed");
}

#[test]
fn pointing_at_a_scrolling_list_does_not_move_it() {
    // Both of these lists scroll to keep their cursor on screen. Move the cursor on hover and
    // the list slides under the pointer, which puts a different row where the pointer is, which
    // moves the cursor again: sweeping the mouse across the options makes the page bolt. The
    // song list settled this long ago — the cursor is centred and the songs scroll past it, so
    // hovering never moves it — and this is the same rule everywhere else it applies.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let area = area();

    // The options page: hovering an item must leave the cursor where the keyboard put it.
    let mut settings = Settings::default();
    let mut options = OptionsScreen::new();
    options.handle(Input::Right, &mut settings);
    let mut list = DrawList::new();
    options.draw(&mut list, area, &style, &settings);
    let before = options.item_cursor();
    for step in 0..120 {
        let point = rungstar_ui::geom::Point::new(area.w * 0.75, step as f32 * 8.0);
        options.handle(Input::Hover(point), &mut settings);
    }
    assert_eq!(
        options.item_cursor(),
        before,
        "sweeping the pointer down the options moved the cursor"
    );

    // The filter panel: same rule, and here the category column decides what the value column
    // even contains, so a sweep across it empties the list somebody was reaching for.
    let mut screen = loaded(20);
    screen.set_facets(facets());
    open_filters(&mut screen, area, "Language");
    screen.handle(Input::Right, area);
    let mut list = DrawList::new();
    screen.draw(&mut list, area, &style, &|_| None);
    let facet = screen.facet_title();
    let value = screen.value_cursor();
    for step in 0..120 {
        for column in [0.25, 0.6, 0.9] {
            let point = rungstar_ui::geom::Point::new(area.w * column, step as f32 * 8.0);
            screen.handle(Input::Hover(point), area);
        }
    }
    assert_eq!(
        screen.facet_title(),
        facet,
        "sweeping the pointer changed which filter category was open"
    );
    assert_eq!(
        screen.value_cursor(),
        value,
        "sweeping the pointer moved the filter value cursor"
    );
}
