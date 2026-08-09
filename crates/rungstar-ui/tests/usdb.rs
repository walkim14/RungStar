//! The in-game USDB browser.

use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::Rect;
use rungstar_ui::songselect::Input;
use rungstar_ui::theme::Theme;
use rungstar_ui::usdbscreen::{Activity, Local, Mode, Row, UsdbOutcome, UsdbScreen};
use rungstar_usdb::SongId;

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

fn draw(screen: &mut UsdbScreen) -> DrawList {
    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &Theme::builtin().resolve_default());
    assert!(list.is_balanced());
    list
}

fn row(id: i64, artist: &str, title: &str, local: Local) -> Row {
    Row {
        id: SongId(id),
        artist: artist.into(),
        title: title.into(),
        language: "English".into(),
        year: Some(1974),
        rating: 4.5,
        golden: true,
        local,
    }
}

fn loaded() -> UsdbScreen {
    let mut screen = UsdbScreen::new();
    screen.catalog_size = 3;
    screen.set_rows(vec![
        row(1, "Abba", "Waterloo", Local::Absent),
        row(2, "Blur", "Song 2", Local::Held),
        row(3, "Nena", "99 Luftballons", Local::Stale),
    ]);
    screen
}

#[test]
fn an_empty_catalog_says_how_to_fill_it() {
    // A first run reaches this screen before there is anything on it, and "no songs" with no
    // explanation is where somebody closes the game.
    let mut screen = UsdbScreen::new();
    let text = strings(&draw(&mut screen)).join(" ");
    assert!(text.contains("No catalog yet"), "{text}");
    assert!(text.contains("Sync"), "it does not say what to do");
}

#[test]
fn a_song_already_in_the_library_is_marked_and_not_downloaded_again() {
    let mut screen = loaded();
    let text = strings(&draw(&mut screen));
    assert!(text.iter().any(|t| t == "in library"));
    assert!(text.iter().any(|t| t == "updated"), "the stale one");

    // The first row is absent, so confirming fetches it.
    assert_eq!(
        screen.handle(Input::Confirm).1,
        UsdbOutcome::Download(SongId(1))
    );
    // The second is held, so confirming does nothing rather than fetching it again — a button
    // that looks like it did nothing is worse than one that is not offered.
    screen.handle(Input::Down);
    assert_eq!(screen.handle(Input::Confirm).1, UsdbOutcome::None);
    // The third is stale, so it can be fetched again.
    screen.handle(Input::Down);
    assert_eq!(
        screen.handle(Input::Confirm).1,
        UsdbOutcome::Download(SongId(3))
    );
}

#[test]
fn a_song_being_fetched_is_not_asked_for_twice() {
    let mut screen = UsdbScreen::new();
    screen.set_rows(vec![row(1, "Abba", "Waterloo", Local::Fetching)]);
    assert_eq!(screen.handle(Input::Confirm).1, UsdbOutcome::None);
    assert!(strings(&draw(&mut screen)).iter().any(|t| t == "fetching"));
}

#[test]
fn typing_narrows_the_list_as_it_goes() {
    let mut screen = loaded();
    screen.handle(Input::Search);
    assert_eq!(screen.mode(), Mode::Searching);
    assert!(screen.wants_text(), "letter keys are text now");

    let (_, outcome) = screen.handle(Input::Type('a'));
    assert_eq!(outcome, UsdbOutcome::Search("a".to_owned()));
    assert!(screen.needs_rows(), "it did not ask for new rows");
    assert_eq!(screen.search_text(), "a");

    // And the search survives closing the keyboard.
    screen.handle(Input::Back);
    assert_eq!(screen.mode(), Mode::Browsing);
    assert_eq!(screen.search_text(), "a");
    assert!(!screen.wants_text());
}

