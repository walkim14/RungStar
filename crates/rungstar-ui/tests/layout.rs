//! Geometry, and the claim the whole design rests on: the same layout code is correct on
//! every display it will actually run on.

use rungstar_ui::draw::{
    approx_text_width, Align, Command, DrawList, Font, ImageId, Overflow, TextStyle, VAlign,
    WHOLE_IMAGE,
};
use rungstar_ui::geom::{Anchor, Point, Projection, Rect, DESIGN_HEIGHT};
use rungstar_ui::screen::{ControlState, Widgets};
use rungstar_ui::theme::Theme;
use rungstar_ui::Color;

/// The displays this has to be right on: a Steam Deck, a 1080p and a 1440p monitor, a 4K
/// television, an ultrawide, and the 4:3 a projector still hands you at a party.
const DISPLAYS: [(u32, u32); 6] = [
    (1280, 800),
    (1920, 1080),
    (2560, 1440),
    (3840, 2160),
    (3440, 1440),
    (1024, 768),
];

#[test]
fn every_display_gets_the_same_height_in_design_units() {
    for (w, h) in DISPLAYS {
        let screen = Projection::new(w, h).screen();
        assert_eq!(screen.h, DESIGN_HEIGHT, "height differs at {w}x{h}");
        assert_eq!(screen.y, 0.0);
        // Width follows the aspect ratio, so a wider display has more room rather than a
        // stretched copy of the same picture.
        let expected = DESIGN_HEIGHT * w as f32 / h as f32;
        assert!(
            (screen.w - expected).abs() < 0.01,
            "width {} != {expected} at {w}x{h}",
            screen.w
        );
    }
}

#[test]
fn a_row_is_the_same_physical_fraction_of_every_screen() {
    // The point of the design space: a 64-unit row is 6.4% of the display's height whatever
    // the display is. UltraStar's 800x600 coordinates cannot say that.
    for (w, h) in DISPLAYS {
        let projection = Projection::new(w, h);
        let pixels = projection.px(64.0);
        let fraction = pixels / h as f32;
        assert!(
            (fraction - 0.064).abs() < 0.0001,
            "row is {fraction} of the height at {w}x{h}"
        );
    }
}

#[test]
fn unprojecting_a_pixel_returns_the_design_point_that_produced_it() {
    for (w, h) in DISPLAYS {
        let projection = Projection::new(w, h);
        let point = Point::new(321.0, 654.0);
        let there_and_back =
            projection.unproject(projection.point(point).x, projection.point(point).y);
        assert!((there_and_back.x - point.x).abs() < 0.01);
        assert!((there_and_back.y - point.y).abs() < 0.01);
    }
}

#[test]
fn content_is_capped_on_a_wide_display_and_full_width_on_a_narrow_one() {
    let narrow = Projection::new(1024, 768).content(1400.0);
    assert_eq!(narrow.w, Projection::new(1024, 768).screen().w);

    let ultrawide = Projection::new(3440, 1440);
    let content = ultrawide.content(1400.0);
    assert_eq!(content.w, 1400.0);
    // Capped, and centred rather than left-aligned: the margin is shared.
    let screen = ultrawide.screen();
    assert!((content.x - (screen.w - content.w) / 2.0).abs() < 0.01);
}

#[test]
fn anchors_place_a_box_in_each_corner_and_edge() {
    let area = Rect::new(0.0, 0.0, 1000.0, 500.0);
    let corner = area.anchored(Anchor::TopLeft, 100.0, 50.0, 20.0);
    assert_eq!((corner.x, corner.y), (20.0, 20.0));

    let corner = area.anchored(Anchor::BottomRight, 100.0, 50.0, 20.0);
    assert_eq!((corner.right(), corner.bottom()), (980.0, 480.0));

    let middle = area.anchored(Anchor::Center, 100.0, 50.0, 20.0);
    assert_eq!(middle.center(), area.center());
}

#[test]
fn splitting_preserves_the_area_it_was_given() {
    let area = Rect::new(10.0, 20.0, 900.0, 400.0);
    for n in 1..8 {
        let gap = 12.0;
        let columns = area.columns(n, gap);
        assert_eq!(columns.len(), n);
        assert_eq!(columns[0].x, area.x);
        assert!((columns[n - 1].right() - area.right()).abs() < 0.01);
        // Columns never overlap, whatever the count.
        for pair in columns.windows(2) {
            assert!(pair[1].x >= pair[0].right() - 0.001);
        }
    }
}

