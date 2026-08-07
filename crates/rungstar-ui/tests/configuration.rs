//! Settings and the options pages built from them.

use rungstar_ui::options::{Action, Control, Page};
use rungstar_ui::settings::{
    Choice, Difficulty, FrameLimit, MicBoost, Settings, MAX_PLAYERS, THRESHOLDS,
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
            if item.is_button() || matches!(item.control, Control::Text { .. }) {
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
        Action::ManageMicrophones,
        Action::RebindControls,
        Action::ResetToDefaults,
    ] {
        assert!(buttons.contains(&action), "{action:?} is on no page");
    }
}
