//! The sing screen. Driven by state and read back as commands, like every other screen.

use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::geom::Rect;
use rungstar_ui::screen::Transition;
use rungstar_ui::settings::LyricEffect;
use rungstar_ui::singscreen::{
    fold_to_octave, rating_title, Note, NoteKind, NoteLine, Overlay, PartView, PauseChoice,
    SingScreen, Singer, Sung, Syllable,
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
            part: 0,
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

fn draw(screen: &mut SingScreen, beat: f64) -> DrawList {
    let theme = Theme::builtin();
    let mut list = DrawList::new();
    let line = line();
    let words = syllables();
    screen.draw(
        &mut list,
        area(),
        &theme.resolve_default(),
        &[PartView {
            line: &line,
            syllables: &words,
            next_line: "the next line",
        }],
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
        let mut screen = sing_screen(count);
        assert_eq!(screen.singers.len(), count);
        let list = draw(&mut screen, 10.0);
        assert!(list.is_balanced(), "{count} singers left a clip pushed");
        assert!(!list.is_empty());
    }
}

#[test]
fn every_singer_gets_a_panel_and_a_distinct_colour() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(6);
    let list = draw(&mut screen, 10.0);
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
                screen.draw(
                    &mut list,
                    area,
                    &style,
                    &[PartView {
                        line: &line(),
                        syllables: &syllables(),
                        next_line: "next",
                    }],
                    20.0,
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

    let list = draw(&mut screen, 200.0);
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

    let text = strings(&draw(&mut screen, 10.0)).join(" ");
    assert!(text.contains("no microphone"), "not reported: {text}");
}

#[test]
fn the_level_meter_never_vanishes() {
    // It used to be drawn only while something was wrong, so it disappeared the moment you
    // sang loudly enough — the reading gone exactly when it turned good.
    let theme = Theme::builtin();
    let style = theme.resolve_default();

    let meters = |level: f32| -> usize {
        let mut screen = sing_screen(1);
        screen.show_input_panel = false;
        screen.singers[0].has_microphone = true;
        screen.singers[0].ever_heard = true;
        screen.singers[0].gate = 0.1;
        screen.singers[0].level = level;
        let list = draw(&mut screen, 10.0);
        // The meter track is the sunken bar in the singer panel.
        list.commands()
            .iter()
            .filter(|c| matches!(c, Command::Rect { color, .. } if *color == style.surface_sunken))
            .count()
    };

    assert!(meters(0.02) > 0, "no meter while too quiet");
    assert!(
        meters(0.5) > 0,
        "the meter vanished once the gate was cleared"
    );
    assert_eq!(
        meters(0.02),
        meters(0.5),
        "the meter came and went with the level"
    );
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

    let text = strings(&draw(&mut screen, 10.0)).join(" ");
    assert!(
        !text.contains("listening"),
        "the panel drew uninvited: {text}"
    );

    screen.show_input_panel = true;
    let text = strings(&draw(&mut screen, 10.0)).join(" ");
    assert!(text.contains('D'), "the detected note is not shown: {text}");
}

#[test]
fn the_lyric_being_sung_is_drawn_differently_from_the_rest() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    // Beat 11 is inside the second syllable, which starts at 10 and lasts 2.
    let mut screen = sing_screen(1);
    let list = draw(&mut screen, 11.0);

    let colours: Vec<(String, rungstar_ui::Color)> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { text, style, .. } => Some((text.clone(), style.color)),
            _ => None,
        })
        .collect();

    // Slide draws the active syllable twice — a resting layer and an accented one clipped to
    // how much of it has been sung — so it is enough that one of them is the accent.
    assert!(
        colours
            .iter()
            .any(|(t, c)| t == "ver " && *c == style.accent),
        "the active syllable is never drawn in the accent: {colours:?}"
    );
    assert!(
        !colours
            .iter()
            .any(|(t, c)| t == "na " && *c == style.accent),
        "an unsung syllable was drawn as active"
    );
}

#[test]
fn lyrics_are_outlined_so_they_survive_a_background() {
    // Lyrics sit over artwork and video, where an outline is the difference between readable
    // and not.
    let mut screen = sing_screen(1);
    let list = draw(&mut screen, 11.0);
    let outlined = list.commands().iter().any(|c| match c {
        Command::Text { text, style, .. } => text == "ver " && style.outline.is_some(),
        _ => false,
    });
    assert!(outlined, "the lyrics have no outline");
}

