//! The editor screen: keys turn into operations, and nothing is lost on the way out.

use rungstar_editor::{Editor, Waveform};
use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::editorscreen::{EditorOutcome, EditorScreen, Mode};
use rungstar_ui::geom::Rect;
use rungstar_ui::screen::Transition;
use rungstar_ui::songselect::Input;
use rungstar_ui::theme::Theme;

const SONG: &str = "\
#TITLE:Waterloo
#ARTIST:Abba
#MP3:audio.ogg
#BPM:300
#GAP:1000
: 0 4 60 My~
: 4 4 62 wa
: 8 4 64 ter
- 12
: 16 4 60 loo
: 20 8 62 oo
E
";

fn area() -> Rect {
    Rect::new(0.0, 0.0, 1600.0, 1000.0)
}

fn screen() -> EditorScreen {
    let parsed = rungstar_editor::song::SongTxt::parse_bytes(SONG.as_bytes()).unwrap();
    EditorScreen::new(Editor::over(parsed.song, "song.txt".into()))
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

fn draw(screen: &mut EditorScreen) -> DrawList {
    let mut list = DrawList::new();
    screen.draw(&mut list, area(), &Theme::builtin().resolve_default());
    assert!(list.is_balanced());
    list
}

#[test]
fn the_arrows_walk_the_notes_and_change_the_pitch() {
    // Left and right are time, up and down are pitch, because that is what the picture says
    // they are.
    let mut screen = screen();
    assert_eq!(screen.editor.current().unwrap().text, "My~");
    screen.handle(Input::Right);
    assert_eq!(screen.editor.current().unwrap().text, "wa");
    screen.handle(Input::Up);
    assert_eq!(screen.editor.current().unwrap().pitch, 63);
    screen.handle(Input::Down);
    screen.handle(Input::Down);
    assert_eq!(screen.editor.current().unwrap().pitch, 61);
}

#[test]
fn the_letter_shortcuts_move_and_resize() {
    // One hand on the keyboard and one on the space bar: every modifier is a reason to look
    // down at what you are pressing.
    let mut screen = screen();
    screen.handle(Input::Right);
    screen.handle(Input::Right); // "ter", 8..12, which has room after it
    screen.handle(Input::Type('i'));
    assert_eq!(screen.editor.current().unwrap().duration, 5);
    screen.handle(Input::Type('k'));
    assert_eq!(screen.editor.current().unwrap().duration, 4);
    screen.handle(Input::Type('l'));
    assert_eq!(screen.editor.current().unwrap().start, 9);
    screen.handle(Input::Type('j'));
    assert_eq!(screen.editor.current().unwrap().start, 8);
}

#[test]
fn undo_and_redo_are_a_keystroke() {
    let mut screen = screen();
    screen.handle(Input::Up);
    assert_eq!(screen.editor.current().unwrap().pitch, 61);
    screen.handle(Input::Type('z'));
    assert_eq!(screen.editor.current().unwrap().pitch, 60);
    screen.handle(Input::Type('y'));
    assert_eq!(screen.editor.current().unwrap().pitch, 61);
}

#[test]
fn typing_a_syllable_changes_it_as_it_is_typed() {
    // The point of the picture is watching the word land on the note, so it updates live.
    let mut screen = screen();
    screen.handle(Input::Search);
    assert_eq!(screen.mode(), Mode::Typing);
    assert!(screen.wants_text());
    // It starts with the syllable that is there, because amending one is far more common
    // than replacing it and retyping "Waterloo" a letter at a time is not editing.
    for c in "no".chars() {
        screen.handle(Input::Type(c));
    }
    assert_eq!(screen.editor.current().unwrap().text, "My~no");
    for _ in 0..5 {
        screen.handle(Input::Backspace);
    }
    for c in "Oh".chars() {
        screen.handle(Input::Type(c));
    }
    assert_eq!(screen.editor.current().unwrap().text, "Oh");
    screen.handle(Input::Submit);
    assert_eq!(screen.mode(), Mode::Notes);
    assert_eq!(screen.editor.current().unwrap().text, "Oh");
}

#[test]
fn confirming_plays_from_the_cursor_and_again_stops() {
    // Hearing the edit is the whole loop of timing a song, so it is the easiest thing to do.
    let mut screen = screen();
    screen.handle(Input::PageDown); // the second line, beat 16
    let (_, outcome) = screen.handle(Input::Confirm);
    match outcome {
        EditorOutcome::Play(at) => {
            // gap 1000 ms plus 16 beats at 300 BPM, less half a second of run-up.
            assert!((at - 1.3).abs() < 1e-6, "{at}");
        }
        other => panic!("expected playback, got {other:?}"),
    }
    screen.playing = Some(1.3);
    assert_eq!(screen.handle(Input::Confirm).1, EditorOutcome::Stop);
}

#[test]
fn a_refused_edit_says_why_rather_than_looking_broken() {
    let mut screen = screen();
    screen.handle(Input::Right);
    screen.handle(Input::Type('j')); // into the note before it
    let text = strings(&draw(&mut screen)).join(" ");
    assert!(text.contains("run into"), "{text}");
}

#[test]
fn leaving_with_unsaved_work_offers_to_save_it() {
    let mut screen = screen();
    screen.handle(Input::Up);
    assert!(screen.editor.dirty());

    let (transition, _) = screen.handle(Input::Back);
    assert_eq!(transition, Transition::None, "it left without asking");
    assert_eq!(screen.mode(), Mode::Leaving);
    let text = strings(&draw(&mut screen));
    assert!(text.iter().any(|t| t == "Save this song?"));
    assert!(text.iter().any(|t| t == "Discard"));

    // Save is what the cursor starts on: somebody who pressed Escape with unsaved work
    // almost always meant to keep it.
    let (transition, outcome) = screen.handle(Input::Confirm);
    assert_eq!(outcome, EditorOutcome::Save);
    assert_eq!(transition, Transition::None);

    // And discarding does leave.
    screen.handle(Input::Back);
    screen.handle(Input::Right);
    let (transition, outcome) = screen.handle(Input::Confirm);
    assert_eq!(transition, Transition::Pop);
    assert_eq!(outcome, EditorOutcome::Leave);
}

#[test]
fn leaving_with_nothing_changed_just_leaves() {
    let mut screen = screen();
    let (transition, outcome) = screen.handle(Input::Back);
    assert_eq!(transition, Transition::Pop);
    assert_eq!(outcome, EditorOutcome::Leave);
}

#[test]
fn the_menu_holds_what_is_not_a_keystroke() {
    let mut screen = screen();
    screen.handle(Input::ContextMenu);
    assert_eq!(screen.mode(), Mode::Menu);
    let text = strings(&draw(&mut screen));
    for expected in [
        "Save",
        "Double the tempo",
        "Chorus starts here",
        "Close the editor",
    ] {
        assert!(text.iter().any(|t| t == expected), "{expected}: {text:?}");
    }
    // A song that is not a duet says so rather than offering a part that is not there.
    assert!(text.iter().any(|t| t == "not a duet"));

    // The first row saves.
    let (_, outcome) = screen.handle(Input::Confirm);
    assert_eq!(outcome, EditorOutcome::Save);
    assert_eq!(screen.mode(), Mode::Notes);
}

#[test]
fn doubling_the_tempo_from_the_menu_scales_the_song() {
    let mut screen = screen();
    screen.handle(Input::ContextMenu);
    for _ in 0..3 {
        screen.handle(Input::Down);
    }
    screen.handle(Input::Confirm);
    assert_eq!(screen.editor.song().bpm().value(), 600.0);
    assert_eq!(screen.editor.lines()[0].notes[1].start, 8);
}

#[test]
fn the_waveform_is_drawn_at_the_same_scale_as_the_notes() {
    let mut screen = screen();
    let without = draw(&mut screen).len();
    assert!(strings(&draw(&mut screen))
        .iter()
        .any(|t| t == "no audio to draw"));

    // One second of tone at 1 kHz.
    let samples: Vec<i16> = (0..1000)
        .map(|i| if i % 4 < 2 { 8000 } else { -8000 })
        .collect();
    screen.waveform = Waveform::from_samples(&samples, 1, 1000);
    let with = draw(&mut screen).len();
    assert!(with > without, "the waveform drew nothing");
    assert!(!strings(&draw(&mut screen))
        .iter()
        .any(|t| t == "no audio to draw"));
}

#[test]
fn zooming_changes_how_much_of_the_song_is_on_screen() {
    let mut screen = screen();
    let before = screen.zoom;
    screen.handle(Input::Type('+'));
    assert!(screen.zoom < before);
    screen.handle(Input::Type('-'));
    screen.handle(Input::Type('-'));
    assert!(screen.zoom > before);
    // And it stops rather than going to nothing or to everything.
    for _ in 0..20 {
        screen.handle(Input::Type('-'));
    }
    assert!(rungstar_ui::editorscreen::ZOOM_RANGE.contains(&screen.zoom));
}

#[test]
fn every_mode_stays_inside_the_window() {
    let style = Theme::builtin().resolve_default();
    for (w, h) in [(1778.0, 1000.0), (1600.0, 1000.0), (1333.0, 1000.0)] {
        let bounds = Rect::new(0.0, 0.0, w, h);
        for open in [None, Some(Input::Search), Some(Input::ContextMenu)] {
            let mut screen = screen();
            screen.handle(Input::Up);
            if let Some(input) = open {
                screen.handle(input);
            }
            let mut list = DrawList::new();
            screen.draw(&mut list, bounds, &style);
            assert!(list.is_balanced(), "{open:?} left a clip pushed");
            for command in list.commands() {
                if let Command::Rect { rect, .. } = command {
                    assert!(
                        rect.bottom() <= bounds.bottom() + 1.0
                            && rect.right() <= bounds.right() + 1.0
                            && rect.x >= -1.0
                            && rect.y >= -1.0,
                        "{open:?} left the {w}x{h} window: {rect:?}"
                    );
                }
            }
        }
        // And the leaving question, which only appears with unsaved work.
        let mut screen = screen();
        screen.handle(Input::Up);
        screen.handle(Input::Back);
        let mut list = DrawList::new();
        screen.draw(&mut list, bounds, &style);
        assert!(list.is_balanced());
    }
}

#[test]
fn a_song_with_no_notes_draws_rather_than_panicking() {
    // Real libraries contain them, and an editor that panics on one is where a song stays
    // broken.
    let parsed = rungstar_editor::song::SongTxt::parse_bytes(
        b"#TITLE:Nothing\n#ARTIST:Nobody\n#MP3:a.ogg\n#BPM:300\n#GAP:0\n: 0 1 0 x\nE\n",
    )
    .unwrap();
    let mut screen = EditorScreen::new(Editor::over(parsed.song, "song.txt".into()));
    screen.editor.apply(rungstar_editor::Op::Delete);
    let list = draw(&mut screen);
    assert!(list.is_balanced());
}
