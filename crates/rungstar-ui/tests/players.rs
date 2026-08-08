//! The singer screen, in both of its jobs: managing profiles, and asking who is about to sing.

use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::Rect;
use rungstar_ui::playerscreen::{Entry, PlayerOutcome, PlayerScreen};
use rungstar_ui::screen::Transition;
use rungstar_ui::songselect::Input;
use rungstar_ui::theme::Theme;

fn area() -> Rect {
    Rect::new(0.0, 0.0, 1600.0, 1000.0)
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

fn entry(id: i64, name: &str) -> Entry {
    Entry {
        id,
        name: name.into(),
        colour: (id as u8) % 6,
        songs: 3,
        best: 7000,
    }
}

fn picker(microphones: usize) -> PlayerScreen {
    let mut screen = PlayerScreen::new();
    screen.players = vec![entry(1, "Ada"), entry(2, "Grace"), entry(3, "Kata")];
    screen.microphones = microphones;
    screen.for_song = Some("Abba - Waterloo".to_owned());
    screen
}

#[test]
fn a_song_can_be_started_from_the_picker() {
    let mut screen = picker(2);
    // Down past the three profiles and the Add row.
    for _ in 0..4 {
        screen.handle(Input::Down);
    }
    let (transition, outcome) = screen.handle(Input::Confirm);
    assert_eq!(outcome, PlayerOutcome::Start);
    assert_eq!(transition, Transition::None);
}

#[test]
fn the_managing_screen_has_no_start_row() {
    let mut screen = picker(2);
    screen.for_song = None;
    // Wrapping all the way round must never produce a Start: there is no song to start.
    for _ in 0..10 {
        let (_, outcome) = screen.handle(Input::Confirm);
        assert_ne!(outcome, PlayerOutcome::Start);
        screen.handle(Input::Down);
    }
}

#[test]
fn choosing_singers_is_capped_by_the_microphones() {
    let mut screen = picker(2);
    for row in 0..3 {
        screen.handle(Input::Confirm);
        screen.handle(Input::Down);
        let _ = row;
    }
    assert_eq!(
        screen.singers.len(),
        2,
        "three profiles were chosen for two microphones"
    );
    // The last press replaced rather than did nothing, so the newest choice is honoured.
    assert_eq!(screen.singers, vec![1, 3]);
}

#[test]
fn the_picker_names_the_song_and_the_duet_parts() {
    let mut screen = picker(2);
    screen.duet = Some(("Sonny".to_owned(), "Cher".to_owned()));
    screen.singers = vec![1, 2];
    let theme = Theme::builtin();
    let style = theme.resolve_default();

    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &style);
    assert!(list.is_balanced());
    let text = strings(&list);
    assert!(
        text.iter().any(|t| t == "Abba - Waterloo"),
        "the song being started was not named: {text:?}"
    );
    assert!(
        text.iter()
            .any(|t| t.contains("Sonny") && t.contains("Cher")),
        "the duet parts were not named: {text:?}"
    );
}

#[test]
fn a_duet_with_one_singer_says_so_rather_than_refusing() {
    let mut screen = picker(2);
    screen.duet = Some(("Sonny".to_owned(), "Cher".to_owned()));
    screen.singers = vec![1];
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &style);

    let text = strings(&list);
    assert!(
        text.iter().any(|t| t.contains("two singers")),
        "a one-singer duet gave no hint: {text:?}"
    );

    // And it still starts. Singing both parts alone is allowed; it is just harder.
    for _ in 0..4 {
        screen.handle(Input::Down);
    }
    assert_eq!(screen.handle(Input::Confirm).1, PlayerOutcome::Start);
}

#[test]
fn the_start_row_can_be_clicked() {
    let mut screen = picker(2);
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &style);

    // Whatever the Start row's rectangle is, moving to it and clicking must start the song.
    for _ in 0..4 {
        screen.handle(Input::Down);
    }
    let mut probe = None;
    for y in 0..1000 {
        let point = rungstar_ui::geom::Point::new(800.0, y as f32);
        let mut copy = picker(2);
        copy.draw(&mut DrawList::new(), area(), &style);
        if copy.handle(Input::Click(point)).1 == PlayerOutcome::Start {
            probe = Some(y);
            break;
        }
    }
    assert!(probe.is_some(), "no row on the screen starts the song");
}

#[test]
fn the_picker_stays_inside_the_window() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    for (w, h) in [(1600.0, 1000.0), (1280.0, 800.0), (1000.0, 1000.0)] {
        let bounds = Rect::new(0.0, 0.0, w, h);
        let mut screen = picker(3);
        screen.duet = Some(("One".to_owned(), "Two".to_owned()));
        let mut list = DrawList::new();
        screen.draw(&mut list, bounds, &style);
        for command in list.commands() {
            if let Command::Rect { rect, .. } = command {
                assert!(
                    rect.bottom() <= bounds.bottom() + 1.0 && rect.right() <= bounds.right() + 1.0,
                    "a panel left the {w}x{h} window: {rect:?}"
                );
            }
        }
    }
}
