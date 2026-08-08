//! The sing screen. Driven by state and read back as commands, like every other screen.

use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::Rect;
use rungstar_ui::screen::Transition;
use rungstar_ui::singscreen::{
    rating_title, Note, NoteKind, Overlay, PauseChoice, SingScreen, Singer, Sung, Syllable,
};
use rungstar_ui::songselect::Input;
use rungstar_ui::theme::Theme;

fn area() -> Rect {
    Rect::new(0.0, 0.0, 1600.0, 1000.0)
}

fn notes() -> Vec<Note> {
    (0..40)
        .map(|i| Note {
            start: i as f64 * 4.0,
            duration: 3.0,
            pitch: 60 + (i % 7),
            kind: match i % 6 {
                0 => NoteKind::Golden,
                3 => NoteKind::Freestyle,
                _ => NoteKind::Normal,
            },
        })
        .collect()
}

fn syllables() -> Vec<Syllable> {
    ["Ne", "ver ", "gon", "na "]
        .iter()
        .enumerate()
        .map(|(i, text)| Syllable {
            text: (*text).to_owned(),
            start: 8.0 + i as f64 * 2.0,
            duration: 2.0,
            golden: i == 0,
        })
        .collect()
}

fn draw(screen: &SingScreen, beat: f64) -> DrawList {
    let theme = Theme::builtin();
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &theme.resolve_default(),
        &notes(),
        &syllables(),
        "the next line",
        beat,
    );
    list
}

fn strings(list: &DrawList) -> Vec<String> {
    list.commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn one_singer_and_six_are_the_same_code() {
    // The whole point of the screen taking a slice: UltraStar hand-places every player count
    // in every theme, which is why adding a sixth singer there is a layout change.
    for count in 1..=6 {
        let screen = SingScreen::new("Artist", "Title", count);
        assert_eq!(screen.singers.len(), count);
        let list = draw(&screen, 10.0);
        assert!(list.is_balanced(), "{count} singers left a clip pushed");
        assert!(!list.is_empty());
    }
}

#[test]
fn every_singer_gets_a_panel_and_a_distinct_colour() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let screen = SingScreen::new("Artist", "Title", 6);
    let list = draw(&screen, 10.0);
    let text = strings(&list);
    for player in 1..=6 {
        assert!(
            text.iter().any(|t| t == &format!("Player {player}")),
            "player {player} has no panel: {text:?}"
        );
    }
    // And the colours they are drawn in are all different, or the panels are unreadable as a
    // group.
    for i in 0..6 {
        for j in (i + 1)..6 {
            assert_ne!(style.player(i), style.player(j));
        }
    }
}

#[test]
fn the_screen_stays_inside_the_window_at_every_size_and_player_count() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    for (name, area) in [
        ("deck", Rect::new(0.0, 0.0, 1600.0, 1000.0)),
        ("1080p", Rect::new(0.0, 0.0, 1778.0, 1000.0)),
        ("4:3", Rect::new(0.0, 0.0, 1333.0, 1000.0)),
        ("ultrawide", Rect::new(0.0, 0.0, 2389.0, 1000.0)),
    ] {
        for count in 1..=6 {
            for overlay in [Overlay::None, Overlay::Paused, Overlay::Results] {
                let mut screen = SingScreen::new("Artist", "Title", count);
                screen.overlay = overlay;
                screen.show_input_panel = true;
                let mut list = DrawList::new();
                screen.draw(
                    &mut list,
                    area,
                    &style,
                    &notes(),
                    &syllables(),
                    "next",
                    10.0,
                );

                let mut depth = 0;
                for command in list.commands() {
                    match command {
                        Command::PushClip(_) => depth += 1,
                        Command::PopClip => depth -= 1,
                        _ if depth > 0 => {}
                        Command::Rect { rect, .. }
                        | Command::Outline { rect, .. }
                        | Command::Text { rect, .. }
                        | Command::Image { rect, .. } => assert!(
                            rect.x >= area.x - 1.0
                                && rect.y >= area.y - 1.0
                                && rect.right() <= area.right() + 1.0
                                && rect.bottom() <= area.bottom() + 1.0,
                            "{name} {count} singers {overlay:?}: {command:?} is outside the window"
                        ),
                        _ => {}
                    }
                }
                assert!(list.is_balanced());
            }
        }
    }
}