#[test]
fn what_was_sung_is_drawn_over_the_notes() {
    let mut screen = sing_screen(1);
    let plain = draw(&mut screen, 10.0).len();

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
    let with_sung = draw(&mut screen, 10.0).len();
    assert!(
        with_sung > plain,
        "what was sung was not drawn: {plain} then {with_sung}"
    );
}

#[test]
fn a_song_with_no_notes_still_draws() {
    // Real libraries contain them, and a screen that panics on one is worse than a blank one.
    let theme = Theme::builtin();
    let mut screen = sing_screen(2);
    let mut list = DrawList::new();
    let empty = NoteLine::default();
    screen.draw(
        &mut list,
        area(),
        &theme.resolve_default(),
        &[PartView {
            line: &empty,
            syllables: &[],
            next_line: "",
        }],
        0.0,
    );
    assert!(list.is_balanced());
    assert!(!list.is_empty());
}

#[test]
fn the_progress_bar_only_appears_once_the_length_is_known() {
    let mut screen = sing_screen(1);
    screen.duration = 0.0;
    let without = draw(&mut screen, 10.0).len();

    screen.duration = 200.0;
    screen.position = 100.0;
    let with = draw(&mut screen, 10.0).len();
    assert!(with > without, "no progress bar was drawn");
}

/// The widest rectangle drawn in `color`.
///
/// The singer panel also draws a score bar in the player's colour, and it is zero wide at a
/// score of zero — so taking the first match found that instead, and two tests comparing
/// positions across variants were quietly comparing the same unmoving bar.
fn widest(list: &DrawList, color: rungstar_ui::Color) -> Rect {
    list.commands()
        .iter()
        .filter_map(|c| match c {
            Command::Rect { rect, color: c, .. } if *c == color => Some(*rect),
            _ => None,
        })
        .max_by(|a, b| a.w.partial_cmp(&b.w).unwrap_or(std::cmp::Ordering::Equal))
        .expect("nothing was drawn in that colour")
}

/// The note bars, matched by the colours only notes are drawn in.
///
/// Matching on shape alone also catches the score bars and the progress bar, which is how the
/// first version of this helper measured a note as most of the screen.
fn bars(list: &DrawList, style: &rungstar_ui::Style) -> Vec<Rect> {
    let solo_fill = style.muted.alpha(0.8);
    let solo_freestyle = style.muted.alpha(0.35);
    // A duet outlines each note in its part's colour and fills it faintly; a solo fills it and
    // outlines only the freestyle ones. Either way this yields one rectangle per note.
    let owners: Vec<rungstar_ui::Color> = (0..6)
        .map(|i| style.player(i))
        .chain(std::iter::once(style.text))
        .collect();
    let owned = |colour: &rungstar_ui::Color, alpha: f32| {
        owners.iter().any(|owner| owner.alpha(alpha) == *colour)
    };

    list.commands()
        .iter()
        .filter_map(|c| match c {
            // Rounded, because notes are panels; the gate marker on the input strip is the
            // same warning colour but drawn as a square fill.
            Command::Rect {
                rect,
                color,
                radius,
            } if *radius > 0.0 && (*color == solo_fill || *color == style.warning) => Some(*rect),
            Command::Outline { rect, color, .. }
                if *color == solo_freestyle || owned(color, 0.85) || owned(color, 0.35) =>
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
            part: 0,
        }];
        for (index, pitch) in others.iter().enumerate() {
            notes.push(Note {
                start: 12.0 + index as f64 * 4.0,
                duration: 3.0,
                pitch: *pitch,
                kind: NoteKind::Normal,
                part: 0,
            });
        }
        let line = NoteLine {
            start: 8.0,
            end: notes.last().map(Note::end).unwrap_or(11.0),
            notes,
        };
        let mut screen = sing_screen(1);
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line,
                syllables: &[],
                next_line: "",
            }],
            9.0,
        );
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
    let mut screen = sing_screen(1);
    let line = line();

    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line,
            syllables: &syllables(),
            next_line: "",
        }],
        10.0,
    );
    let drawn = bars(&list, &style);
    assert!(
        drawn.len() >= line.notes.len(),
        "only {} of {} notes were drawn",
        drawn.len(),
        line.notes.len()
    );

    // And the same notes are in the same places later in the line: they do not move.
    let mut later = DrawList::new();
    screen.draw(
        &mut later,
        area(),
        &style,
        &[PartView {
            line: &line,
            syllables: &syllables(),
            next_line: "",
        }],
        34.0,
    );
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
    let mut screen = sing_screen(1);
    let line = line();

    // The playhead is the tall thin accent bar.
    let mut head_x = |beat: f64| -> f32 {
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line,
                syllables: &[],
                next_line: "",
            }],
            beat,
        );
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

    let mut position = |beat: f64| -> Option<Rect> {
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line,
                syllables: &[],
                next_line: "",
            }],
            beat,
        );
        // The sung bar is drawn in the player's colour, and is the widest thing in it.
        Some(widest(&list, style.player(0)))
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
            part: 0,
        })
        .collect();
    let line = NoteLine {
        start: 8.0,
        end: 19.0,
        notes,
    };

    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line,
            syllables: &[],
            next_line: "",
        }],
        10.0,
    );
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
            part: 0,
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
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line,
                syllables: &[],
                next_line: "",
            }],
            9.0,
        );
        widest(&list, style.player(0)).y
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
    let mut screen = sing_screen(1);
    let words = syllables();
    let first = words[0].start;
    let last = words.last().map(|s| s.start + s.duration).unwrap();

    // The bar is the narrow accent panel in the lyric strip.
    let mut bar_x = |beat: f64| -> Option<f32> {
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line(),
                syllables: &words,
                next_line: "",
            }],
            beat,
        );
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

