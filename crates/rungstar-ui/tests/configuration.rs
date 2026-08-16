//! Settings and the options pages built from them.

use rungstar_ui::glyphs::{Glyphs, Named};
use rungstar_ui::options::{Action, Control, Page};
use rungstar_ui::settings::{
    Choice, Difficulty, FrameLimit, MicBoost, Settings, Vocals, MAX_PLAYERS, THRESHOLDS,
};

#[test]
fn a_choice_cycles_in_both_directions_and_wraps() {
    assert_eq!(Difficulty::Easy.next(), Difficulty::Medium);
    assert_eq!(Difficulty::Hard.next(), Difficulty::Easy);
    assert_eq!(Difficulty::Easy.previous(), Difficulty::Hard);

    // Stepping through every value returns to the start, for every choice type.
    fn full_circle<C: Choice + std::fmt::Debug>(start: C) {
        let mut value = start;
        for _ in 0..C::VALUES.len() {
            value = value.next();
        }
        assert_eq!(
            value.position(),
            start.position(),
            "{start:?} did not return"
        );
    }
    full_circle(Difficulty::Easy);
    full_circle(MicBoost::Off);
    full_circle(FrameLimit::Sixty);
}

#[test]
fn every_choice_has_a_label_and_they_are_all_different() {
    fn labelled<C: Choice>() {
        let labels = C::labels();
        assert_eq!(labels.len(), C::VALUES.len());
        for label in &labels {
            assert!(!label.is_empty(), "a choice has an empty label");
        }
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "two choices share a label");
    }
    labelled::<Difficulty>();
    labelled::<MicBoost>();
    labelled::<FrameLimit>();
    labelled::<rungstar_ui::browse::Layout>();
    labelled::<Glyphs>();
}

#[test]
fn controller_button_names_match_the_selected_hardware() {
    assert_eq!(Glyphs::Xbox.resolve(), Named::XBOX);
    assert_eq!(Glyphs::Deck.resolve(), Named::DECK);
    assert_eq!(Glyphs::PlayStation.resolve(), Named::PLAYSTATION);
    assert!(matches!(
        Glyphs::Automatic.resolve(),
        Named::XBOX | Named::DECK
    ));
    assert_eq!(Named::DECK.confirm, "A");
    assert_eq!(Named::DECK.left_shoulder, "L1");
    assert_eq!(Named::PLAYSTATION.confirm, "Cross");
}

#[test]
fn settings_round_trip_through_toml() {
    let mut settings = Settings::default();
    settings.game.players = 4;
    settings.game.difficulty = Difficulty::Hard;
    settings.sound.mic_delay_ms = 95;
    settings.appearance.accent = "teal".to_owned();
    settings.appearance.text_scale = 1.25;

    let text = toml::to_string_pretty(&settings).expect("serialise");
    let back: Settings = toml::from_str(&text).expect("parse");
    assert_eq!(back, settings);
}

#[test]
fn a_missing_config_is_the_first_run_and_not_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("does-not-exist.toml");
    let settings = Settings::load(&path).expect("a missing config is normal");
    assert_eq!(settings, Settings::default());
}

#[test]
fn a_corrupt_config_is_reported_rather_than_silently_reset() {
    // Silently resetting somebody's settings and saying nothing is worse than telling them.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.toml");
    std::fs::write(&path, "this is not toml {{{").unwrap();
    assert!(Settings::load(&path).is_err());
}

#[test]
fn saving_and_loading_preserves_everything() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested").join("settings.toml");

    let mut settings = Settings::default();
    settings.game.players = 6;
    settings.game.song_roots = vec!["D:/Songs".to_owned(), "E:/More".to_owned()];
    settings.save(&path).expect("save creates its directory");

    let back = Settings::load(&path).expect("load");
    assert_eq!(back, settings);
    // The temporary file used for the atomic write must not be left behind.
    assert!(!path.with_extension("toml.tmp").exists());
}

#[test]
fn settings_written_by_a_newer_build_survive_a_downgrade() {
    // A config with keys this build does not know must not lose them on the next save, or
    // downgrading once silently discards every newer setting.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.toml");
    std::fs::write(
        &path,
        "[game]\nplayers = 3\n\n[from_the_future]\nholograms = true\n",
    )
    .unwrap();

    let settings = Settings::load(&path).expect("load");
    assert_eq!(settings.game.players, 3);
    settings.save(&path).expect("save");

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("holograms"),
        "an unknown section was dropped:\n{text}"
    );
}

