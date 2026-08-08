//! The party screen and what a challenge does to the sing screen.

use rungstar_party::{Bracket, Challenge, Party, Team};
use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::{Point, Rect};
use rungstar_ui::partyscreen::{Kind, PartyOutcome, PartyScreen, Stage};
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

fn screen(pool: &[&str]) -> PartyScreen {
    let mut screen = PartyScreen::new();
    screen.pool = pool.iter().map(|s| (*s).to_owned()).collect();
    screen
}

fn draw(screen: &mut PartyScreen) -> DrawList {
    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &Theme::builtin().resolve_default());
    assert!(list.is_balanced());
    list
}

/// Put the setup cursor on the row whose help begins with `prefix`.
fn to_row(screen: &mut PartyScreen, prefix: &str) {
    for _ in 0..8 {
        if screen.help().starts_with(prefix) {
            return;
        }
        screen.handle(Input::Down);
    }
    panic!("no setup row explained by {prefix:?}");
}

#[test]
fn the_setup_offers_the_three_kinds_and_says_what_each_does() {
    let mut screen = screen(&["Ada", "Grace"]);
    let mut seen = Vec::new();
    for _ in 0..Kind::ALL.len() {
        seen.push(screen.kind);
        let text = strings(&draw(&mut screen));
        assert!(
            text.iter().any(|t| t == screen.kind.name()),
            "the kind was not named: {text:?}"
        );
        assert!(
            text.iter().any(|t| t == screen.kind.blurb()),
            "the kind was not explained"
        );
        screen.handle(Input::Right);
    }
    assert_eq!(seen, Kind::ALL.to_vec());
    assert_eq!(screen.kind, Kind::ALL[0], "the kinds did not come round");
}

#[test]
fn a_tournament_only_offers_powers_of_two() {
    let mut screen = screen(&["a", "b", "c", "d"]);
    assert_eq!(screen.sizes(), &[2, 3], "a party takes two or three teams");

    screen.kind = Kind::Tournament;
    assert_eq!(screen.sizes(), &[2, 4, 8, 16]);

    // Switching to a tournament with three teams chosen cannot leave three players standing:
    // a bracket of three needs a bye and a bye is somebody who advances without singing.
    let mut screen = screen_with(Kind::Classic, 3);
    to_row(&mut screen, "Two or three teams");
    screen.handle(Input::Up);
    screen.handle(Input::Right);
    assert_eq!(screen.kind, Kind::Free);
    screen.handle(Input::Right);
    assert_eq!(screen.kind, Kind::Tournament);
    assert!(
        screen.sizes().contains(&screen.size),
        "left at {} players, which is not a bracket",
        screen.size
    );
}

fn screen_with(kind: Kind, size: usize) -> PartyScreen {
    let mut screen = screen(&["a", "b", "c", "d"]);
    screen.kind = kind;
    screen.size = size;
    screen
}

#[test]
fn a_party_cannot_start_without_enough_singers() {
    let mut screen = screen(&["Ada"]);
    screen.size = 2;
    assert_eq!(screen.short_by(), 1);
    to_row(&mut screen, "Singers come from");
    let (_, outcome) = screen.handle(Input::Confirm);
    assert_eq!(outcome, PartyOutcome::None, "started a party of one");

    // And it says how many more are wanted rather than doing nothing.
    let text = strings(&draw(&mut screen));
    assert!(
        text.iter().any(|t| t.contains("One more singer")),
        "no explanation for the dead button: {text:?}"
    );

    screen.pool.push("Grace".to_owned());
    assert_eq!(screen.short_by(), 0);
    let (_, outcome) = screen.handle(Input::Confirm);
    assert_eq!(outcome, PartyOutcome::Begin);
}

#[test]
fn a_classic_round_offers_a_joker_and_a_free_one_does_not() {
    let mut screen = screen(&["Ada", "Grace"]);
    screen.party = Some(Party::new(
        vec![
            Team::new("Team 1", vec!["Ada".into()]),
            Team::new("Team 2", vec!["Grace".into()]),
        ],
        3,
    ));
    screen.party.as_mut().unwrap().offer("Abba - Waterloo");
    screen.offered = Some("Abba - Waterloo".to_owned());
    screen.to_round();

    let text = strings(&draw(&mut screen));
    assert!(text.iter().any(|t| t == "Sing it"));
    assert!(text.iter().any(|t| t == "Use a joker"));
    assert!(
        text.iter().any(|t| t.contains("5 jokers")),
        "the jokers left are not shown: {text:?}"
    );

    screen.kind = Kind::Free;
    let text = strings(&draw(&mut screen));
    assert!(
        !text.iter().any(|t| t == "Use a joker"),
        "free choice needs no joker"
    );
    assert!(text.iter().any(|t| t == "Choose a song"));
}

#[test]
fn the_joker_disappears_when_a_team_runs_out() {
    let mut screen = screen(&["Ada", "Grace"]);
    let mut party = Party::new(
        vec![
            Team::new("Team 1", vec!["Ada".into()]),
            Team::new("Team 2", vec!["Grace".into()]),
        ],
        3,
    );
    for _ in 0..5 {
        party.offer("Something");
        assert!(party.reject());
    }
    party.offer("Something");
    screen.party = Some(party);
    screen.offered = Some("Something".to_owned());
    screen.to_round();

    let text = strings(&draw(&mut screen));
    assert!(
        !text.iter().any(|t| t == "Use a joker"),
        "a sixth joker was offered: {text:?}"
    );
}