#[test]
fn a_hit_is_drawn_on_the_note_even_when_it_was_a_semitone_off() {
    // Difficulty allows up to two semitones either side, so an honest hit can be a semitone
    // away. Drawing it where the singer actually was puts the blue bar beside the bubble it
    // just scored, which reads as a bug rather than as generosity.
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
            part: 0,
        }],
    };

    let marker_y = |pitch: i32, hit: bool| -> f32 {
        let mut screen = sing_screen(1);
        screen.singers[0].sung = vec![Sung {
            start: 8.0,
            duration: 3.0,
            pitch,
            hit,
        }];
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line,
                syllables: &[],
                next_line: "",
            }],
            9.0,
        );
        let wanted = if hit { style.player(0) } else { style.danger };
        widest(&list, wanted).y
    };

    let exact = marker_y(60, true);
    for off in [59, 61, 58, 62] {
        assert!(
            (marker_y(off, true) - exact).abs() < 1.0,
            "a hit at {off} was drawn away from the note it scored"
        );
    }

    // A miss still shows where the singer really was — that is what a miss tells you.
    assert!(
        (marker_y(64, false) - exact).abs() > 1.0,
        "a miss was snapped onto the note as well"
    );
}

#[test]
fn the_lyric_bar_runs_in_from_the_edge_of_the_screen() {
    // Starting beside the first word, or at the same moment the notes appear, is a run-up
    // that is over before it has been noticed.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(1);
    let words = syllables();
    let first = words[0].start;

    let mut bar_x = |beat: f64| -> f32 {
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line(),
                syllables: &words,
                next_line: "",
            }],
            beat,
        );
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
            .expect("no bar was drawn")
    };

    // Well before the line, the bar is at the very left of the screen.
    let earliest = bar_x(first - 16.0);
    assert!(
        earliest <= area().x + 4.0,
        "the bar started at {earliest} rather than the screen edge"
    );

    // And it is already well on its way by the time the words are due, rather than arriving
    // with them.
    let at_start = bar_x(first);
    let halfway = bar_x(first - 8.0);
    assert!(
        halfway > earliest && halfway < at_start,
        "the run-up did not progress: {earliest}, {halfway}, {at_start}"
    );
}

#[test]
fn a_hit_fills_the_bubble_it_landed_in() {
    // The two should read as one shape lighting up, not as a bar that happens to overlap a
    // bubble. A miss stays a thin mark, because it belongs to no bubble and drawing it as one
    // would claim it did.
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
            part: 0,
        }],
    };

    let mark = |hit: bool| -> Rect {
        let mut screen = sing_screen(1);
        screen.singers[0].sung = vec![Sung {
            start: 8.0,
            duration: 3.0,
            pitch: 60,
            hit,
        }];
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line,
                syllables: &[],
                next_line: "",
            }],
            9.0,
        );
        let wanted = if hit { style.player(0) } else { style.danger };
        widest(&list, wanted)
    };

    let note = bars(
        &{
            let mut screen = sing_screen(1);
            let mut list = DrawList::new();
            screen.draw(
                &mut list,
                area(),
                &style,
                &[PartView {
                    line: &line,
                    syllables: &[],
                    next_line: "",
                }],
                9.0,
            );
            list
        },
        &style,
    )[0];

    let hit = mark(true);
    assert!(
        (hit.h - note.h).abs() < 0.5 && (hit.y - note.y).abs() < 0.5,
        "a hit was drawn as {hit:?} against a note of {note:?}"
    );

    let miss = mark(false);
    assert!(
        miss.h < note.h * 0.75,
        "a miss filled the bubble as well: {miss:?} against {note:?}"
    );
}

