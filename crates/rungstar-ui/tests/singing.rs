//! The sing screen. Driven by state and read back as commands, like every other screen.

use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::Rect;
use rungstar_ui::screen::Transition;
use rungstar_ui::singscreen::{
    fold_to_octave, rating_title, Note, NoteKind, NoteLine, Overlay, PauseChoice, SingScreen,
    Singer, Sung, Syllable,
};
use rungstar_ui::songselect::Input;
use rungstar_ui::theme::Theme;

fn area() -> Rect {
    Rect::new(0.0, 0.0, 1600.0, 1000.0)
}

/// One line of nine notes across a seven-semitone range.
fn line() -> NoteLine {
    let notes: Vec<Note> = (0..9)
        .map(|i| Note {
            start: 8.0 + i as f64 * 4.0,
            duration: 3.0,
            pitch: 60 + (i % 7),
            kind: match i % 6 {
                0 => NoteKind::Golden,
                3 => NoteKind::Freestyle,
                _ => NoteKind::Normal,
            },
        })
        .collect();
    NoteLine {
        start: notes.first().map(|n| n.start).unwrap_or(0.0),
        end: notes.last().map(Note::end).unwrap_or(0.0),
        notes,
    }
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
        &line(),
        &syllables(),
        "the next line",
        beat,
    );
    list
}

/// A screen with the song's pitch range already set, as the application does at song start.
fn sing_screen(singers: usize) -> SingScreen {
    let mut screen = SingScreen::new("Artist", "Title", singers);
    screen.pitch_low = 60;
    screen.pitch_high = 66;
    screen
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
        let screen = sing_screen(count);
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
    let screen = sing_screen(6);
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
                let mut screen = sing_screen(count);
                screen.overlay = overlay;
                screen.show_input_panel = true;
                let mut list = DrawList::new();
                screen.draw(&mut list, area, &style, &line(), &syllables(), "next", 20.0);

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
    let mut screen = sing_screen(2);
    let (transition, choice) = screen.handle(Input::Back);
    assert_eq!(transition, Transition::None);
    assert_eq!(choice, None);
    assert_eq!(screen.overlay, Overlay::Paused);
}

#[test]
fn the_pause_menu_offers_a_way_out_and_a_way_back() {
    let mut screen = sing_screen(1);
    screen.handle(Input::Back);

    // Continue is first, so the most likely choice needs no navigation.
    let (_, choice) = screen.handle(Input::Confirm);
    assert_eq!(choice, Some(PauseChoice::Continue));
    assert_eq!(screen.overlay, Overlay::None);

    let mut screen = sing_screen(1);
    screen.handle(Input::Back);
    screen.handle(Input::Down);
    let (_, choice) = screen.handle(Input::Confirm);
    assert_eq!(choice, Some(PauseChoice::Restart));

    // Up from the first entry wraps to the last.
    let mut screen = sing_screen(1);
    screen.handle(Input::Back);
    screen.handle(Input::Up);
    let (_, choice) = screen.handle(Input::Confirm);
    assert_eq!(choice, Some(PauseChoice::Quit), "the cursor did not wrap");
}

#[test]
fn escape_from_the_pause_menu_resumes() {
    let mut screen = sing_screen(1);
    screen.handle(Input::Back);
    let (_, choice) = screen.handle(Input::Back);
    assert_eq!(choice, Some(PauseChoice::Continue));
    assert_eq!(screen.overlay, Overlay::None);
}

#[test]
fn the_results_stay_up_until_dismissed() {
    // In a party the result is the point; popping straight back to the browser throws it away
    // before anybody has read it.
    let mut screen = sing_screen(3);
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
    let mut screen = sing_screen(1);
    screen.show_input_panel = false;
    screen.singers[0].has_microphone = false;

    let text = strings(&draw(&screen, 10.0)).join(" ");
    assert!(text.contains("no microphone"), "not reported: {text}");
}