#[test]
fn hand_edited_nonsense_is_clamped_into_range() {
    // Config files get edited by hand, and a zero here is a division by zero three crates
    // away. Clamping on load means nothing downstream has to check.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.toml");
    std::fs::write(
        &path,
        "[game]\nplayers = 99\nlanguage = \"\"\n\n\
         [graphics]\nwidth = 1\nheight = 0\n\n\
         [sound]\nthreshold = 250\nmaster_volume = 200\nmic_delay_ms = 100000\n\n\
         [appearance]\ntext_scale = 40.0\n",
    )
    .unwrap();

    let settings = Settings::load(&path).expect("load");
    assert_eq!(settings.game.players, MAX_PLAYERS);
    assert_eq!(settings.game.language, "en");
    assert!(settings.graphics.width >= 640);
    assert!(settings.graphics.height >= 480);
    assert!((settings.sound.threshold as usize) < THRESHOLDS.len());
    assert!(settings.sound.master_volume <= 100);
    assert!(settings.sound.mic_delay_ms <= 1000);
    assert!(settings.appearance.text_scale <= 1.6);
    // And the clamped value is a usable one, not just an in-range one.
    assert!(settings.threshold() > 0.0);
}

#[test]
fn difficulty_maps_to_the_tolerances_ultrastar_uses() {
    let mut settings = Settings::default();
    for (difficulty, tolerance) in [
        (Difficulty::Easy, 2),
        (Difficulty::Medium, 1),
        (Difficulty::Hard, 0),
    ] {
        settings.game.difficulty = difficulty;
        assert_eq!(settings.tolerance(), tolerance);
    }
}

#[test]
fn every_option_on_every_page_has_a_label_and_an_explanation() {
    // An option a player cannot understand is an option they will not touch. Deriving pages
    // from the settings means an option cannot be added without answering "what is this".
    for page in Page::all() {
        assert!(!page.title.is_empty());
        assert!(!page.items.is_empty(), "page {} is empty", page.title);
        for item in &page.items {
            assert!(
                !item.label.is_empty(),
                "{}: an item has no label",
                page.title
            );
            assert!(
                item.help.len() > 20,
                "{}/{}: help is too short to be useful",
                page.title,
                item.label
            );
        }
    }
}

#[test]
fn no_two_options_share_a_label_within_a_page() {
    // The copy-paste error this catches is two rows that look different but edit the same
    // field, which is invisible until someone changes one and the other moves.
    for page in Page::all() {
        let mut seen = std::collections::HashSet::new();
        for item in &page.items {
            assert!(
                seen.insert(item.label),
                "{} has two rows called {}",
                page.title,
                item.label
            );
        }
    }
}

#[test]
fn stepping_every_option_changes_it_and_stepping_back_restores_it() {
    for page in Page::all() {
        for item in &page.items {
            // A button and a row that shows a folder are both pressed rather than stepped;
            // free text is typed. None of them has a left and a right to come back from.
            if !item.is_adjustable() {
                continue;
            }
            let mut settings = Settings::default();
            let before = item.value(&settings);

            item.adjust(&mut settings, 1);
            let after = item.value(&settings);
            assert_ne!(
                before, after,
                "{}/{} did nothing when stepped",
                page.title, item.label
            );

            item.adjust(&mut settings, -1);
            assert_eq!(
                item.value(&settings),
                before,
                "{}/{} did not come back",
                page.title,
                item.label
            );
        }
    }
}

#[test]
fn a_string_choice_needs_two_values_to_be_adjustable() {
    let mut page = Page::appearance();
    let skin = page
        .items
        .iter_mut()
        .find(|item| item.label == "Skin")
        .unwrap();
    match &mut skin.control {
        Control::StringChoice { choices, .. } => choices.truncate(1),
        _ => panic!("Skin is not a theme choice"),
    }
    assert!(!skin.is_adjustable());
    match &mut skin.control {
        Control::StringChoice { choices, .. } => choices.clear(),
        _ => unreachable!(),
    }
    assert!(!skin.is_adjustable());
}

#[test]
fn numbers_clamp_at_their_ends_rather_than_wrapping() {
    // A volume that jumps from 100 to 0 because the stick was held a moment too long is a
    // genuinely bad surprise; a choice that wraps is not.
    for page in Page::all() {
        for item in &page.items {
            let Control::Number { min, max, .. } = item.control else {
                continue;
            };
            let mut settings = Settings::default();
            for _ in 0..500 {
                item.adjust(&mut settings, 1);
            }
            assert_eq!(item.fraction(&settings), Some(1.0), "{}", item.label);

            for _ in 0..1000 {
                item.adjust(&mut settings, -1);
            }
            assert_eq!(item.fraction(&settings), Some(0.0), "{}", item.label);
            assert!(min < max, "{} has an empty range", item.label);
        }
    }
}