#[test]
fn the_pause_menu_takes_the_mouse() {
    // Every other menu accepts a click, so one that does not just reads as broken.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(1);
    screen.handle(Input::Back);
    assert_eq!(screen.overlay, Overlay::Paused);

    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line(),
            syllables: &[],
            next_line: "",
        }],
        10.0,
    );

    // The rows are the wide panels in the centred card. Probe down the middle of the screen
    // until one of them takes the pointer.
    let mut moved = None;
    for row in 0..24 {
        let point = rungstar_ui::geom::Point::new(area().center().x, 300.0 + row as f32 * 25.0);
        let before = screen.pause_cursor();
        screen.handle(Input::Hover(point));
        if screen.pause_cursor() != before {
            moved = Some(point);
            break;
        }
    }
    let point = moved.expect("no pause row responded to the pointer");

    // Clicking the row under the pointer chooses it.
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line(),
            syllables: &[],
            next_line: "",
        }],
        10.0,
    );
    let (_, choice) = screen.handle(Input::Click(point));
    assert!(choice.is_some(), "a click on a pause row chose nothing");
}

#[test]
fn a_click_dismisses_the_results() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(2);
    screen.overlay = Overlay::Results;
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line(),
            syllables: &[],
            next_line: "",
        }],
        10.0,
    );

    let (transition, _) = screen.handle(Input::Click(area().center()));
    assert_eq!(
        transition,
        Transition::Pop,
        "a click did not dismiss the results"
    );
}

#[test]
fn the_song_itself_ignores_the_pointer() {
    // There is nothing on the staff to click, and a stray click during a song must not do
    // anything at all.
    let mut screen = sing_screen(1);
    assert_eq!(screen.overlay, Overlay::None);
    let (transition, choice) = screen.handle(Input::Click(area().center()));
    assert_eq!(transition, Transition::None);
    assert_eq!(choice, None);
    assert_eq!(screen.overlay, Overlay::None, "a click paused the song");
}

#[test]
fn confirm_does_nothing_until_the_last_note_has_gone() {
    // A stray press must never end a song that is still being sung.
    let mut screen = sing_screen(1);
    assert!(!screen.outro);
    let (transition, choice) = screen.handle(Input::Confirm);
    assert_eq!(transition, Transition::None);
    assert_eq!(choice, None);
    assert_eq!(screen.overlay, Overlay::None);
}

#[test]
fn confirm_skips_the_outro_once_there_is_nothing_left_to_sing() {
    // An instrumental tail is part of the song and worth hearing, but sitting through forty
    // seconds of it with nothing to do is another matter — and the only way out was Give up,
    // which used to throw the score away.
    let mut screen = sing_screen(1);
    screen.outro = true;
    let (_, choice) = screen.handle(Input::Confirm);
    assert_eq!(choice, Some(PauseChoice::SkipOutro));

    // And Enter does the same, since it is the key the hint names.
    let mut screen = sing_screen(1);
    screen.outro = true;
    let (_, choice) = screen.handle(Input::Submit);
    assert_eq!(choice, Some(PauseChoice::SkipOutro));
}

#[test]
fn the_outro_says_how_to_skip_it() {
    // A shortcut nobody is told about is a shortcut nobody uses.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(1);

    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line(),
            syllables: &syllables(),
            next_line: "",
        }],
        10.0,
    );
    let quiet = strings(&list).join(" ");
    assert!(!quiet.contains("your score"), "offered mid-song: {quiet}");

    screen.outro = true;
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line(),
            syllables: &syllables(),
            next_line: "",
        }],
        10.0,
    );
    let offered = strings(&list).join(" ");
    assert!(offered.contains("for your score"), "not offered: {offered}");
}

