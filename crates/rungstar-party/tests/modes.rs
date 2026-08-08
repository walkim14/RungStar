//! The challenge rules, a party and a bracket, all driven without singing anything.
//!
//! This is the point of taking the scripting engine out: every one of these was a Lua plugin
//! that could only be exercised by playing a song with a microphone.

use rungstar_party::challenge::{Finish, Knockout, Length, Music};
use rungstar_party::{Bracket, BracketError, Challenge, Ending, Party, Phase, Team, Watch};

fn watch(challenge: &str, singers: usize, total_beats: f64) -> Watch {
    Watch::new(
        Challenge::by_id(challenge).effects,
        singers,
        total_beats,
        0xC0FFEE,
    )
}

// ---------------------------------------------------------------- the catalogue

#[test]
fn every_challenge_is_named_explained_and_distinct() {
    let mut ids = std::collections::HashSet::new();
    for challenge in Challenge::ALL {
        assert!(ids.insert(challenge.id), "two challenges share an id");
        assert!(!challenge.name.is_empty());
        assert!(
            challenge.blurb.len() > 20,
            "{} does not say what it does",
            challenge.id
        );
        // A blurb is a sentence, not a label.
        assert!(challenge.blurb.ends_with('.'), "{}", challenge.id);
    }
    assert_eq!(
        Challenge::ALL.len(),
        15,
        "the fourteen reference plugins plus the plain song"
    );
}

#[test]
fn an_id_from_a_newer_build_opens_as_the_plain_song() {
    // A saved party naming a mode this build does not have should still load. Refusing would
    // lose the whole party over one row.
    assert_eq!(Challenge::by_id("normal").id, "normal");
    assert_eq!(Challenge::by_id("moonwalk").id, "normal");
    assert!(Challenge::by_id("moonwalk").effects.is_plain());
}

#[test]
fn only_the_knockout_modes_can_put_somebody_out() {
    let knockouts: Vec<&str> = Challenge::ALL
        .iter()
        .filter(|c| c.is_knockout())
        .map(|c| c.id)
        .collect();
    assert_eq!(
        knockouts,
        vec!["hardcore", "hold-the-line", "hold-the-line-blind"]
    );
}

#[test]
fn the_blind_modes_take_away_what_they_say_they_do() {
    let effects = |id| Challenge::by_id(id).effects;
    assert!(!effects("blind-lyrics").lyrics && effects("blind-lyrics").notes);
    assert!(effects("blind-notes").lyrics && !effects("blind-notes").notes);
    assert!(!effects("blind").lyrics && !effects("blind").notes);
    assert!(!effects("hold-the-line-blind").lyrics);
    assert!(matches!(
        effects("hold-the-line-blind").knockout,
        Some(Knockout::Rising { .. })
    ));
    assert_eq!(effects("to-5000").finish, Finish::AtPoints(5000));
    assert_eq!(effects("short").length, Length::Half);
    assert!(matches!(effects("deaf").music, Music::Cutting { .. }));
}

// ---------------------------------------------------------------- first to N

#[test]
fn a_points_target_ends_the_song_and_names_who_got_there() {
    let mut watch = watch("to-2000", 2, 400.0);
    watch.line_ended(50.0, &[900, 800], &[0.9, 0.8]);
    assert_eq!(watch.ending(), None, "nobody is there yet");

    watch.line_ended(100.0, &[1500, 2100], &[0.9, 0.9]);
    assert_eq!(
        watch.ending(),
        Some(Ending::Reached {
            singer: 1,
            points: 2100
        })
    );

    // And it stays decided: a later line cannot hand it to somebody else.
    watch.line_ended(150.0, &[9000, 2100], &[1.0, 0.0]);
    assert_eq!(
        watch.ending(),
        Some(Ending::Reached {
            singer: 1,
            points: 2100
        })
    );
}

#[test]
fn two_singers_crossing_the_line_together_is_won_by_the_higher_score() {
    let mut watch = watch("to-2000", 2, 400.0);
    watch.line_ended(100.0, &[2400, 2100], &[1.0, 1.0]);
    assert_eq!(
        watch.ending(),
        Some(Ending::Reached {
            singer: 0,
            points: 2400
        })
    );
}

