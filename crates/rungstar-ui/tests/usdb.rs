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
    let (_, outcome) = screen.handle(Input::CycleLayout);
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
    screen.handle(Input::CycleLayout);
    for c in "walki".chars() {
        screen.handle(Input::Type(c));
    }
    screen.handle(Input::Back);
    assert_eq!(screen.mode(), Mode::Browsing);

    // Starting again asks for the username afresh rather than remembering half a sign-in.
    screen.handle(Input::CycleLayout);
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
    assert_eq!(screen.handle(Input::CycleLayout).1, UsdbOutcome::LogOut);
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
        UsdbOutcome::Repair,
        "with nothing to stop, that key repairs instead"
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
    // Repair shares a key with Stop, decided by whether there is anything to stop: cancelling
    // is urgent and repairing is not.
    assert_eq!(screen.handle(Input::ContextMenu).1, UsdbOutcome::Repair);
    assert_eq!(screen.handle(Input::Type('g')).1, UsdbOutcome::GetTool);
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
                screen.handle(Input::CycleLayout);
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
    screen.handle(Input::CycleLayout);
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
    screen.handle(Input::CycleLayout);
    screen.handle(Input::Submit);
    screen.handle(Input::Sort);
    for c in "secret".chars() {
        screen.handle(Input::Type(c));
    }
    assert!(strings(&draw(&mut screen)).iter().any(|t| t == "secret"));

    screen.handle(Input::Back);
    screen.handle(Input::CycleLayout);
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
    screen.handle(Input::CycleLayout);
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
        screen.handle(Input::CycleLayout);
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

#[test]
fn the_filter_narrows_to_what_is_not_already_held() {
    // The question this screen exists to answer. Thirty thousand songs is not a list anybody
    // scrolls, and most of them are ones you already have.
    let mut screen = loaded();
    assert_eq!(screen.narrow, rungstar_ui::usdbscreen::Narrow::Everything);
    let (_, outcome) = screen.handle(Input::CycleFilter);
    assert_eq!(outcome, UsdbOutcome::None);
    assert_eq!(screen.narrow, rungstar_ui::usdbscreen::Narrow::New);
    assert!(screen.needs_rows(), "it did not ask for the list again");

    // And it says what it is hiding, because a list quietly missing songs looks like a
    // catalog that does not have them.
    let text = strings(&draw(&mut screen)).join(" ");
    assert!(text.contains("Not in my library"), "{text}");
}

#[test]
fn every_filter_keeps_what_it_claims_to() {
    use rungstar_ui::usdbscreen::Narrow;
    let absent = row(1, "A", "One", Local::Absent);
    let held = row(2, "B", "Two", Local::Held);
    let mut poor = row(3, "C", "Three", Local::Absent);
    poor.rating = 2.0;
    poor.golden = false;

    assert!(Narrow::Everything.keeps(&absent) && Narrow::Everything.keeps(&held));
    assert!(Narrow::New.keeps(&absent) && !Narrow::New.keeps(&held));
    assert!(Narrow::Held.keeps(&held) && !Narrow::Held.keeps(&absent));
    assert!(Narrow::WellRated.keeps(&absent) && !Narrow::WellRated.keeps(&poor));
    assert!(Narrow::Golden.keeps(&absent) && !Narrow::Golden.keeps(&poor));
}

#[test]
fn the_language_filter_only_offers_languages_the_catalog_has() {
    let mut screen = loaded();
    screen.languages = vec![("English".to_owned(), 900), ("German".to_owned(), 400)];
    assert_eq!(screen.language, None, "everything, to begin with");

    screen.handle(Input::Random);
    assert_eq!(screen.language.as_deref(), Some("English"));
    screen.handle(Input::Random);
    assert_eq!(screen.language.as_deref(), Some("German"));
    // Past the end is back to any, rather than stuck on the last one.
    screen.handle(Input::Random);
    assert_eq!(screen.language, None);

    // With no catalog there is nothing to cycle and it stays at any.
    let mut empty = loaded();
    empty.handle(Input::Random);
    assert_eq!(empty.language, None);
}

#[test]
fn a_rating_is_drawn_as_shapes_rather_than_as_glyphs() {
    // It was a star character until somebody saw five empty boxes: the game borrows a system
    // font, and a borrowed font is not promised to have any particular glyph in it.
    let mut screen = loaded();
    let list = draw(&mut screen);
    assert!(
        !strings(&list)
            .iter()
            .any(|t| t.contains('\u{2605}') || t.contains('\u{2606}')),
        "the rating is still drawn as text"
    );
    // Five pips per row, drawn as rounded rectangles.
    let pips = list
        .commands()
        .iter()
        .filter(|c| match c {
            Command::Rect { rect, .. } | Command::Outline { rect, .. } => {
                (rect.w - rect.h).abs() < 1.0 && rect.w > 2.0 && rect.w < 30.0
            }
            _ => false,
        })
        .count();
    assert!(pips >= 15, "only {pips} pips for three rows of five");
}

#[test]
fn a_signed_out_visitor_is_told_where_to_get_an_account() {
    // Browsing works without one and downloading does not, and somebody who has never heard
    // of USDB cannot guess that it is a website they register on first.
    let mut screen = loaded();
    let text = strings(&draw(&mut screen)).join(" ");
    assert!(text.contains("usdb.animux.de"), "{text}");
    assert!(text.contains("free USDB account"));

    // And it goes away once they are in, rather than nagging forever.
    screen.user = Some("walki".to_owned());
    let text = strings(&draw(&mut screen)).join(" ");
    assert!(!text.contains("usdb.animux.de"), "still nagging: {text}");
}