#[test]
fn each_lyric_effect_draws_something_different() {
    // Five entries in the settings that all did the same thing would be five lies. Each one
    // has to change the picture.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let words = syllables();

    let frame = |effect: LyricEffect| -> DrawList {
        let mut screen = sing_screen(1);
        screen.effect = effect;
        let mut list = DrawList::new();
        // Part way through the second syllable, so an effect has something to act on.
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line(),
                syllables: &words,
                next_line: "",
            }],
            11.0,
        );
        list
    };

    let effects = [
        LyricEffect::Simple,
        LyricEffect::Zoom,
        LyricEffect::Slide,
        LyricEffect::Ball,
        LyricEffect::Shift,
    ];
    let frames: Vec<DrawList> = effects.iter().map(|e| frame(*e)).collect();

    for (i, a) in frames.iter().enumerate() {
        for (j, b) in frames.iter().enumerate().skip(i + 1) {
            assert!(
                a != b,
                "{:?} and {:?} draw the same thing",
                effects[i],
                effects[j]
            );
        }
    }
}

#[test]
fn zoom_swells_the_syllable_being_sung() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let words = syllables();

    let size_of = |beat: f64, wanted: &str| -> f32 {
        let mut screen = sing_screen(1);
        screen.effect = LyricEffect::Zoom;
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line(),
                syllables: &words,
                next_line: "",
            }],
            beat,
        );
        list.commands()
            .iter()
            .filter_map(|c| match c {
                Command::Text { text, style, .. } if text == wanted => Some(style.size),
                _ => None,
            })
            .next()
            .expect("the syllable was not drawn")
    };

    // Mid-syllable it is larger than the same syllable before its turn.
    let resting = size_of(9.0, "ver ");
    let swollen = size_of(11.0, "ver ");
    assert!(
        swollen > resting * 1.05,
        "zoom did not swell: {resting} then {swollen}"
    );
}

#[test]
fn shift_keeps_the_syllable_being_sung_in_the_middle() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let words = syllables();

    let centre_of = |beat: f64, wanted: &str| -> f32 {
        let mut screen = sing_screen(1);
        screen.effect = LyricEffect::Shift;
        let mut list = DrawList::new();
        screen.draw(
            &mut list,
            area(),
            &style,
            &[PartView {
                line: &line(),
                syllables: &words,
                next_line: "",
            }],
            beat,
        );
        list.commands()
            .iter()
            .filter_map(|c| match c {
                Command::Text { text, rect, .. } if text == wanted => Some(rect.center().x),
                _ => None,
            })
            .next()
            .expect("the syllable was not drawn")
    };

    // Whichever syllable is due sits in the same place — the middle of the lyric strip,
    // which is not the middle of the screen because the singer panels take the right edge.
    // The property that matters is that the spot does not move, so the eye never has to
    // find it again.
    let anchor = centre_of(9.0, "Ne");
    for (beat, word) in [(11.0, "ver "), (13.0, "gon"), (15.0, "na ")] {
        let at = centre_of(beat, word);
        assert!(
            (at - anchor).abs() < 2.0,
            "at beat {beat}, {word:?} sat at {at} rather than the anchor {anchor}"
        );
    }
    // And it really is roughly central, not pinned to an edge.
    assert!(anchor > area().w * 0.25 && anchor < area().w * 0.75);
}

#[test]
fn a_duet_shares_one_staff_and_colours_the_notes_by_part() {
    // Two stacked staves was the first attempt. It halves the height of both, and a duet
    // spends most of its time with only one part singing, so half the screen sits empty while
    // the other half is cramped. One staff, coloured by whose the note is, reads better.
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(2);
    screen.parts = vec!["Freddie".to_owned(), "Bowie".to_owned()];
    screen.singer_part = vec![0, 1];
    assert!(screen.is_duet());

    // Two parts at different pitches, plus one beat they both sing.
    let mine = NoteLine {
        start: 8.0,
        end: 20.0,
        notes: vec![
            Note {
                start: 8.0,
                duration: 3.0,
                pitch: 60,
                kind: NoteKind::Normal,
                part: 0,
            },
            Note {
                start: 16.0,
                duration: 3.0,
                pitch: 62,
                kind: NoteKind::Normal,
                part: 0,
            },
        ],
    };
    let theirs = NoteLine {
        start: 8.0,
        end: 20.0,
        notes: vec![
            Note {
                start: 12.0,
                duration: 3.0,
                pitch: 65,
                kind: NoteKind::Normal,
                part: 0,
            },
            Note {
                start: 16.0,
                duration: 3.0,
                pitch: 62,
                kind: NoteKind::Normal,
                part: 0,
            },
        ],
    };
    let words = syllables();

    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[
            PartView {
                line: &mine,
                syllables: &words,
                next_line: "one",
            },
            PartView {
                line: &theirs,
                syllables: &words,
                next_line: "two",
            },
        ],
        11.0,
    );
    assert!(list.is_balanced());

    // Each part's own notes are in that singer's colour, and the beat they share is in
    // neither — two bubbles at the same pitch on the same beat is one bubble with a seam.
    let colours: Vec<rungstar_ui::Color> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Outline { color, .. } => Some(*color),
            _ => None,
        })
        .collect();
    assert!(
        colours.contains(&style.player(0).alpha(0.85)),
        "part one's notes are not in its colour"
    );
    assert!(
        colours.contains(&style.player(1).alpha(0.85)),
        "part two's notes are not in its colour"
    );
    assert!(
        colours.contains(&style.text.alpha(0.85)),
        "the shared note is not drawn as shared"
    );
}