#[test]
fn cutting_a_strip_leaves_exactly_the_rest() {
    let area = Rect::new(0.0, 0.0, 800.0, 600.0);
    let (top, rest) = area.cut_top(100.0);
    assert_eq!(top.h, 100.0);
    assert_eq!(rest.y, 100.0);
    assert_eq!(rest.h, 500.0);

    // Cutting more than there is takes everything rather than producing a negative height.
    let (all, nothing) = area.cut_top(9999.0);
    assert_eq!(all.h, 600.0);
    assert_eq!(nothing.h, 0.0);
}

#[test]
fn fitting_letterboxes_and_covering_crops() {
    let wide = Rect::new(0.0, 0.0, 1600.0, 900.0);
    // A square cover in a wide box: fit gives a square, no wider than the box is tall.
    let fitted = wide.fit_aspect(1.0);
    assert_eq!(fitted.w, 900.0);
    assert_eq!(fitted.h, 900.0);
    assert_eq!(fitted.center(), wide.center());

    // Covering the same box with a square means overflowing the sides.
    let covered = wide.cover_aspect(1.0);
    assert_eq!(covered.w, 1600.0);
    assert_eq!(covered.h, 1600.0);
    assert_eq!(covered.center(), wide.center());
}

#[test]
fn a_draw_list_records_what_it_was_told_and_balances_its_clips() {
    let mut list = DrawList::new();
    let area = Rect::new(0.0, 0.0, 100.0, 100.0);
    list.fill(area, Color::WHITE);
    list.clipped(area, |inner| {
        inner.text(area, "hello", TextStyle::new(30.0, Color::BLACK));
    });

    assert!(list.is_balanced());
    assert_eq!(list.len(), 4);
    assert!(matches!(list.commands()[0], Command::Rect { .. }));
    assert!(matches!(list.commands()[1], Command::PushClip(_)));
    assert!(matches!(list.commands()[2], Command::Text { .. }));
    assert!(matches!(list.commands()[3], Command::PopClip));
}

#[test]
fn an_unbalanced_clip_is_detectable() {
    let mut list = DrawList::new();
    list.push(Command::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)));
    assert!(!list.is_balanced());
    list.push(Command::PopClip);
    assert!(list.is_balanced());

    list.push(Command::PopClip);
    assert!(
        !list.is_balanced(),
        "an unmatched pop must invalidate the frame"
    );

    list.clear();
    assert!(list.is_balanced(), "a cleared list starts a fresh frame");
}

#[test]
fn estimated_text_width_errs_wide() {
    // A box a little too wide looks fine; one a little too narrow clips the title. The
    // estimate is only used where a size is needed before the glyphs exist.
    let size = 30.0;
    assert!(approx_text_width("", size) == 0.0);
    assert!(approx_text_width("iiii", size) > 0.0);
    assert!(approx_text_width("WWWW", size) < 4.0 * size * 1.1);
    assert!(approx_text_width("ab", size) < approx_text_width("abc", size));
}

