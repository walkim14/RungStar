//! Themes, and the property that makes them safe to hand to a player: no combination of skin
//! and accent can produce something unreadable.

use rungstar_ui::color::Color;
use rungstar_ui::theme::Theme;

#[test]
fn the_builtin_theme_parses_and_validates() {
    // `Theme::builtin` unwraps, so this test is what stops a bad edit reaching a launch.
    let theme = Theme::builtin();
    theme.validate().expect("built-in theme is valid");
    assert_eq!(theme.meta.name, "Rung");
    assert!(theme.skins.contains_key(&theme.meta.default_skin));
    assert!(theme.accents.contains_key(&theme.meta.default_accent));
}

#[test]
fn colours_parse_in_every_length_and_round_trip() {
    assert_eq!(Color::parse("#fff").unwrap(), Color::rgb(255, 255, 255));
    assert_eq!(Color::parse("#f00").unwrap(), Color::rgb(255, 0, 0));
    assert_eq!(
        Color::parse("#ff000080").unwrap(),
        Color::rgba(255, 0, 0, 128)
    );
    assert_eq!(Color::parse("#1a2b3c").unwrap(), Color::rgb(26, 43, 60));

    // Short form expands so that `#abc` and `#aabbcc` are the same colour, which is what
    // every other tool means by it.
    assert_eq!(
        Color::parse("#abc").unwrap(),
        Color::parse("#aabbcc").unwrap()
    );

    for text in ["#123456", "#12345678"] {
        let parsed = Color::parse(text).unwrap();
        assert_eq!(parsed.to_string(), text);
    }

    assert!(Color::parse("123456").is_err(), "must require a hash");
    assert!(
        Color::parse("#12345").is_err(),
        "odd length is not a colour"
    );
    assert!(Color::parse("#gggggg").is_err(), "g is not a hex digit");
}

#[test]
fn text_on_the_accent_is_always_readable() {
    // Any accent a theme offers, on any skin, must produce a label that can be read. This is
    // the reason `on_accent` is derived rather than authored: a theme author picking white
    // for every accent breaks the yellow one, and nothing would catch it.
    let theme = Theme::builtin();
    for skin in theme.skin_names() {
        for accent in theme.accent_names() {
            let style = theme.resolve(skin, accent);
            let difference = (style.accent.luminance() - style.on_accent.luminance()).abs();
            assert!(
                difference > 0.4,
                "{skin}/{accent}: accent {} and text {} are too close ({difference:.2})",
                style.accent,
                style.on_accent
            );
        }
    }
}

#[test]
fn surfaces_separate_in_both_directions() {
    // On a dark skin "raised" is lighter than the surface; on a light skin it is lighter
    // still but sunken must go the other way. A theme that got this backwards would have an
    // invisible list cursor, which is exactly the bug deriving them prevents.
    let theme = Theme::builtin();
    for skin in theme.skin_names() {
        let style = theme.resolve(skin, "");
        assert_ne!(
            style.surface_raised, style.surface,
            "{skin}: raised is flat"
        );
        assert_ne!(
            style.surface_sunken, style.surface,
            "{skin}: sunken is flat"
        );
        assert!(
            style.surface_raised.luminance() > style.surface_sunken.luminance(),
            "{skin}: raised is not brighter than sunken"
        );
        // Body text has to stand off its own surface.
        let contrast = (style.text.luminance() - style.surface.luminance()).abs();
        assert!(contrast > 0.4, "{skin}: text on surface is too close");
    }
}

#[test]
fn an_unknown_skin_or_accent_falls_back_instead_of_failing() {
    // These names come out of a config file the player can edit and out of themes that get
    // replaced. Refusing to draw because a colour was renamed is the wrong trade.
    let theme = Theme::builtin();
    let fallback = theme.resolve("no-such-skin", "no-such-accent");
    let default = theme.resolve_default();
    assert_eq!(fallback.background, default.background);
    assert_eq!(fallback.accent, default.accent);
}

#[test]
fn a_theme_always_has_six_player_colours_and_they_are_distinct() {
    let theme = Theme::builtin();
    let style = theme.resolve_default();
    assert!(style.players.len() >= 6);
    for i in 0..6 {
        for j in (i + 1)..6 {
            assert_ne!(
                style.player(i),
                style.player(j),
                "players {i} and {j} share a colour"
            );
        }
    }
    // Asking beyond the list wraps rather than panicking.
    assert_eq!(style.player(0), style.player(style.players.len()));
}

#[test]
fn a_theme_missing_its_default_skin_is_rejected() {
    let text = r##"
        [meta]
        name = "Broken"
        default_skin = "midnight"

        [skins.dark]
        background = "#000000"
        surface = "#111111"
        text = "#ffffff"
        muted = "#888888"
        accent = "#ff0000"
    "##;
    let theme = Theme::parse(text).expect("parses");
    assert!(
        theme.validate().is_err(),
        "a missing default skin is an error"
    );
    // It still resolves, because a broken theme must not stop the game starting.
    let style = theme.resolve("midnight", "");
    assert_eq!(style.background, rungstar_ui::Color::rgb(0, 0, 0));
}

#[test]
fn text_scale_multiplies_every_size() {
    let mut theme = Theme::builtin();
    theme.metrics.text_scale = 1.5;
    let style = theme.resolve_default();
    assert_eq!(style.text_size(), theme.metrics.text_size * 1.5);
    assert_eq!(style.scaled_text(2.0), theme.metrics.text_size * 3.0);
}