#[test]
fn a_duet_shows_both_sets_of_words_in_their_own_colours() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(2);
    screen.parts = vec!["Freddie".to_owned(), "Bowie".to_owned()];
    screen.singer_part = vec![0, 1];

    let first = line();
    let words = syllables();
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[
            PartView {
                line: &first,
                syllables: &words,
                next_line: "one",
            },
            PartView {
                line: &first,
                syllables: &words,
                next_line: "two",
            },
        ],
        11.0,
    );

    // The syllable being sung is highlighted in each part's own colour, so a singer finds
    // their line by its colour rather than by guessing which is theirs.
    let highlights: Vec<rungstar_ui::Color> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { text, style, .. } if text == "ver " => Some(style.color),
            _ => None,
        })
        .collect();
    assert!(
        highlights.contains(&style.player(0)),
        "part one's words are not in its colour: {highlights:?}"
    );
    assert!(
        highlights.contains(&style.player(1)),
        "part two's words are not in its colour"
    );

    // And both lines are at the bottom rather than one being up by the staff.
    let lyric_rows: Vec<f32> = list
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Text { text, rect, .. } if text == "ver " => Some(rect.center().y),
            _ => None,
        })
        .collect();
    assert!(lyric_rows.len() >= 2);
    for y in &lyric_rows {
        assert!(
            *y > area().h * 0.5,
            "a lyric line was drawn at {y}, not at the bottom"
        );
    }
}

#[test]
fn an_ordinary_song_is_not_split() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    let mut screen = sing_screen(1);
    assert!(!screen.is_duet());

    let single = line();
    let words = syllables();
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &single,
            syllables: &words,
            next_line: "",
        }],
        11.0,
    );
    // One staff, using the full height.
    let notes = bars(&list, &style);
    let (top, bottom) = notes.iter().fold((f32::MAX, f32::MIN), |(t, b), r| {
        (t.min(r.y), b.max(r.bottom()))
    });
    assert!(
        bottom - top > area().h * 0.15,
        "a single part was squeezed into a duet's share: {top} to {bottom}"
    );
}

#[test]
fn only_the_singers_the_screen_was_given_get_a_panel() {
    // The panel count comes from the session, which counts scorers, not from the microphone
    // assignment. Two microphones with one person singing is one panel; a second one would
    // sit at zero for the whole song and look like a broken microphone.
    for count in 1..=4 {
        let mut screen = sing_screen(count);
        assert_eq!(screen.singers.len(), count);
        for (index, singer) in screen.singers.iter_mut().enumerate() {
            singer.name = format!("Name{index}");
        }
        let list = draw(&mut screen, 0.0);
        let text = strings(&list);
        for index in 0..count {
            assert!(
                text.iter().any(|t| t == &format!("Name{index}")),
                "singer {index} of {count} had no panel: {text:?}"
            );
        }
        assert!(
            !text.iter().any(|t| t.starts_with("Name") && t[4..].parse::<usize>().is_ok_and(|n| n >= count)),
            "a panel was drawn for a singer that does not exist: {text:?}"
        );
    }
}

#[test]
fn a_panel_shows_the_profile_name_it_was_given() {
    let mut screen = sing_screen(2);
    screen.singers[0].name = "Walki".to_owned();
    screen.singers[1].name = "Ada".to_owned();
    let text = strings(&draw(&mut screen, 0.0));
    assert!(text.iter().any(|t| t == "Walki"));
    assert!(text.iter().any(|t| t == "Ada"));
    assert!(
        !text.iter().any(|t| t.starts_with("Player ")),
        "a named singer was still labelled generically: {text:?}"
    );
}

