//! No line of text sits on top of another one.
//!
//! Rows in this interface are a label with a caption under it, and both were sized from the
//! spacing constant while the text itself scales with the **Text size** setting. At 1.0 they
//! clear each other by a unit or two; anywhere above it the caption climbs into the label, and
//! at 1.6 — which is what somebody playing on a television across a room actually sets — the
//! two are drawn through each other.
//!
//! The check has to be on the *ink*, not on the boxes. Text is placed by its baseline, so the
//! glyphs reach above and below the rectangle they were given, and two boxes that merely touch
//! can still collide. [`TextStyle::ink`] is the same arithmetic `rungstar-platform` uses to put
//! the baseline down, which is what makes this assertable with no window open.

mod common;

use rungstar_ui::draw::{Command, TextStyle};
use rungstar_ui::geom::Rect;
use rungstar_ui::theme::Theme;

/// A Steam Deck, which is the tightest shape the game supports.
const DECK: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1600.0,
    h: 1000.0,
};

/// A wide desktop, where rows are shorter relative to the text.
const WIDE: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 2400.0,
    h: 1000.0,
};

/// One drawn string, reduced to where its glyphs actually are.
struct Ink {
    text: String,
    /// Where in the draw order it was, so what is painted over it can be found.
    at: usize,
    /// How many modal scrims were drawn before it.
    layer: usize,
    box_rect: Rect,
    top: f32,
    bottom: f32,
    align: rungstar_ui::Align,
}

/// How much two spans share, negative when they are apart.
fn overlap(a: (f32, f32), b: (f32, f32)) -> f32 {
    a.1.min(b.1) - a.0.max(b.0)
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x <= inner.x + 0.5
        && outer.y <= inner.y + 0.5
        && outer.right() >= inner.right() - 0.5
        && outer.bottom() >= inner.bottom() - 0.5
}

/// Whether this rectangle is a modal scrim: something covering the whole screen.
///
/// A dialog dims what is behind it and draws on top, and the dimmed browser stays visible
/// around the panel *on purpose*. So a scrim separates layers: text before it and text after
/// it are not on the same picture and cannot be said to collide, however their boxes overlap.
fn is_scrim(command: &Command, area: Rect) -> bool {
    matches!(command, Command::Rect { rect, .. }
        if rect.w >= area.w * 0.95 && rect.h >= area.h * 0.95)
}

/// Whether something opaque was painted over `earlier` before `later` was drawn.
///
/// The backend is a painter's algorithm, so a panel drawn on top of an earlier one hides it —
/// which is exactly what happens when the search keyboard opens over the detail panel. Without
/// this the test reports the cover-art initials colliding with a key on a keyboard that is
/// drawn on top of them, which is not a collision, it is a lid.
fn covered(commands: &[Command], earlier: &Ink, later_at: usize) -> bool {
    commands[earlier.at + 1..=later_at].iter().any(|command| {
        matches!(command, Command::Rect { rect, color, .. }
            if color.a == 255 && contains(*rect, earlier.box_rect))
    })
}