#[test]
fn a_working_microphone_stays_quiet_unless_asked() {
    let mut screen = sing_screen(1);
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
    let screen = sing_screen(1);
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
    let screen = sing_screen(1);
    let list = draw(&screen, 11.0);
    let outlined = list.commands().iter().any(|c| match c {
        Command::Text { text, style, .. } => text == "ver " && style.outline.is_some(),
        _ => false,
    });
    assert!(outlined, "the lyrics have no outline");
}

#[test]
fn what_was_sung_is_drawn_over_the_notes() {
    let mut screen = sing_screen(1);
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
    let screen = sing_screen(2);
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &theme.resolve_default(),
        &NoteLine::default(),
        &[],
        "",
        0.0,
    );
    assert!(list.is_balanced());
    assert!(!list.is_empty());
}

#[test]
fn the_progress_bar_only_appears_once_the_length_is_known() {
    let mut screen = sing_screen(1);
    screen.duration = 0.0;
    let without = draw(&screen, 10.0).len();

    screen.duration = 200.0;
    screen.position = 100.0;
    let with = draw(&screen, 10.0).len();
    assert!(with > without, "no progress bar was drawn");
}

/// The note bars, matched by the colours only notes are drawn in.
///
/// Matching on shape alone also catches the score bars and the progress bar, which is how the
/// first version of this helper measured a note as most of the screen.
fn bars(list: &DrawList, style: &rungstar_ui::Style) -> Vec<Rect> {
    let note = style.muted.alpha(0.8);
    let golden = style.warning;
    list.commands()
        .iter()
        .filter_map(|c| match c {
            // Rounded, because notes are panels; the gate marker on the input strip is the
            // same warning colour but drawn as a square fill.
            Command::Rect {
                rect,
                color,
                radius,
            } if *radius > 0.0 && (*color == note || *color == golden) => Some(*rect),
            // Freestyle notes are outlines rather than bars — still notes, still drawn.
            Command::Outline { rect, color, .. }
                if *color == note.alpha(0.4) || *color == golden.alpha(0.4) =>
            {
                Some(*rect)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_note_sits_at_the_same_height_whatever_else_is_on_screen() {
    // The scale is the song's, not the visible window's. Deriving it from whatever happens to
    // be on screen makes a note jump vertically as the view moves, so you cannot tell whether
    // you are above or below it — which is the one thing a pitch display is for.
    let theme = Theme::builtin();
    let style = theme.resolve_default();

    let height_of = |others: &[i32]| -> f32 {
        let mut notes = vec![Note {
            start: 8.0,
            duration: 3.0,
            pitch: 62,
            kind: NoteKind::Normal,
        }];
        for (index, pitch) in others.iter().enumerate() {
            notes.push(Note {
                start: 12.0 + index as f64 * 4.0,
                duration: 3.0,
                pitch: *pitch,
                kind: NoteKind::Normal,
            });
        }
        let line = NoteLine {
            start: 8.0,
            end: notes.last().map(Note::end).unwrap_or(11.0),
            notes,
        };
        let screen = sing_screen(1);
        let mut list = DrawList::new();
        screen.draw(&mut list, area(), &style, &line, &[], "", 9.0);
        // The first bar drawn is the note at pitch 62.
        bars(&list, &style)
            .first()
            .map(|r| r.y)
            .expect("a note was drawn")
    };

    // The same note, in the company of a narrow range and then a wide one.
    let narrow = height_of(&[63, 61]);
    let wide = height_of(&[66, 60, 65, 61]);
    assert!(
        (narrow - wide).abs() < 1.0,
        "the note moved from {narrow} to {wide} when its neighbours changed"
    );
}

#[test]
fn the_whole_line_is_on_screen_at_once() {
    // A scrolling window shows a few beats at a time, so a note and the mark you made on it
    // are never both visible long enough to compare. The line is laid out across the width
    // instead, and the playhead sweeps it.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let screen = sing_screen(1);
    let line = line();

    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &style, &line, &syllables(), "", 10.0);
    let drawn = bars(&list, &style);
    assert!(
        drawn.len() >= line.notes.len(),
        "only {} of {} notes were drawn",
        drawn.len(),
        line.notes.len()
    );

    // And the same notes are in the same places later in the line: they do not move.
    let mut later = DrawList::new();
    screen.draw(&mut later, area(), &style, &line, &syllables(), "", 34.0);
    let moved = bars(&later, &style);
    for (before, after) in drawn.iter().zip(moved.iter()) {
        assert!(
            (before.x - after.x).abs() < 0.5 && (before.y - after.y).abs() < 0.5,
            "a note moved between {before:?} and {after:?} as the beat advanced"
        );
    }
}

#[test]
fn the_playhead_sweeps_from_left_to_right_across_the_line() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let screen = sing_screen(1);
    let line = line();

    // The playhead is the tall thin accent bar.
    let head_x = |beat: f64| -> f32 {
        let mut list = DrawList::new();
        screen.draw(&mut list, area(), &style, &line, &[], "", beat);
        list.commands()
            .iter()
            .filter_map(|c| match c {
                Command::Rect { rect, color, .. }
                    if rect.w < 5.0 && rect.h > 100.0 && *color == style.accent.alpha(0.85) =>
                {
                    Some(rect.x)
                }
                _ => None,
            })
            .next()
            .expect("no playhead was drawn")
    };

    let early = head_x(line.start);
    let middle = head_x((line.start + line.end) / 2.0);
    let late = head_x(line.end);
    assert!(early < middle, "playhead went {early} then {middle}");
    assert!(middle < late, "playhead went {middle} then {late}");
}

#[test]
fn what_was_sung_stays_put_until_the_line_turns() {
    // The reason for a static line: the mark you made has to still be there when you look at
    // it. With a scrolling window it has already left the screen.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(1);
    let line = line();
    screen.singers[0].sung = vec![Sung {
        start: 8.0,
        duration: 3.0,
        pitch: 60,
        hit: true,
    }];

    let position = |beat: f64| -> Option<Rect> {
        let mut list = DrawList::new();
        screen.draw(&mut list, area(), &style, &line, &[], "", beat);
        // The sung bar is drawn in the player's colour.
        list.commands()
            .iter()
            .filter_map(|c| match c {
                Command::Rect { rect, color, .. } if *color == style.player(0) => Some(*rect),
                _ => None,
            })
            .next()
    };

    let early = position(10.0).expect("the sung note was not drawn");
    let late = position(38.0).expect("the sung note disappeared later in the line");
    assert!(
        (early.x - late.x).abs() < 0.5,
        "the sung mark moved from {early:?} to {late:?}"
    );
}

#[test]
fn a_narrow_line_does_not_fill_the_staff() {
    // A line covering three semitones should look like three semitones, not an octave. This
    // is the same defect as the moving heights, seen from the other side.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(1);
    // Two octaves of song range.
    screen.pitch_low = 48;
    screen.pitch_high = 72;

    let notes: Vec<Note> = [60, 61, 62]
        .iter()
        .enumerate()
        .map(|(i, pitch)| Note {
            start: 8.0 + i as f64 * 4.0,
            duration: 3.0,
            pitch: *pitch,
            kind: NoteKind::Normal,
        })
        .collect();
    let line = NoteLine {
        start: 8.0,
        end: 19.0,
        notes,
    };

    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &style, &line, &[], "", 10.0);
    let drawn = bars(&list, &style);
    let (top, bottom) = drawn.iter().fold((f32::MAX, f32::MIN), |(t, b), r| {
        (t.min(r.y), b.max(r.bottom()))
    });
    // Three semitones out of a twenty-eight row staff is about a ninth of it.
    let staff_height = area().h;
    assert!(
        (bottom - top) < staff_height * 0.35,
        "three semitones covered {} of a {staff_height} tall screen",
        bottom - top
    );
}

#[test]
fn folding_puts_a_sung_pitch_in_the_octave_it_was_scored_in() {
    // Matching is octave-agnostic, so singing the right note an octave down scores. The
    // detector reports the octave it heard, and drawing that raw put the marker twelve
    // semitones from the note it had just scored against.
    assert_eq!(fold_to_octave(48, 60), 60, "an octave below should fold up");
    assert_eq!(
        fold_to_octave(72, 60),
        60,
        "an octave above should fold down"
    );
    assert_eq!(fold_to_octave(60, 60), 60);
    // Two octaves, and three.
    assert_eq!(fold_to_octave(36, 60), 60);
    assert_eq!(fold_to_octave(96, 62), 60);
    // A genuinely wrong note stays wrong. It may be moved into the other octave — 67 against
    // 60 is seven semitones up, so it folds to 55, five semitones down — but it must not
    // become the target.
    assert_ne!(fold_to_octave(55, 60), 60);
    assert_ne!(fold_to_octave(67, 60), 60);
    assert_eq!(
        fold_to_octave(67, 60),
        55,
        "the shorter way round is five below"
    );
    // The fold always lands within half an octave of the target, which is the window the
    // scorer compares in.
    for target in 40..80 {
        for sung in 0..128 {
            let folded = fold_to_octave(sung, target);
            assert!(
                (folded - target).abs() <= 6,
                "{sung} against {target} folded to {folded}"
            );
            assert_eq!(
                folded.rem_euclid(12),
                sung.rem_euclid(12),
                "folding changed the pitch class"
            );
        }
    }
}

#[test]
fn a_sung_note_an_octave_out_is_drawn_on_the_note_it_scored() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let line = NoteLine {
        start: 8.0,
        end: 11.0,
        notes: vec![Note {
            start: 8.0,
            duration: 3.0,
            pitch: 60,
            kind: NoteKind::Normal,
        }],
    };

    let marker_y = |sung_pitch: i32| -> f32 {
        let mut screen = sing_screen(1);
        screen.singers[0].sung = vec![Sung {
            start: 8.0,
            duration: 3.0,
            pitch: sung_pitch,
            hit: true,
        }];
        let mut list = DrawList::new();
        screen.draw(&mut list, area(), &style, &line, &[], "", 9.0);
        list.commands()
            .iter()
            .filter_map(|c| match c {
                Command::Rect { rect, color, .. } if *color == style.player(0) => Some(rect.y),
                _ => None,
            })
            .next()
            .expect("the sung note was not drawn")
    };

    // Sung on the note, and sung an octave below it: both scored, so both must be drawn in
    // the same place.
    assert!(
        (marker_y(60) - marker_y(48)).abs() < 1.0,
        "an octave below was drawn at {} instead of {}",
        marker_y(48),
        marker_y(60)
    );
    assert!((marker_y(60) - marker_y(72)).abs() < 1.0);
}