/// The widest bar in the first player's colour, which is their sung mark.
fn mark(screen: &mut SingScreen, line: &NoteLine, beat: f64, style: &rungstar_ui::Style) -> Rect {
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        style,
        &[PartView {
            line,
            syllables: &[],
            next_line: "",
        }],
        beat,
    );
    widest(&list, style.player(0))
}

#[test]
fn a_mark_grows_in_rather_than_appearing_at_full_width() {
    // Scoring lands a whole beat at a time, and a beat is a wide piece of a staff, so a mark
    // drawn straight from the score jumps across the screen in steps. The drawn end eases.
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut screen = sing_screen(1);

    // The first frame settles the clock; nothing has been sung yet.
    screen.singers[0].sung = vec![];
    mark_or_none(&mut screen, &line, 8.0, &style);

    screen.singers[0].sung = vec![Sung {
        start: 8.0,
        duration: 4.0,
        pitch: 60,
        hit: true,
    }];
    let first = mark(&mut screen, &line, 8.1, &style);
    let second = mark(&mut screen, &line, 8.3, &style);
    let settled = mark(&mut screen, &line, 12.0, &style);

    assert!(
        first.w < second.w,
        "the mark did not grow: {} then {}",
        first.w,
        second.w
    );
    assert!(second.w < settled.w, "the mark stopped growing early");
    assert!(
        first.w < settled.w * 0.75,
        "the first frame already drew most of it: {} of {}",
        first.w,
        settled.w
    );
    // The left edge stays where it was sung. Only the end moves.
    assert!((first.x - settled.x).abs() < 1.0);
}

#[test]
fn a_mark_never_reaches_past_what_was_scored() {
    // The ease approaches the scored end and stops there. Running ahead of it to meet the
    // playhead is what the previous attempt did, and it lurched back when detection landed.
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut screen = sing_screen(1);
    screen.singers[0].sung = vec![Sung {
        start: 8.0,
        duration: 2.0,
        pitch: 60,
        hit: true,
    }];

    let mut previous = 0.0;
    for step in 0..40 {
        let rect = mark(&mut screen, &line, 8.0 + step as f64 * 0.1, &style);
        assert!(rect.w + 0.01 >= previous, "the mark went backwards");
        previous = rect.w;
    }
    // Ten beats of frames later it is exactly two beats long, not three.
    let settled = mark(&mut screen, &line, 20.0, &style);
    let two_beats = settled.w;
    screen.singers[0].sung[0].duration = 4.0;
    let longer = mark(&mut screen, &line, 40.0, &style);
    assert!(
        (longer.w - two_beats * 2.0).abs() < two_beats * 0.1,
        "two beats drew {two_beats} but four drew {}",
        longer.w
    );
}

#[test]
fn a_finished_mark_does_not_move_when_the_next_one_starts() {
    // The frontier is per singer rather than per run: a run that the drawing has caught up
    // with is finished, and must not jump the last few units on when the next run begins.
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut screen = sing_screen(1);
    screen.singers[0].sung = vec![Sung {
        start: 8.0,
        duration: 2.0,
        pitch: 60,
        hit: true,
    }];
    let settled = mark(&mut screen, &line, 20.0, &style);

    screen.singers[0].sung.push(Sung {
        start: 12.0,
        duration: 2.0,
        pitch: 62,
        hit: true,
    });
    // Immediately after, the new run has barely started, so the widest bar is still the first.
    let after = mark(&mut screen, &line, 20.05, &style);
    assert!(
        (after.w - settled.w).abs() < 1.0 && (after.x - settled.x).abs() < 1.0,
        "the finished mark moved: {settled:?} then {after:?}"
    );
}

#[test]
fn a_seek_lands_the_marks_rather_than_scrolling_them_in() {
    // After a pause or a restart the clock jumps. Easing across that would redraw the whole
    // line growing in, which looks like the song replaying itself.
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut screen = sing_screen(1);
    screen.singers[0].sung = vec![Sung {
        start: 8.0,
        duration: 4.0,
        pitch: 60,
        hit: true,
    }];
    let settled = mark(&mut screen, &line, 40.0, &style);

    // A jump backwards, as a restart does.
    let after_restart = mark(&mut screen, &line, 9.0, &style);
    assert!(
        (after_restart.w - settled.w).abs() < 1.0,
        "a seek eased instead of landing: {settled:?} then {after_restart:?}"
    );
}