// ---------------------------------------------------------------- short song

#[test]
fn a_short_song_stops_at_the_first_line_past_halfway() {
    let mut watch = watch("short", 1, 400.0);
    watch.line_ended(180.0, &[500], &[0.5]);
    assert_eq!(watch.ending(), None, "still in the first half");
    watch.line_ended(210.0, &[900], &[0.5]);
    assert_eq!(watch.ending(), Some(Ending::Halfway));
}

#[test]
fn a_whole_song_is_not_cut_short() {
    let mut watch = watch("normal", 1, 400.0);
    for beat in [100.0, 200.0, 300.0, 399.0] {
        watch.line_ended(beat, &[1000], &[0.5]);
        assert_eq!(watch.ending(), None);
    }
    watch.song_ended();
    assert_eq!(watch.ending(), Some(Ending::Song));
}

// ---------------------------------------------------------------- hardcore

#[test]
fn hardcore_puts_out_the_singer_who_is_missing_more_than_the_others() {
    let mut watch = watch("hardcore", 2, 400.0);
    let mut score = [0, 0];
    // Three silent lines for the second singer while the first keeps scoring.
    for line in 0..3 {
        score[0] += 300;
        watch.line_ended(50.0 * (line + 1) as f64, &score, &[0.7, 0.0]);
    }
    assert!(!watch.is_out(0));
    assert!(watch.is_out(1), "three silent lines and still in");
    assert_eq!(watch.ending(), Some(Ending::Knockout));
    assert_eq!(watch.standings()[1].out_at, Some(2));
}

#[test]
fn hardcore_does_not_end_a_round_because_everybody_missed_the_same_verse() {
    // The rule that is easy to get wrong. A hard verse that nobody scores is a hard verse.
    let mut watch = watch("hardcore", 2, 400.0);
    for line in 0..5 {
        watch.line_ended(50.0 * (line + 1) as f64, &[0, 0], &[0.0, 0.0]);
        assert_eq!(watch.ending(), None, "line {line} ended the round");
    }
    assert!(!watch.is_out(0) && !watch.is_out(1));

    // But the moment one of them falls a line further behind, they are out.
    watch.line_ended(300.0, &[400, 0], &[0.6, 0.0]);
    assert!(watch.is_out(1));
    assert!(!watch.is_out(0));
}

#[test]
fn hardcore_counts_silent_lines_in_a_row_rather_than_in_total() {
    let mut watch = watch("hardcore", 2, 400.0);
    let mut score = [0, 0];
    // Two silent, then one that scores, then two silent again: five bad lines out of six, and
    // never three together, so still in. Somebody having a rough verse twice is not out.
    for gained in [0, 0, 500, 0, 0] {
        score[1] += gained;
        score[0] += 300;
        watch.line_ended(50.0, &score, &[0.7, 0.3]);
    }
    assert!(!watch.is_out(1), "counted in total rather than in a row");
    assert_eq!(watch.standings()[1].silent_lines, 2);
}

#[test]
fn a_lone_singer_is_never_knocked_out_by_hardcore() {
    // There is nobody to be worse than, and ending a solo song early because the singer had
    // three quiet lines is the game taking the microphone off them.
    let mut watch = watch("hardcore", 1, 400.0);
    for line in 0..6 {
        watch.line_ended(50.0 * (line + 1) as f64, &[0], &[0.0]);
    }
    assert_eq!(watch.ending(), None);
    assert!(!watch.is_out(0));
}

// ---------------------------------------------------------------- hold the line

#[test]
fn the_bar_rises_from_nothing_to_perfect_three_quarters_in() {
    let watch = watch("hold-the-line", 2, 400.0);
    assert_eq!(watch.bar_at(0.0), Some(0.0), "the first line cannot end it");
    assert_eq!(watch.bar_at(150.0), Some(0.5));
    assert_eq!(watch.bar_at(300.0), Some(1.0));
    assert_eq!(watch.bar_at(400.0), Some(1.0), "and stays there");
}