#[test]
fn the_lyric_bar_arrives_before_the_first_word_and_sweeps_with_it() {
    // Knowing when to come in is most of singing a song you half-remember, so the bar enters
    // ahead of the first syllable rather than appearing once you are already late.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let screen = sing_screen(1);
    let words = syllables();
    let first = words[0].start;
    let last = words.last().map(|s| s.start + s.duration).unwrap();

    // The bar is the narrow accent panel in the lyric strip.
    let bar_x = |beat: f64| -> Option<f32> {
        let mut list = DrawList::new();
        screen.draw(&mut list, area(), &style, &line(), &words, "", beat);
        list.commands()
            .iter()
            .filter_map(|c| match c {
                Command::Rect { rect, color, .. }
                    if rect.w < 6.0 && *color == style.accent.alpha(0.9) =>
                {
                    Some(rect.x)
                }
                _ => None,
            })
            .next()
    };

    let before = bar_x(first - 3.0).expect("no bar during the lead-in");
    let at_start = bar_x(first).expect("no bar at the first syllable");
    let middle = bar_x((first + last) / 2.0).expect("no bar mid line");
    let end = bar_x(last).expect("no bar at the end");

    assert!(
        before < at_start,
        "the bar did not lead in: {before} then {at_start}"
    );
    assert!(
        at_start < middle,
        "the bar did not sweep: {at_start} then {middle}"
    );
    assert!(middle < end, "the bar stalled: {middle} then {end}");
}