#[test]
fn no_two_lines_of_text_are_drawn_through_each_other() {
    let mut found: Vec<String> = Vec::new();
    for scale in [0.7, 1.0, 1.3, 1.6] {
        for area in [DECK, WIDE] {
            let mut theme = Theme::builtin();
            theme.metrics.text_scale = scale;
            let style = theme.resolve_default();

            for (name, list) in common::every_screen(area, &style) {
                let commands = list.commands();
                let mut layer = 0;
                let mut clips: Vec<Rect> = Vec::new();
                let mut inks: Vec<Ink> = Vec::new();
                for (at, command) in commands.iter().enumerate() {
                    match command {
                        // A clipped list is cut at its edge, so the row half off the top of a
                        // scrolling table is not drawn over the heading above it — it is not
                        // drawn at all past the boundary.
                        Command::PushClip(rect) => clips.push(*rect),
                        Command::PopClip => {
                            clips.pop();
                        }
                        _ if is_scrim(command, area) => layer += 1,
                        Command::Text { rect, text, style } if !text.trim().is_empty() => {
                            let (mut top, mut bottom) = style.ink(*rect);
                            let mut box_rect = *rect;
                            for clip in &clips {
                                top = top.max(clip.y);
                                bottom = bottom.min(clip.bottom());
                                let left = box_rect.x.max(clip.x);
                                let right = box_rect.right().min(clip.right());
                                box_rect = Rect::new(
                                    left,
                                    box_rect.y,
                                    (right - left).max(0.0),
                                    box_rect.h,
                                );
                            }
                            // Entirely outside its clip: nothing is drawn.
                            if bottom <= top || box_rect.w <= 0.0 {
                                continue;
                            }
                            inks.push(Ink {
                                text: text.clone(),
                                at,
                                layer,
                                box_rect,
                                top,
                                bottom,
                                align: style.align,
                            });
                        }
                        _ => {}
                    }
                }

                for (i, a) in inks.iter().enumerate() {
                    for b in &inks[i + 1..] {
                        if a.layer != b.layer {
                            continue;
                        }
                        // Only strings that share horizontal space can collide. Two columns of
                        // a row sit at the same height on purpose.
                        let across = overlap(
                            (a.box_rect.x, a.box_rect.right()),
                            (b.box_rect.x, b.box_rect.right()),
                        );
                        if across <= 1.0 {
                            continue;
                        }
                        // A label and its value share one rectangle and are pushed to opposite
                        // ends of it, so they are at the same height on purpose. Whether they
                        // meet in the middle is a question about string widths, which needs a
                        // font — `Overflow::Ellipsis` answers that, and `deck.rs` checks the
                        // boxes. This test is about the vertical axis.
                        if a.align != b.align {
                            continue;
                        }
                        let down = overlap((a.top, a.bottom), (b.top, b.bottom));
                        // A unit of slack: a descender grazing a cap height is not a collision,
                        // and demanding a hairline gap would fail on rounding.
                        if down <= 1.0 || covered(commands, a, b.at) {
                            continue;
                        }
                        found.push(format!(
                            "scale {scale} {}x{} {name}: {:?} through {:?} ({down:.1})",
                            area.w, area.h, a.text, b.text,
                        ));
                    }
                }
            }
        }
    }
    found.dedup();
    // Reported all together rather than one at a time: they come in families — one layout
    // mistake is every row of a list — and fixing them one failure per run is a long afternoon.
    assert!(
        found.is_empty(),
        "{} collisions:
{}",
        found.len(),
        found.join(
            "
"
        )
    );
}

#[test]
fn a_row_knows_how_tall_its_text_needs_it_to_be() {
    // The arithmetic the fix depends on. A box exactly `height()` tall holds the line with
    // nothing to spare, and one unit shorter does not.
    let needed = TextStyle::height(40.0);
    let exact = Rect::new(0.0, 100.0, 200.0, needed);
    for valign in [
        rungstar_ui::VAlign::Top,
        rungstar_ui::VAlign::Middle,
        rungstar_ui::VAlign::Bottom,
    ] {
        let style = TextStyle::new(40.0, rungstar_ui::Color::WHITE).valign(valign);
        let (top, bottom) = style.ink(exact);
        assert!(
            top >= exact.y - 0.01,
            "{valign:?}: {top} is above {}",
            exact.y
        );
        assert!(
            bottom <= exact.bottom() + 0.01,
            "{valign:?}: {bottom} is below {}",
            exact.bottom()
        );
    }
}

#[test]
fn text_larger_than_its_box_is_what_used_to_go_wrong() {
    // Bottom-aligned text in a box too short for it grows *upwards*, out of the box and into
    // whatever is above. That is the shape of the bug: the row was sized in spacing units and
    // the text in text units, and only one of the two followed the Text size setting.
    let style = TextStyle::new(60.0, rungstar_ui::Color::WHITE).valign(rungstar_ui::VAlign::Bottom);
    let cramped = Rect::new(0.0, 100.0, 200.0, 20.0);
    let (top, _) = style.ink(cramped);
    assert!(
        top < cramped.y,
        "expected the glyphs to escape upwards, got {top}"
    );
}