#[test]
fn a_plain_song_has_no_bar_to_show() {
    assert_eq!(watch("normal", 1, 400.0).bar_at(200.0), None);
    assert_eq!(watch("hardcore", 1, 400.0).bar_at(200.0), None);
}

#[test]
fn hold_the_line_puts_out_whoever_falls_under_the_rising_bar() {
    let mut watch = watch("hold-the-line", 2, 400.0);
    // Early on the bar is low and a mediocre line survives.
    watch.line_ended(20.0, &[300, 300], &[0.4, 0.4]);
    assert!(!watch.is_out(0) && !watch.is_out(1));

    // Later it is not enough for the one who has been coasting.
    for line in 1..6 {
        watch.line_ended(20.0 + 40.0 * line as f64, &[300, 300], &[0.95, 0.35]);
    }
    assert!(watch.is_out(1), "a running average of 0.4 cleared the bar");
    assert!(!watch.is_out(0));
    assert_eq!(watch.ending(), Some(Ending::Knockout));
}

#[test]
fn one_disastrous_line_does_not_end_a_round_on_its_own() {
    // The bar is what rises; what it measures should not be jumpy. A single missed line in an
    // otherwise clean song has to survive, or every round ends on a cough.
    let mut watch = watch("hold-the-line", 2, 400.0);
    for line in 0..8 {
        let rating = if line == 5 { 0.0 } else { 1.0 };
        watch.line_ended(30.0 * (line + 1) as f64, &[900, 900], &[rating, rating]);
    }
    assert_eq!(watch.ending(), None);
    assert!(!watch.is_out(0));
}

// ---------------------------------------------------------------- deaf

#[test]
fn deaf_cuts_the_music_out_and_brings_it_back() {
    let mut watch = watch("deaf", 1, 400.0);
    assert!(watch.music_at(0.0), "it starts with the music on");

    let mut changes = 0;
    let mut playing = true;
    let mut silent_for = 0.0;
    let mut longest_silence: f64 = 0.0;
    for step in 1..4000 {
        let seconds = step as f64 * 0.1;
        let now = watch.music_at(seconds);
        if now != playing {
            changes += 1;
            playing = now;
            longest_silence = longest_silence.max(silent_for);
            silent_for = 0.0;
        } else if !now {
            silent_for += 0.1;
        }
    }
    assert!(changes > 10, "the music never cut out: {changes} changes");
    assert!(
        longest_silence < 6.5,
        "a silence of {longest_silence:.1}s loses the beat entirely"
    );
}

#[test]
fn every_other_mode_leaves_the_music_alone() {
    for challenge in Challenge::ALL.iter().filter(|c| c.id != "deaf") {
        let mut watch = Watch::new(challenge.effects, 1, 400.0, 7);
        for step in 0..200 {
            assert!(
                watch.music_at(step as f64 * 0.5),
                "{} muted the song",
                challenge.id
            );
        }
    }
}

#[test]
fn the_same_seed_cuts_the_music_at_the_same_moments() {
    // Deterministic on purpose: a mode that cannot be replayed cannot be debugged.
    let pattern = |seed| {
        let mut watch = Watch::new(Challenge::by_id("deaf").effects, 1, 400.0, seed);
        (0..600)
            .map(|step| watch.music_at(step as f64 * 0.1))
            .collect::<Vec<bool>>()
    };
    assert_eq!(pattern(42), pattern(42));
    assert_ne!(pattern(42), pattern(43));
}

// ---------------------------------------------------------------- party

fn party() -> Party {
    Party::new(
        vec![
            Team::new("Left", vec!["Ada".into(), "Grace".into()]),
            Team::new("Right", vec!["Kata".into()]),
        ],
        4,
    )
}

#[test]
fn a_party_runs_its_rounds_and_then_stops() {
    let mut party = party();
    assert_eq!(party.phase(), Phase::Choosing);
    for round in 1..=4 {
        assert_eq!(party.round(), round);
        party.offer(format!("Song {round}"));
        assert_eq!(party.accept(), Some(format!("Song {round}").as_str()));
        assert_eq!(party.phase(), Phase::Singing);
        party.finish_round(&[5000, 4000]);
    }
    assert_eq!(party.phase(), Phase::Finished);
    assert_eq!(party.played.len(), 4);
    assert_eq!(party.winner(), Some(0));

    // And a finished party stays finished.
    party.finish_round(&[9000, 0]);
    assert_eq!(party.played.len(), 4);
}