#[test]
fn every_page_of_settings_still_loads_after_being_maximised() {
    // Push every numeric option to its limit, save, and load. This is the combination a
    // player who explores the menus actually produces, and it must round-trip.
    let mut settings = Settings::default();
    for page in Page::all() {
        for item in &page.items {
            for _ in 0..200 {
                item.adjust(&mut settings, 1);
            }
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.toml");
    settings.save(&path).unwrap();
    let back = Settings::load(&path).unwrap();
    assert_eq!(back, settings);
}

#[test]
fn the_buttons_a_page_offers_are_the_ones_a_screen_handles() {
    let buttons: Vec<Action> = Page::all()
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item.control {
            Control::Button(action) => Some(action),
            _ => None,
        })
        .collect();
    // Every action defined must be reachable from some page, or it is dead code that looks
    // like a feature.
    for action in [
        Action::RescanLibrary,
        Action::RebuildIndex,
        Action::AddSongFolder,
        Action::ForgetInstrumentalFolder,
        Action::ManageMicrophones,
        Action::RebindControls,
        Action::ImportUltrastar,
        Action::WipeStatistics,
        Action::ResetToDefaults,
    ] {
        assert!(buttons.contains(&action), "{action:?} is on no page");
    }
}

#[test]
fn the_backing_track_folder_is_a_row_that_says_which_folder_and_changes_it() {
    // Both halves in one row, the way the song folder row works: a caption that cannot be
    // pressed is the bug that row was written to fix.
    let page = Page::game();
    let item = page
        .items
        .iter()
        .find(|item| item.label == "Backing tracks")
        .expect("the game page offers a backing-track folder");
    assert_eq!(item.pressed(), Some(Action::SetInstrumentalFolder));

    let mut settings = Settings::default();
    assert_eq!(item.value(&settings), "None");
    settings.game.instrumental_root = Some("D:/Instrumentals".to_owned());
    assert_eq!(item.value(&settings), "D:/Instrumentals");

    // And it is not a thing left and right can edit -- a path is chosen in a file dialog.
    assert!(!item.is_adjustable());
}

#[test]
fn the_vocals_setting_survives_being_written_and_read_back() {
    let mut settings = Settings::default();
    assert_eq!(settings.game.vocals, Vocals::Original);
    settings.game.vocals = Vocals::Instrumental;
    settings.game.instrumental_root = Some("D:/Instrumentals".to_owned());

    let text = toml::to_string_pretty(&settings).expect("write");
    let read: Settings = toml::from_str(&text).expect("read");
    assert_eq!(read.game.vocals, Vocals::Instrumental);
    assert_eq!(
        read.game.instrumental_root.as_deref(),
        Some("D:/Instrumentals")
    );
}

#[test]
fn every_action_that_cannot_be_undone_asks_first() {
    for action in [Action::WipeStatistics, Action::ResetToDefaults] {
        let Some((question, detail)) = action.confirmation() else {
            panic!("{action:?} destroys something and does not ask");
        };
        assert!(question.ends_with('?'), "{action:?}: {question:?}");
        assert!(
            detail.len() > 40,
            "{action:?} asks without saying what is lost"
        );
    }
    // And the ones that do not destroy anything must not: a confirmation on a harmless
    // button trains people to press through them.
    for action in [
        Action::RescanLibrary,
        Action::AddSongFolder,
        Action::ManageMicrophones,
        Action::ImportUltrastar,
    ] {
        assert!(
            action.confirmation().is_none(),
            "{action:?} asks and does not need to"
        );
    }
}

#[test]
fn a_microphone_keeps_its_own_measured_delay() {
    // One delay for the whole game is wrong the moment two people sing into different
    // hardware: a USB microphone and a Bluetooth headset are hundreds of milliseconds apart,
    // and the delay shifts the whole scoring clock, so whichever singer is on the wrong one
    // sings perfectly and scores badly with nothing on screen to say why.
    let mut sound = Settings::default().sound;
    assert_eq!(
        sound.mic_delay_for("Anything", 0),
        sound.mic_delay_ms,
        "an unmeasured microphone should fall back to the shared value"
    );

    sound.set_mic_delay("Blue Yeti", 0, 120);
    sound.set_mic_delay("Jabra Speak", 0, 310);
    assert_eq!(sound.mic_delay_for("Blue Yeti", 0), 120);
    assert_eq!(sound.mic_delay_for("Jabra Speak", 0), 310);
    // A microphone nobody measured is still on the shared value rather than on somebody
    // else's, which would be worse than the guess it replaced.
    assert_eq!(sound.mic_delay_for("Webcam", 0), sound.mic_delay_ms);

    // Measuring again replaces the answer instead of leaving two.
    sound.set_mic_delay("Blue Yeti", 0, 135);
    assert_eq!(sound.mic_delay_for("Blue Yeti", 0), 135);
    assert_eq!(sound.mic_delays.len(), 2);
}

#[test]
fn two_microphones_of_the_same_model_are_measured_separately() {
    // A pair of identical USB karaoke microphones report identical names, which is the
    // ordinary way somebody ends up with two. Keyed on the name alone, the second one would
    // silently overwrite the first's measurement.
    let mut sound = Settings::default().sound;
    sound.set_mic_delay("USB Microphone", 0, 100);
    sound.set_mic_delay("USB Microphone", 1, 250);
    assert_eq!(sound.mic_delay_for("USB Microphone", 0), 100);
    assert_eq!(sound.mic_delay_for("USB Microphone", 1), 250);
}

#[test]
fn measuring_gives_each_microphone_its_own_delay() {
    // The sweep already measures every microphone separately and then threw all but one
    // answer away, because there was only one place to put it. Keeping them is the whole
    // point of having measured them.
    let mut sound = Settings::default().sound;
    let applied = sound.record_measurements(&[
        ("Blue Yeti".to_owned(), 0, 118.6),
        ("Jabra Speak".to_owned(), 0, 305.2),
        ("USB Mic".to_owned(), 1, 141.0),
    ]);

    assert_eq!(sound.mic_delay_for("Blue Yeti", 0), 119);
    assert_eq!(sound.mic_delay_for("Jabra Speak", 0), 305);
    assert_eq!(sound.mic_delay_for("USB Mic", 1), 141);

    // The shared value stays the fallback and becomes the median of what was heard: the best
    // guess for a microphone that was not in the round. Never the largest, which would make a
    // fast microphone late, nor the smallest, which would make a slow one early.
    assert_eq!(applied, Some(141));
    assert_eq!(sound.mic_delay_ms, 141);
    assert_eq!(sound.mic_delay_for("Never Measured", 0), 141);
}

#[test]
fn a_round_that_heard_nothing_changes_nothing() {
    // Speakers pointing the wrong way, or a dead device. Overwriting a delay somebody has
    // already calibrated with a number nothing supports is worse than leaving it alone.
    let mut sound = Settings::default().sound;
    sound.set_mic_delay("Blue Yeti", 0, 118);
    let before = sound.mic_delay_ms;

    assert_eq!(sound.record_measurements(&[]), None);
    assert_eq!(sound.mic_delay_ms, before);
    assert_eq!(
        sound.mic_delay_for("Blue Yeti", 0),
        118,
        "an earlier answer was lost"
    );
}

#[test]
fn the_measurement_screen_says_each_microphone_keeps_its_own_delay() {
    // The screen used to promise "one value covers every microphone", which was true and is
    // now the opposite of what happened. A screen that reports the old behaviour after the
    // new one has run is worse than one that says nothing: somebody reads it, believes their
    // headset and their USB mic are on the same figure, and stops looking.
    use rungstar_ui::calibratescreen::{CalibrateScreen, Report};
    use rungstar_ui::draw::DrawList;
    use rungstar_ui::geom::Rect;
    use rungstar_ui::theme::Theme;

    let mut screen = CalibrateScreen::new();
    screen.applied = Some(141);
    screen.reports = vec![
        Report {
            name: "Blue Yeti".to_owned(),
            occurrence: 0,
            delay: Ok(119.0),
            passes: Vec::new(),
        },
        Report {
            name: "Jabra Speak".to_owned(),
            occurrence: 0,
            delay: Ok(305.0),
            passes: Vec::new(),
        },
    ];

    let theme = Theme::builtin();
    let mut list = DrawList::new();
    screen.draw(
        &mut list,
        Rect::new(0.0, 0.0, 1600.0, 1000.0),
        &theme.resolve_default(),
    );
    let text: String = list
        .commands()
        .iter()
        .filter_map(|command| match command {
            rungstar_ui::draw::Command::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        !text.contains("One value covers every microphone"),
        "the screen still promises one shared value: {text}"
    );
    assert!(
        text.contains("its own"),
        "the screen did not say each microphone keeps its own delay: {text}"
    );
    // And the shared figure is still named, because it is what anything unmeasured uses.
    assert!(text.contains("141"), "the fallback was not named: {text}");
}