/// Draw and return the mark if there is one, for frames where nothing has been sung.
fn mark_or_none(
    screen: &mut SingScreen,
    line: &NoteLine,
    beat: f64,
    style: &rungstar_ui::Style,
) -> Option<Rect> {
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        style,
        &[PartView {
            line,
            syllables: &[],
            next_line: "",
        }],
        beat,
    );
    list.commands()
        .iter()
        .filter_map(|c| match c {
            Command::Rect { rect, color, .. } if *color == style.player(0) => Some(*rect),
            _ => None,
        })
        .max_by(|a, b| a.w.partial_cmp(&b.w).unwrap_or(std::cmp::Ordering::Equal))
}

#[test]
fn a_blind_challenge_says_what_it_took_away() {
    // Blank is not the same as hidden. An empty middle of the screen is indistinguishable
    // from a song that failed to load, and somebody who walked in has no way to tell.
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut screen = sing_screen(1);
    screen.challenge = Some("Blind".to_owned());
    screen.show_lyrics = false;
    screen.show_notes = false;

    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        &style,
        &[PartView {
            line: &line,
            syllables: &[Syllable {
                text: "secret".into(),
                start: 8.0,
                duration: 1.0,
                golden: false,
            }],
            next_line: "also secret",
        }],
        8.0,
    );
    assert!(list.is_balanced());
    let text = strings(&list);
    assert!(text.iter().any(|t| t == "Words hidden"), "{text:?}");
    assert!(text.iter().any(|t| t == "Notes hidden"));
    assert!(text.iter().any(|t| t == "Blind"), "the mode is not named");
    assert!(
        !text.iter().any(|t| t.contains("secret")),
        "the words were drawn anyway: {text:?}"
    );
}

#[test]
fn the_music_cutting_out_is_said_rather_than_just_happening() {
    // Silence with no explanation reads as the game having crashed.
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut screen = sing_screen(1);
    screen.challenge = Some("Deaf".to_owned());
    screen.audible = false;
    let text = strings(&draw_with(&mut screen, &line, 8.0, &style));
    assert!(
        text.iter().any(|t| t.contains("music off")),
        "nothing said the music had gone: {text:?}"
    );
}

#[test]
fn the_rising_bar_is_drawn_with_the_number_on_it() {
    // A rule that puts somebody out without showing how close they were cannot be played
    // against, and "you are out" with no warning reads as a bug.
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut screen = sing_screen(2);
    screen.challenge = Some("Hold the line".to_owned());
    screen.bar = Some(0.42);
    let text = strings(&draw_with(&mut screen, &line, 8.0, &style));
    assert!(
        text.iter().any(|t| t.contains("42%")),
        "the bar is not readable: {text:?}"
    );

    // Somebody knocked out keeps their panel and their score, and is marked.
    screen.knocked_out = vec![false, true];
    screen.singers[1].name = "Grace".to_owned();
    screen.singers[1].score = 3100;
    let text = strings(&draw_with(&mut screen, &line, 8.0, &style));
    assert!(text.iter().any(|t| t == "OUT"), "{text:?}");
    assert!(
        text.iter().any(|t| t == "Grace") && text.iter().any(|t| t == "3100"),
        "a knocked-out singer lost the thing they want to look at"
    );
}

#[test]
fn the_jukebox_has_no_panels_and_the_staff_takes_the_room() {
    let style = Theme::builtin().resolve_default();
    let line = line();
    let mut with = sing_screen(1);
    with.singers[0].name = "Walki".to_owned();
    let widest_with = bars(&draw_with(&mut with, &line, 8.0, &style), &style)
        .iter()
        .map(|r| r.right())
        .fold(0.0_f32, f32::max);

    let mut without = sing_screen(1);
    without.singers[0].name = "Walki".to_owned();
    without.show_panels = false;
    let list = draw_with(&mut without, &line, 8.0, &style);
    assert!(
        !strings(&list).iter().any(|t| t == "Walki"),
        "the jukebox drew a singer panel"
    );
    let widest_without = bars(&list, &style)
        .iter()
        .map(|r| r.right())
        .fold(0.0_f32, f32::max);
    assert!(
        widest_without > widest_with,
        "the staff did not take the room the panels left: {widest_with} then {widest_without}"
    );
}

/// Draw one frame of a screen and hand back the list.
fn draw_with(
    screen: &mut SingScreen,
    line: &NoteLine,
    beat: f64,
    style: &rungstar_ui::Style,
) -> DrawList {
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        area(),
        style,
        &[PartView {
            line,
            syllables: &[],
            next_line: "",
        }],
        beat,
    );
    assert!(list.is_balanced());
    list
}