#[test]
fn two_teams_give_one_point_to_the_winner_of_each_round() {
    let mut party = party();
    party.offer("One");
    party.accept();
    party.finish_round(&[3000, 7000]);
    assert_eq!(party.teams[0].points, 0);
    assert_eq!(party.teams[1].points, 1);
    assert_eq!(party.played[0].awarded, vec![0, 1]);
    // Song points are kept as well, because round points alone read as a mystery.
    assert_eq!(party.teams[1].sung, 7000);
}

#[test]
fn three_teams_give_three_for_first_and_one_for_second() {
    let mut party = Party::new(
        vec![
            Team::new("A", vec!["a".into()]),
            Team::new("B", vec!["b".into()]),
            Team::new("C", vec!["c".into()]),
        ],
        2,
    );
    party.offer("One");
    party.accept();
    party.finish_round(&[4000, 8000, 6000]);
    assert_eq!(party.played[0].awarded, vec![0, 3, 1]);
}

#[test]
fn a_tied_round_shares_the_placing_rather_than_inventing_an_order() {
    let mut party = Party::new(
        vec![
            Team::new("A", vec!["a".into()]),
            Team::new("B", vec!["b".into()]),
            Team::new("C", vec!["c".into()]),
        ],
        2,
    );
    party.offer("One");
    party.accept();
    party.finish_round(&[8000, 8000, 6000]);
    // Both firsts, and nobody second: that is what happens on a podium.
    assert_eq!(party.played[0].awarded, vec![3, 3, 0]);
}

#[test]
fn a_joker_rejects_a_song_and_runs_out() {
    let mut party = party();
    assert_eq!(party.teams[0].jokers, 5);
    for _ in 0..5 {
        party.offer("Something nobody knows");
        assert!(party.can_reject());
        assert!(party.reject());
        assert_eq!(party.offered, None, "the song survived the joker");
    }
    assert_eq!(party.teams[0].jokers, 0);

    party.offer("Something nobody knows");
    assert!(!party.can_reject(), "a sixth joker appeared");
    assert!(!party.reject());
    assert_eq!(
        party.offered.as_deref(),
        Some("Something nobody knows"),
        "a refused rejection must leave the song alone"
    );
}

#[test]
fn the_teams_take_turns_being_offered_the_song() {
    let mut party = party();
    for round in 0..4 {
        assert_eq!(party.team_up(), round % 2);
        party.offer("Song");
        party.accept();
        party.finish_round(&[100, 100]);
    }
}

#[test]
fn everybody_in_a_team_sings_before_anybody_sings_twice() {
    let mut party = party();
    assert_eq!(party.teams[0].singer(), Some("Ada"));
    party.offer("One");
    party.accept();
    party.finish_round(&[100, 100]);
    assert_eq!(party.teams[0].singer(), Some("Grace"));
    assert_eq!(party.teams[1].singer(), Some("Kata"), "a team of one");
    party.offer("Two");
    party.accept();
    party.finish_round(&[100, 100]);
    assert_eq!(party.teams[0].singer(), Some("Ada"), "round the team again");
}

#[test]
fn a_dead_heat_over_the_whole_party_has_no_winner() {
    let mut party = Party::new(
        vec![
            Team::new("A", vec!["a".into()]),
            Team::new("B", vec!["b".into()]),
        ],
        2,
    );
    for _ in 0..2 {
        party.offer("Song");
        party.accept();
        party.finish_round(&[5000, 5000]);
    }
    assert_eq!(party.phase(), Phase::Finished);
    assert_eq!(party.winner(), None, "a tie is a tie");
}