#[test]
fn back_pauses_rather_than_quitting() {
    // Leaving a song by accident in the middle of a party is worse than one extra press.
    let mut screen = SingScreen::new("Artist", "Title", 2);
    let (transition, choice) = screen.handle(Input::Back);
    assert_eq!(transition, Transition::None);
    assert_eq!(choice, None);
    assert_eq!(screen.overlay, Overlay::Paused);
}

#[test]
fn the_pause_menu_offers_a_way_out_and_a_way_back() {
    let mut screen = SingScreen::new("Artist", "Title", 1);
    screen.handle(Input::Back);

    // Continue is first, so the most likely choice needs no navigation.
    let (_, choice) = screen.handle(Input::Confirm);
    assert_eq!(choice, Some(PauseChoice::Continue));
    assert_eq!(screen.overlay, Overlay::None);

    let mut screen = SingScreen::new("Artist", "Title", 1);
    screen.handle(Input::Back);
    screen.handle(Input::Down);
    let (_, choice) = screen.handle(Input::Confirm);
    assert_eq!(choice, Some(PauseChoice::Restart));

    // Up from the first entry wraps to the last.
    let mut screen = SingScreen::new("Artist", "Title", 1);
    screen.handle(Input::Back);
    screen.handle(Input::Up);
    let (_, choice) = screen.handle(Input::Confirm);
    assert_eq!(choice, Some(PauseChoice::Quit), "the cursor did not wrap");
}

#[test]
fn escape_from_the_pause_menu_resumes() {
    let mut screen = SingScreen::new("Artist", "Title", 1);
    screen.handle(Input::Back);
    let (_, choice) = screen.handle(Input::Back);
    assert_eq!(choice, Some(PauseChoice::Continue));
    assert_eq!(screen.overlay, Overlay::None);
}

#[test]
fn the_results_stay_up_until_dismissed() {
    // In a party the result is the point; popping straight back to the browser throws it away
    // before anybody has read it.
    let mut screen = SingScreen::new("Artist", "Title", 3);
    screen.overlay = Overlay::Results;
    screen.singers[0].score = 7200;
    screen.singers[1].score = 9100;
    screen.singers[2].score = 3000;

    let list = draw(&screen, 200.0);
    let text = strings(&list).join(" ");
    // Ranked, because in a party the order is the whole point.
    assert!(text.contains("9100") && text.contains("7200") && text.contains("3000"));
    assert!(text.contains("Ultrastar"), "no rating shown: {text}");
    assert!(
        text.contains("Rising Star"),
        "only the winner was rated: {text}"
    );

    assert_eq!(screen.handle(Input::Up).0, Transition::None);
    assert_eq!(screen.handle(Input::Confirm).0, Transition::Pop);
}

#[test]
fn the_ratings_match_ultrastars_tiers() {
    // Scores are comparable with another install only if the words are too.
    assert_eq!(rating_title(0), "Tone Deaf");
    assert_eq!(rating_title(2009), "Tone Deaf");
    assert_eq!(rating_title(2010), "Amateur");
    assert_eq!(rating_title(4010), "Wannabe");
    assert_eq!(rating_title(5010), "Hopeful");
    assert_eq!(rating_title(6010), "Rising Star");
    assert_eq!(rating_title(7510), "Lead Singer");
    assert_eq!(rating_title(8510), "Superstar");
    assert_eq!(rating_title(9010), "Ultrastar");
    assert_eq!(rating_title(10000), "Ultrastar");
}

#[test]
fn the_three_input_failures_are_told_apart() {
    // They fail independently, and the first version of this screen made a dead microphone
    // and a silent room look identical: both drew nothing at all.
    let mut singer = Singer::new("P1");
    assert_eq!(singer.input_problem(), Some("no microphone"));

    singer.has_microphone = true;
    assert_eq!(singer.input_problem(), Some("no audio arriving"));

    singer.ever_heard = true;
    singer.gate = 0.1;
    singer.level = 0.02;
    assert_eq!(singer.input_problem(), Some("too quiet to score"));

    singer.level = 0.4;
    assert_eq!(singer.input_problem(), None);
}