#[test]
fn signing_in_asks_for_the_username_then_the_password() {
    let mut screen = loaded();
    let (_, outcome) = screen.handle(Input::CycleFilter);
    assert_eq!(outcome, UsdbOutcome::None);
    assert_eq!(screen.mode(), Mode::LoggingIn { password: false });
    assert!(strings(&draw(&mut screen))
        .iter()
        .any(|t| t == "USDB username"));

    for c in "walki".chars() {
        screen.handle(Input::Type(c));
    }
    screen.handle(Input::Submit);
    assert_eq!(screen.mode(), Mode::LoggingIn { password: true });
    assert!(strings(&draw(&mut screen))
        .iter()
        .any(|t| t == "USDB password"));

    for c in "hunter2".chars() {
        screen.handle(Input::Type(c));
    }
    // The password is never drawn. Not for shoulder-surfing on a sofa — for the screenshot
    // somebody takes of the party and puts online.
    let text = strings(&draw(&mut screen));
    assert!(
        !text.iter().any(|t| t.contains("hunter2")),
        "the password was on screen: {text:?}"
    );
    assert!(
        text.iter().any(|t| t.contains('\u{2022}')),
        "no dots either"
    );

    let (_, outcome) = screen.handle(Input::Submit);
    assert_eq!(
        outcome,
        UsdbOutcome::LogIn {
            user: "walki".to_owned(),
            password: "hunter2".to_owned()
        }
    );
    assert_eq!(screen.mode(), Mode::Browsing);
}

#[test]
fn cancelling_a_sign_in_keeps_nothing() {
    let mut screen = loaded();
    screen.handle(Input::CycleFilter);
    for c in "walki".chars() {
        screen.handle(Input::Type(c));
    }
    screen.handle(Input::Back);
    assert_eq!(screen.mode(), Mode::Browsing);

    // Starting again asks for the username afresh rather than remembering half a sign-in.
    screen.handle(Input::CycleFilter);
    assert_eq!(screen.mode(), Mode::LoggingIn { password: false });
    screen.handle(Input::Submit);
    let (_, outcome) = screen.handle(Input::Submit);
    assert_eq!(
        outcome,
        UsdbOutcome::LogIn {
            user: String::new(),
            password: String::new()
        }
    );
}

#[test]
fn signing_out_is_the_same_key_once_signed_in() {
    let mut screen = loaded();
    screen.user = Some("walki".to_owned());
    assert_eq!(screen.handle(Input::CycleFilter).1, UsdbOutcome::LogOut);
    assert!(strings(&draw(&mut screen))
        .iter()
        .any(|t| t.contains("walki")));
}

#[test]
fn work_in_progress_is_visible_and_can_be_stopped() {
    // A background job with no visible sign of life is indistinguishable from one that died.
    let mut screen = loaded();
    assert_eq!(
        screen.handle(Input::ContextMenu).1,
        UsdbOutcome::None,
        "nothing to stop"
    );

    screen.activity = Activity {
        what: "Abba - Waterloo \u{2014} audio".to_owned(),
        fraction: Some(0.4),
        queued: 3,
    };
    let text = strings(&draw(&mut screen));
    assert!(text.iter().any(|t| t.contains("audio")), "{text:?}");
    assert!(text.iter().any(|t| t == "3 waiting"));
    assert!(text.iter().any(|t| t == "Stop"), "no way to stop it");
    assert_eq!(screen.handle(Input::ContextMenu).1, UsdbOutcome::Cancel);
}

#[test]
fn a_problem_is_shown_in_words_rather_than_swallowed() {
    let mut screen = loaded();
    screen.problem = "USDB wants a login for that".to_owned();
    assert!(strings(&draw(&mut screen))
        .iter()
        .any(|t| t.contains("wants a login")));
}

#[test]
fn syncing_and_repairing_have_their_own_keys() {
    let mut screen = loaded();
    assert_eq!(screen.handle(Input::Sort).1, UsdbOutcome::Sync);
    assert_eq!(screen.handle(Input::CycleLayout).1, UsdbOutcome::Repair);
}