#[test]
fn equal_round_points_are_broken_by_what_was_actually_sung() {
    let mut party = Party::new(
        vec![
            Team::new("A", vec!["a".into()]),
            Team::new("B", vec!["b".into()]),
        ],
        2,
    );
    party.offer("One");
    party.accept();
    party.finish_round(&[9000, 1000]);
    party.offer("Two");
    party.accept();
    party.finish_round(&[1000, 1100]);
    // One round each, but A sang far better across the two.
    assert_eq!(party.teams[0].points, 1);
    assert_eq!(party.teams[1].points, 1);
    assert_eq!(party.winner(), Some(0));
    assert_eq!(party.standings(), vec![0, 1]);
}

// ---------------------------------------------------------------- bracket

fn players(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("P{i}")).collect()
}

#[test]
fn a_bracket_needs_a_power_of_two() {
    for size in [2, 4, 8, 16] {
        assert!(Bracket::new(players(size)).is_ok());
    }
    for size in [0, 1, 3, 5, 6, 7, 12, 32] {
        assert_eq!(
            Bracket::new(players(size)),
            Err(BracketError::NotAPowerOfTwo(size)),
            "{size} players"
        );
    }
    // And the refusal says why, because "invalid" leaves somebody counting chairs.
    let message = BracketError::NotAPowerOfTwo(5).to_string();
    assert!(message.contains('5') && message.contains("sit out"));
}

#[test]
fn a_bracket_plays_down_to_a_champion() {
    let mut bracket = Bracket::new(players(8)).unwrap();
    assert_eq!(bracket.total_rounds(), 3);
    assert_eq!(bracket.round_name(0), "Quarter-final");
    assert_eq!(bracket.round_name(1), "Semi-final");
    assert_eq!(bracket.round_name(2), "Final");

    let mut played = 0;
    while let Some((round, index)) = bracket.next_match() {
        // The left player always wins, so the champion is predictable.
        bracket.report(round, index, (5000, 1000));
        played += 1;
        assert!(played <= 7, "the bracket never finished");
    }
    assert_eq!(played, 7, "eight players is seven matches");
    assert_eq!(bracket.champion(), Some(0));
    assert!(bracket.is_finished());
    assert_eq!(bracket.name(0), "P1");
}

#[test]
fn the_next_round_appears_only_once_the_last_one_is_complete() {
    let mut bracket = Bracket::new(players(4)).unwrap();
    assert_eq!(bracket.rounds.len(), 1);
    bracket.report(0, 0, (100, 200));
    assert_eq!(bracket.rounds.len(), 1, "half a round is not a round");
    bracket.report(0, 1, (300, 100));
    assert_eq!(bracket.rounds.len(), 2);
    // The winners meet: P2 beat P1, P3 beat P4.
    assert_eq!(bracket.rounds[1][0].left, 1);
    assert_eq!(bracket.rounds[1][0].right, 2);
}

#[test]
fn a_drawn_match_still_sends_somebody_through() {
    // At half past eleven, "sing it again" is not an answer.
    let mut bracket = Bracket::new(players(2)).unwrap();
    bracket.report(0, 0, (4200, 4200));
    assert_eq!(bracket.champion(), Some(0));
}

#[test]
fn placings_rank_by_how_far_somebody_got() {
    let mut bracket = Bracket::new(players(4)).unwrap();
    bracket.report(0, 0, (900, 100)); // P1 beats P2
    bracket.report(0, 1, (100, 900)); // P4 beats P3
    bracket.report(1, 0, (900, 100)); // P1 beats P4
    let placings = bracket.placings();
    assert_eq!(placings[0], 0, "champion first");
    assert_eq!(placings[1], 3, "beaten in the final");
    // The two knocked out in the first round are not ranked against each other; a bracket
    // does not know which of them was better.
    assert_eq!(placings.len(), 4);
    assert!(placings[2..].contains(&1) && placings[2..].contains(&2));
}

#[test]
fn an_unfinished_bracket_has_no_champion() {
    let mut bracket = Bracket::new(players(4)).unwrap();
    assert_eq!(bracket.champion(), None);
    bracket.report(0, 0, (900, 100));
    assert_eq!(bracket.champion(), None);
    assert!(!bracket.is_finished());
}