#[test]
fn a_broken_microphone_is_reported_even_with_the_panel_off() {
    // The panel is off by default, but silence that is not being scored has to say so or it
    // looks like the game is ignoring you.
    let mut screen = SingScreen::new("Artist", "Title", 1);
    screen.show_input_panel = false;
    screen.singers[0].has_microphone = false;

    let text = strings(&draw(&screen, 10.0)).join(" ");
    assert!(text.contains("no microphone"), "not reported: {text}");
}

#[test]
fn a_working_microphone_stays_quiet_unless_asked() {
    let mut screen = SingScreen::new("Artist", "Title", 1);
    screen.show_input_panel = false;
    screen.singers[0].has_microphone = true;
    screen.singers[0].ever_heard = true;
    screen.singers[0].gate = 0.1;
    screen.singers[0].level = 0.5;
    screen.singers[0].pitch = Some(62);

    let text = strings(&draw(&screen, 10.0)).join(" ");
    assert!(
        !text.contains("listening"),
        "the panel drew uninvited: {text}"
    );

    screen.show_input_panel = true;
    let text = strings(&draw(&screen, 10.0)).join(" ");
    assert!(text.contains('D'), "the detected note is not shown: {text}");
}

#[test]
fn the_lyric_being_sung_is_drawn_differently_from_the_rest() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    // Beat 11 is inside the second syllable, which starts at 10 and lasts 2.
    let screen = SingScreen::new("Artist", "Title", 1);
    let list = draw(&screen, 11.0);

    let colours: Vec<(String, rungstar_ui::Color)> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { text, style, .. } => Some((text.clone(), style.color)),
            _ => None,
        })
        .collect();

    let active = colours.iter().find(|(t, _)| t == "ver ").map(|(_, c)| *c);
    let upcoming = colours.iter().find(|(t, _)| t == "na ").map(|(_, c)| *c);
    assert_eq!(
        active,
        Some(style.accent),
        "the active syllable is not highlighted"
    );
    assert_ne!(active, upcoming, "sung and unsung syllables look the same");
}

#[test]
fn lyrics_are_outlined_so_they_survive_a_background() {
    // Lyrics sit over artwork and video, where an outline is the difference between readable
    // and not.
    let screen = SingScreen::new("Artist", "Title", 1);
    let list = draw(&screen, 11.0);
    let outlined = list.commands().iter().any(|c| match c {
        Command::Text { text, style, .. } => text == "ver " && style.outline.is_some(),
        _ => false,
    });
    assert!(outlined, "the lyrics have no outline");
}

#[test]
fn what_was_sung_is_drawn_over_the_notes() {
    let mut screen = SingScreen::new("Artist", "Title", 1);
    let plain = draw(&screen, 10.0).len();

    screen.singers[0].sung = vec![
        Sung {
            start: 8.0,
            duration: 3.0,
            pitch: 62,
            hit: true,
        },
        Sung {
            start: 12.0,
            duration: 2.0,
            pitch: 59,
            hit: false,
        },
    ];
    let with_sung = draw(&screen, 10.0).len();
    assert!(
        with_sung > plain,
        "what was sung was not drawn: {plain} then {with_sung}"
    );
}

#[test]
fn a_song_with_no_notes_still_draws() {
    // Real libraries contain them, and a screen that panics on one is worse than a blank one.
    let theme = Theme::builtin();
    let screen = SingScreen::new("Artist", "Title", 2);
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &theme.resolve_default(),
        &[],
        &[],
        "",
        0.0,
    );
    assert!(list.is_balanced());
    assert!(!list.is_empty());
}

#[test]
fn the_progress_bar_only_appears_once_the_length_is_known() {
    let mut screen = SingScreen::new("Artist", "Title", 1);
    screen.duration = 0.0;
    let without = draw(&screen, 10.0).len();

    screen.duration = 200.0;
    screen.position = 100.0;
    let with = draw(&screen, 10.0).len();
    assert!(with > without, "no progress bar was drawn");
}
