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
use rungstar_ui::songselect::{Input, Mode, SongSelect};
use rungstar_ui::theme::Theme;
use rungstar_ui::Layout;

fn area() -> Rect {
    Rect::new(0.0, 0.0, 1600.0, 1000.0)
}

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
        difficulty: 0.5,
        medley_start: None,
        medley_end: None,
        preview_start: None,
        usdb_id: None,
        times_played: 0,
        last_played: None,
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
    assert!(text.iter().any(|t| t == "Moderate"), "difficulty missing");
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
    assert_eq!(
        menu.handle(Input::Confirm),
        Transition::Push(Route::SongSelect)
    );
    menu.handle(Input::Down);
    assert_eq!(
        menu.handle(Input::Confirm),
        Transition::Push(Route::Options)
    );
    menu.handle(Input::Down);
    assert_eq!(menu.handle(Input::Confirm), Transition::Push(Route::About));
    menu.handle(Input::Down);
    assert_eq!(menu.handle(Input::Confirm), Transition::Quit);
    // And it wraps, so the last entry is not a dead end.
    menu.handle(Input::Down);
    assert_eq!(menu.cursor(), 0);
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