#[test]
fn a_slider_keeps_its_thumb_inside_the_control_at_every_value() {
    let style = Theme::builtin().resolve_default();
    let widgets = Widgets::new(&style);
    let control = Rect::new(100.0, 200.0, 320.0, 60.0);

    for fraction in [-1.0, 0.0, 0.5, 1.0, 2.0] {
        let mut list = DrawList::new();
        widgets.slider(&mut list, control, fraction, false);
        let panels: Vec<Rect> = list
            .commands()
            .iter()
            .filter_map(|command| match command {
                Command::Rect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();

        assert!(panels.len() <= 3, "slider emitted too many draw commands");
        let thumb = panels.last().expect("slider has a thumb");
        assert!((thumb.w - thumb.h).abs() < 0.01, "thumb is not round");
        assert!(
            thumb.x >= control.x - 0.01
                && thumb.right() <= control.right() + 0.01
                && thumb.y >= control.y - 0.01
                && thumb.bottom() <= control.bottom() + 0.01,
            "{fraction}: thumb {thumb:?} escaped {control:?}"
        );
    }
}

#[test]
fn selectable_controls_distinguish_active_focus_from_parent_context() {
    let style = Theme::builtin().resolve_default();
    let widgets = Widgets::new(&style);
    let control = Rect::new(100.0, 200.0, 320.0, 60.0);

    for (state, expected_fill, expected_commands) in [
        (ControlState::Idle, style.surface, 1),
        (ControlState::Chosen, style.surface_raised, 1),
        (ControlState::Active, style.accent, 2),
        (ControlState::Context, style.surface_raised, 2),
    ] {
        let mut list = DrawList::new();
        let palette = widgets.selectable(&mut list, control, state);
        assert_eq!(list.len(), expected_commands, "{state:?} command budget");
        assert!(matches!(
            list.commands().first(),
            Some(Command::Rect { rect, color, .. }) if *rect == control && *color == expected_fill
        ));

        match state {
            ControlState::Idle | ControlState::Chosen => assert_eq!(palette.text, style.text),
            ControlState::Active => assert_eq!(palette.text, style.on_accent),
            ControlState::Context => {
                assert_eq!(palette.text, style.text);
                assert!(matches!(
                    list.commands().last(),
                    Some(Command::Outline { color, .. }) if *color == style.accent_soft
                ));
            }
        }
    }
}

#[test]
fn draw_commands_preserve_the_geometry_and_style_the_backend_needs() {
    let area = Rect::new(10.0, 20.0, 200.0, 80.0);
    let mut list = DrawList::new();
    assert!(list.is_empty());

    let style = TextStyle::new(32.0, Color::WHITE)
        .font(Font::Lyrics)
        .align(Align::End)
        .valign(VAlign::Top)
        .ellipsis()
        .outlined(Color::BLACK, 2.0)
        .color(Color::rgb(10, 20, 30));
    list.outline(area, Color::WHITE, 2.0, 8.0)
        .bubble(area, Color::rgb(10, 20, 30), Color::WHITE)
        .glow(area, Color::rgba(20, 30, 40, 128))
        .text(area, "hello", style.clone())
        .image(area, ImageId(1))
        .image_tinted(area, ImageId(2), Color::rgba(1, 2, 3, 4), 7.0)
        .line(
            Point::new(1.0, 2.0),
            Point::new(3.0, 4.0),
            Color::BLACK,
            5.0,
        )
        .stage_pulse(2.0);

    assert_eq!(style.font, Font::Lyrics);
    assert_eq!(style.align, Align::End);
    assert_eq!(style.valign, VAlign::Top);
    assert_eq!(style.overflow, Overflow::Ellipsis);
    assert_eq!(style.outline, Some((Color::BLACK, 2.0)));
    assert_eq!(style.color, Color::rgb(10, 20, 30));
    assert!(matches!(
        list.commands()[1],
        Command::Bubble { rect, fill: Color { r: 10, .. }, rim: Color::WHITE }
            if rect == area
    ));
    assert!(matches!(
        list.commands()[2],
        Command::Glow { rect, color: Color { a: 128, .. } } if rect == area
    ));
    assert!(matches!(
        &list.commands()[3],
        Command::Text { text, style: drawn, .. } if text == "hello" && *drawn == style
    ));
    assert!(matches!(
        list.commands()[4],
        Command::Image { image: ImageId(1), tint: Color::WHITE, radius: 0.0, source, .. }
            if source == WHOLE_IMAGE
    ));
    assert!(matches!(
        list.commands()[5],
        Command::Image {
            image: ImageId(2),
            tint: Color { a: 4, .. },
            radius: 7.0,
            ..
        }
    ));
    assert!(matches!(
        list.commands()[6],
        Command::Line { width: 5.0, .. }
    ));
    assert!(matches!(
        list.commands()[7],
        Command::StagePulse { strength: 1.0 }
    ));
}

#[test]
fn shared_presentational_widgets_keep_their_visual_roles_distinct() {
    let style = Theme::builtin().resolve_default();
    let widgets = Widgets::new(&style);
    let area = Rect::new(100.0, 100.0, 600.0, 300.0);
    let mut list = DrawList::new();

    widgets.focus_ring(&mut list, area);
    widgets.empty_state(&mut list, area, "Nothing here", "Try another filter");
    widgets.scrim(&mut list, area);
    widgets.card(&mut list, area.inset(20.0));
    widgets.chip(
        &mut list,
        Rect::new(120.0, 340.0, 180.0, 40.0),
        "Duet",
        style.success,
    );

    assert!(list.commands().iter().any(|command| matches!(
        command,
        Command::Outline { color, .. } if *color == style.accent
    )));
    assert!(list.commands().iter().any(|command| matches!(
        command,
        Command::Rect { color, .. } if *color == style.scrim
    )));
    let text: Vec<&str> = list
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains(&"Nothing here"));
    assert!(text.contains(&"Try another filter"));
    assert!(text.contains(&"Duet"));
}