#[test]
fn a_round_names_whose_turn_it_is() {
    let mut screen = screen(&["Ada", "Grace"]);
    screen.party = Some(Party::new(
        vec![
            Team::new("Reds", vec!["Ada".into(), "Kata".into()]),
            Team::new("Blues", vec!["Grace".into()]),
        ],
        3,
    ));
    screen.to_round();
    assert!(screen.up_now().contains("Reds") && screen.up_now().contains("Ada"));

    let mut bracket = PartyScreen::new();
    bracket.kind = Kind::Tournament;
    bracket.bracket = Some(Bracket::new(vec!["Ada".into(), "Grace".into()]).unwrap());
    bracket.to_round();
    assert_eq!(bracket.up_now(), "Ada against Grace");
}

#[test]
fn giving_up_on_a_party_takes_two_presses() {
    // Escape on a party screen must not land on the main menu: three other people are waiting
    // and the standings go with it.
    let mut screen = screen(&["Ada", "Grace"]);
    screen.offered = Some("Abba - Waterloo".to_owned());
    screen.to_round();
    let (transition, outcome) = screen.handle(Input::Back);
    assert_eq!(transition, Transition::None);
    assert_eq!(outcome, PartyOutcome::None);

    let (transition, outcome) = screen.handle(Input::Confirm);
    assert_eq!(transition, Transition::Pop);
    assert_eq!(outcome, PartyOutcome::Leave);
}

#[test]
fn a_finished_party_crowns_the_winner_and_lists_everybody() {
    let mut screen = screen(&["Ada", "Grace"]);
    let mut party = Party::new(
        vec![
            Team::new("Reds", vec!["Ada".into()]),
            Team::new("Blues", vec!["Grace".into()]),
        ],
        1,
    );
    party.offer("Abba - Waterloo");
    party.accept();
    party.finish_round(&[7000, 3000]);
    screen.party = Some(party);
    screen.to_finished();

    let text = strings(&draw(&mut screen));
    assert!(text.iter().any(|t| t == "Reds wins"), "{text:?}");
    assert!(text.iter().any(|t| t == "Blues"), "the loser is not listed");
    assert!(text.iter().any(|t| t.contains("7000 sung")));
}

#[test]
fn a_drawn_party_says_so_rather_than_picking_somebody() {
    let mut screen = screen(&["Ada", "Grace"]);
    let mut party = Party::new(
        vec![
            Team::new("Reds", vec!["Ada".into()]),
            Team::new("Blues", vec!["Grace".into()]),
        ],
        1,
    );
    party.offer("Song");
    party.accept();
    party.finish_round(&[5000, 5000]);
    screen.party = Some(party);
    screen.to_finished();
    assert!(strings(&draw(&mut screen))
        .iter()
        .any(|t| t == "A dead heat"));
}

#[test]
fn the_challenge_chosen_for_a_party_is_named_on_every_round() {
    let mut screen = screen(&["Ada", "Grace"]);
    to_row(&mut screen, Challenge::ALL[0].blurb);
    screen.handle(Input::Right);
    assert_eq!(screen.challenge().id, Challenge::ALL[1].id);
    assert_eq!(
        screen.help(),
        Challenge::ALL[1].blurb,
        "the help follows it"
    );
    screen.offered = Some("Abba - Waterloo".to_owned());
    screen.to_round();
    let text = strings(&draw(&mut screen));
    assert!(
        text.iter().any(|t| t == screen.challenge().name),
        "a party under a challenge did not say which: {text:?}"
    );
}

#[test]
fn every_stage_stays_inside_the_window() {
    let style = Theme::builtin().resolve_default();
    for (w, h) in [(1778.0, 1000.0), (1600.0, 1000.0), (1333.0, 1000.0)] {
        let bounds = Rect::new(0.0, 0.0, w, h);
        let mut screen = screen(&["Ada", "Grace", "Kata"]);
        let mut party = Party::new(
            vec![
                Team::new("Reds", vec!["Ada".into()]),
                Team::new("Blues", vec!["Grace".into()]),
            ],
            2,
        );
        party.offer("Abba - Waterloo");
        screen.party = Some(party);
        screen.offered = Some("Abba - Waterloo".to_owned());

        for stage in [Stage::Setup, Stage::Round, Stage::Finished] {
            screen.stage = stage;
            let mut list = DrawList::new();
            screen.draw(&mut list, bounds, &style);
            assert!(list.is_balanced(), "{stage:?} left a clip pushed");
            for command in list.commands() {
                if let Command::Rect { rect, .. } = command {
                    assert!(
                        rect.bottom() <= bounds.bottom() + 1.0
                            && rect.right() <= bounds.right() + 1.0
                            && rect.x >= -1.0,
                        "{stage:?} left the {w}x{h} window: {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_round_buttons_can_be_clicked() {
    let ready = || {
        let mut screen = screen(&["Ada", "Grace"]);
        screen.offered = Some("Abba - Waterloo".to_owned());
        screen.to_round();
        screen
    };
    let mut sang = false;
    for y in 0..1000 {
        let mut copy = ready();
        draw(&mut copy);
        if copy.handle(Input::Click(Point::new(800.0, y as f32))).1 == PartyOutcome::Sing {
            sang = true;
            break;
        }
    }
    assert!(sang, "the Sing button cannot be clicked");
}