#[test]
fn the_screen_stays_inside_the_window_at_every_size() {
    let style = Theme::builtin().resolve_default();
    for (w, h) in [(1778.0, 1000.0), (1600.0, 1000.0), (1333.0, 1000.0)] {
        let bounds = Rect::new(0.0, 0.0, w, h);
        for mode in [0, 1, 2] {
            let mut screen = loaded();
            screen.activity = Activity {
                what: "fetching".to_owned(),
                fraction: Some(0.5),
                queued: 1,
            };
            if mode >= 1 {
                screen.handle(Input::Search);
            }
            if mode == 2 {
                screen.handle(Input::Back);
                screen.handle(Input::CycleFilter);
            }
            let mut list = DrawList::new();
            screen.draw(&mut list, bounds, &style);
            assert!(list.is_balanced(), "mode {mode} left a clip pushed");
            for command in list.commands() {
                if let Command::Rect { rect, .. } = command {
                    assert!(
                        rect.bottom() <= bounds.bottom() + 1.0
                            && rect.right() <= bounds.right() + 1.0
                            && rect.x >= -1.0
                            && rect.y >= -1.0,
                        "mode {mode} left the {w}x{h} window: {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_cursor_stops_at_both_ends() {
    let mut screen = loaded();
    screen.handle(Input::Up);
    assert_eq!(screen.selected().map(|r| r.id), Some(SongId(1)));
    for _ in 0..10 {
        screen.handle(Input::Down);
    }
    assert_eq!(screen.selected().map(|r| r.id), Some(SongId(3)));
}

#[test]
fn a_password_can_be_shown_while_it_is_being_typed() {
    // A password with symbols in it cannot be checked any other way, and a sign-in that fails
    // with no way to see what was typed is one nobody can debug.
    let mut screen = loaded();
    screen.handle(Input::CycleFilter);
    screen.handle(Input::Submit); // past the username
    for c in "aZ0*^&%".chars() {
        screen.handle(Input::Type(c));
    }

    let hidden = strings(&draw(&mut screen));
    assert!(
        !hidden.iter().any(|t| t.contains("aZ0")),
        "it was legible before being asked: {hidden:?}"
    );
    assert!(hidden.iter().any(|t| t == "Show it"), "no way to show it");

    // F3, or the button, or Y on a pad.
    screen.handle(Input::Sort);
    let shown = strings(&draw(&mut screen));
    assert!(
        shown.iter().any(|t| t == "aZ0*^&%"),
        "showing it did not: {shown:?}"
    );
    assert!(shown.iter().any(|t| t == "Hide it"));

    screen.handle(Input::Sort);
    assert!(!strings(&draw(&mut screen))
        .iter()
        .any(|t| t.contains("aZ0")));
}

#[test]
fn showing_a_password_is_forgotten_when_the_field_is_left() {
    // Never remembered: the next sign-in starts hidden, whatever the last one did.
    let mut screen = loaded();
    screen.handle(Input::CycleFilter);
    screen.handle(Input::Submit);
    screen.handle(Input::Sort);
    for c in "secret".chars() {
        screen.handle(Input::Type(c));
    }
    assert!(strings(&draw(&mut screen)).iter().any(|t| t == "secret"));

    screen.handle(Input::Back);
    screen.handle(Input::CycleFilter);
    screen.handle(Input::Submit);
    for c in "secret".chars() {
        screen.handle(Input::Type(c));
    }
    assert!(
        !strings(&draw(&mut screen)).iter().any(|t| t == "secret"),
        "the next sign-in started with the password on show"
    );
}

#[test]
fn the_username_is_never_hidden() {
    // It is not a secret, and hiding it means somebody cannot check the one thing that is
    // easiest to get wrong.
    let mut screen = loaded();
    screen.handle(Input::CycleFilter);
    for c in "walki".chars() {
        screen.handle(Input::Type(c));
    }
    assert!(strings(&draw(&mut screen)).iter().any(|t| t == "walki"));
    assert!(
        !strings(&draw(&mut screen)).iter().any(|t| t == "Show it"),
        "the username offered a reveal it does not need"
    );
}

#[test]
fn the_reveal_button_can_be_clicked() {
    let style = Theme::builtin().resolve_default();
    let ready = || {
        let mut screen = loaded();
        screen.handle(Input::CycleFilter);
        screen.handle(Input::Submit);
        for c in "secret".chars() {
            screen.handle(Input::Type(c));
        }
        screen
    };
    let mut clicked_somewhere = false;
    for x in (0..1600).step_by(8) {
        for y in (0..1000).step_by(8) {
            let mut screen = ready();
            let mut list = DrawList::new();
            screen.draw(&mut list, area(), &style);
            screen.handle(Input::Click(rungstar_ui::geom::Point::new(
                x as f32, y as f32,
            )));
            let mut after = DrawList::new();
            screen.draw(&mut after, area(), &style);
            if strings(&after).iter().any(|t| t == "secret") {
                clicked_somewhere = true;
                break;
            }
        }
        if clicked_somewhere {
            break;
        }
    }
    assert!(clicked_somewhere, "the reveal button cannot be clicked");
}
