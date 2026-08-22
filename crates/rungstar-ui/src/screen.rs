//! The screen stack, and the widgets every screen shares.
//!
//! A screen is state plus two methods: take an action, produce a display list. It never
//! touches a window, a canvas or a font, so a test drives a screen by feeding it actions and
//! reading the commands back — which is how the song browser and the options pages are
//! checked without a display.
//!
//! Screens are a stack rather than a graph. Opening the options from the main menu pushes;
//! Back pops and the menu is exactly as it was, with the cursor where it was left. UltraStar
//! Deluxe keeps a global "current screen" and each screen hard-codes which one Back goes to,
//! which is why some of its screens return you to the wrong place.

use crate::color::Color;
use crate::draw::{Align, DrawList, Font, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Rect};
use crate::theme::Style;

/// What a screen wants the application to do after handling an action.
#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    /// Stay here.
    None,
    /// Close this screen and go back to the one underneath.
    Pop,
    /// Leave the game.
    Quit,
    /// Start singing the song at this library index.
    Sing(i64),
    /// Open another screen, named so the application decides what that means.
    Push(Route),
}

/// Screens the application knows how to open.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    Main,
    SongSelect,
    Options,
    OptionsPage(usize),
    Search,
    /// Who is singing tonight.
    Players,
    /// Teams, jokers and brackets.
    Party,
    /// Songs played back to back with nobody scoring.
    Jukebox,
    /// The USDB catalog, browsed and downloaded from inside the game.
    Usdb,
    /// The four statistics views.
    Stats,
    About,
}

/// How a selectable control relates to the input focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    /// Not selected.
    Idle,
    /// Chosen or toggled, but not receiving input.
    Chosen,
    /// Selected and receiving input.
    Active,
    /// The selected parent of the control receiving input.
    Context,
}

/// Text colours resolved alongside a selectable control's surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPalette {
    pub text: Color,
    pub muted: Color,
}

/// Draws the pieces every screen is made of, so they look the same on all of them.
///
/// UltraStar has each screen draw its own button, which is why its screens disagree about what
/// a selected item looks like.
pub struct Widgets<'a> {
    pub style: &'a Style,
}

impl<'a> Widgets<'a> {
    pub fn new(style: &'a Style) -> Self {
        Self { style }
    }

    /// Body text style.
    pub fn text(&self) -> TextStyle {
        TextStyle::new(self.style.text_size(), self.style.text)
    }

    /// Secondary text: captions, hints, values that are not the point of the row.
    pub fn muted(&self) -> TextStyle {
        TextStyle::new(self.style.text_size(), self.style.muted)
    }

    /// A heading, sized relative to body text so the theme's text scale reaches it.
    pub fn heading(&self, factor: f32) -> TextStyle {
        TextStyle::new(self.style.scaled_text(factor), self.style.text).font(Font::Bold)
    }

    /// A selectable surface and the text colours that remain readable on it.
    pub fn selectable(
        &self,
        list: &mut DrawList,
        rect: Rect,
        state: ControlState,
    ) -> ControlPalette {
        let (fill, palette) = match state {
            ControlState::Idle => (
                self.style.surface,
                ControlPalette {
                    text: self.style.text,
                    muted: self.style.muted,
                },
            ),
            ControlState::Chosen => (
                self.style.surface_raised,
                ControlPalette {
                    text: self.style.text,
                    muted: self.style.muted,
                },
            ),
            ControlState::Active => (
                self.style.accent,
                ControlPalette {
                    text: self.style.on_accent,
                    muted: self.style.on_accent.alpha(0.8),
                },
            ),
            ControlState::Context => (
                self.style.surface_raised,
                ControlPalette {
                    text: self.style.text,
                    muted: self.style.muted,
                },
            ),
        };
        let radius = self.style.metrics.radius;
        list.panel(rect, fill, radius);
        match state {
            ControlState::Active => {
                // A single translucent hairline gives the flat accent surface a crisp top
                // edge. It is cheaper than a shadow or gradient and only the active control
                // pays for it.
                let line_h = (self.style.metrics.outline * 0.5).max(1.0);
                let line = rect.inset_each(radius, line_h, radius, rect.h - line_h * 2.0);
                if line.w > 0.0 && line.h > 0.0 {
                    list.fill(line, self.style.on_accent.alpha(0.2));
                }
            }
            ControlState::Context => {
                list.outline(
                    rect,
                    self.style.accent_soft,
                    self.style.metrics.outline,
                    radius,
                );
            }
            ControlState::Idle | ControlState::Chosen => {}
        }
        palette
    }

    /// The bar across the top of a screen: title on the left, status on the right.
    pub fn header(&self, list: &mut DrawList, area: Rect, title: &str, status: &str) -> Rect {
        let (bar, rest) = area.cut_top(self.style.gap(5.0));
        let inner = bar.inset_xy(self.style.gap(2.0), 0.0);
        // The title takes the room it needs and the status is cut off, not the other way
        // round: which screen this is matters more than how many songs are on it. Sharing one
        // box would let a long status run back under the title.
        let (title_box, status_box) = if status.is_empty() {
            (inner, inner)
        } else {
            let (left, right) = inner.cut_left(inner.w * 0.62);
            (
                Rect::new(
                    left.x,
                    left.y,
                    (left.w - self.style.gap(1.0)).max(0.0),
                    left.h,
                ),
                right,
            )
        };
        list.text(
            title_box,
            title,
            self.heading(1.5).overflow(Overflow::Ellipsis),
        );
        if !status.is_empty() {
            list.text(
                status_box,
                status,
                self.muted().align(Align::End).overflow(Overflow::Ellipsis),
            );
        }
        // A hairline rather than a filled bar: the header should separate, not compete.
        let line = Rect::new(inner.x, bar.bottom() - 1.5, inner.w, 1.5);
        list.fill(line, self.style.muted.alpha(0.25));
        rest
    }

    /// The strip along the bottom listing what the buttons do right now.
    ///
    /// Always present, because on a controller there is nowhere else to discover that West
    /// opens the search.
    pub fn footer(&self, list: &mut DrawList, area: Rect, hints: &[(&str, &str)]) -> Rect {
        let (bar, rest) = area.cut_bottom(self.style.gap(3.5));
        let mut x = bar.x + self.style.gap(2.0);
        let size = self.style.scaled_text(0.8);
        let edge = bar.right() - self.style.gap(1.0);
        for (button, action) in hints {
            let chip_w = crate::draw::approx_text_width(button, size) + self.style.gap(1.2);
            // Stop rather than run off the end. The row has never had a bound and got away
            // with it while the width estimate was too small; a hint drawn past the edge of
            // the window is not a hint, and on a Deck there is no way to scroll to it. They
            // are written most useful first, so what is dropped is what is least missed.
            let needed = chip_w
                + self.style.gap(0.5)
                + crate::draw::approx_text_width(action, size)
                + self.style.gap(0.5);
            if x + needed > edge {
                break;
            }
            let chip = Rect::new(x, bar.center().y - size * 0.9, chip_w, size * 1.8);
            list.panel(
                chip,
                self.style.surface_raised,
                self.style.metrics.radius * 0.6,
            );
            list.text(
                chip,
                *button,
                TextStyle::new(size, self.style.text).centered().bold(),
            );
            x += chip_w + self.style.gap(0.5);

            let label_w = crate::draw::approx_text_width(action, size) + self.style.gap(0.5);
            list.text(
                Rect::new(x, chip.y, label_w, chip.h),
                *action,
                TextStyle::new(size, self.style.muted),
            );
            x += label_w + self.style.gap(1.2);
        }
        rest
    }

    /// A list row: background, label, and an optional value on the right.
    pub fn row(&self, list: &mut DrawList, rect: Rect, label: &str, value: &str, selected: bool) {
        self.row_state(
            list,
            rect,
            label,
            value,
            if selected {
                ControlState::Active
            } else {
                ControlState::Idle
            },
        );
    }

    /// A list row with an explicit focus/selection relationship.
    pub fn row_state(
        &self,
        list: &mut DrawList,
        rect: Rect,
        label: &str,
        value: &str,
        state: ControlState,
    ) {
        let palette = self.selectable(list, rect, state);
        let inner = rect.inset_xy(self.style.gap(1.2), 0.0);
        // Two columns rather than two strings in one box. An ellipsis is applied at the edge
        // of the rectangle it is given, so a label and a value sharing one rectangle overlap
        // in the middle instead of either of them being cut off.
        let (label_box, value_box) = if value.is_empty() {
            (inner, inner)
        } else {
            let (left, right) = inner.cut_left(inner.w * 0.6);
            (
                Rect::new(
                    left.x,
                    left.y,
                    (left.w - self.style.gap(1.0)).max(0.0),
                    left.h,
                ),
                right,
            )
        };
        list.text(
            label_box,
            label,
            TextStyle::new(self.style.text_size(), palette.text).overflow(Overflow::Ellipsis),
        );
        if !value.is_empty() {
            list.text(
                value_box,
                value,
                TextStyle::new(self.style.text_size(), palette.muted)
                    .align(Align::End)
                    .overflow(Overflow::Ellipsis),
            );
        }
    }

    /// A horizontal bar filled to `fraction`, for volumes and delays.
    pub fn slider(&self, list: &mut DrawList, rect: Rect, fraction: f32, on_accent: bool) {
        let height = self.style.gap(0.5);
        let thumb = height * 1.75;
        // Leave half a thumb at either end, so 0% and 100% remain inside the control rather
        // than being clipped by the row or the edge of the screen.
        let track_box = rect.inset_xy((thumb / 2.0).min(rect.w / 2.0), 0.0);
        let track = track_box.anchored(Anchor::Center, track_box.w, height, 0.0);
        let (track_color, fill_color) = if on_accent {
            (self.style.on_accent.alpha(0.3), self.style.on_accent)
        } else {
            (self.style.surface_sunken, self.style.accent)
        };
        list.panel(track, track_color, height / 2.0);
        let fraction = fraction.clamp(0.0, 1.0);
        let filled = Rect::new(track.x, track.y, track.w * fraction, track.h);
        if filled.w > 0.0 {
            list.panel(filled, fill_color, height / 2.0);
        }
        let centre_x = track.x + track.w * fraction;
        list.panel(
            Rect::new(
                centre_x - thumb / 2.0,
                rect.center().y - thumb / 2.0,
                thumb,
                thumb,
            ),
            fill_color,
            thumb / 2.0,
        );
    }

    /// A ring around the focused element.
    pub fn focus_ring(&self, list: &mut DrawList, rect: Rect) {
        list.outline(
            rect.inset(-self.style.metrics.outline),
            self.style.accent,
            self.style.metrics.outline,
            self.style.metrics.radius + self.style.metrics.outline,
        );
    }

    /// A centred message for an empty list or a failed search.
    ///
    /// Empty states get a reason and a way forward, because "no songs" on a first run with no
    /// explanation is where a player gives up.
    pub fn empty_state(&self, list: &mut DrawList, area: Rect, title: &str, detail: &str) {
        let box_h = self.style.gap(8.0);
        let centred = area.anchored(Anchor::Center, area.w.min(900.0), box_h, 0.0);
        let (top, bottom) = centred.cut_top(box_h / 2.0);
        list.text(
            top,
            title,
            TextStyle::new(self.style.scaled_text(1.3), self.style.text)
                .centered()
                .valign(VAlign::Bottom)
                .bold(),
        );
        list.text(
            bottom.inset_xy(self.style.gap(2.0), 0.0),
            detail,
            TextStyle::new(self.style.text_size(), self.style.muted)
                .centered()
                .valign(VAlign::Top),
        );
    }

    /// A dimming layer over whatever is behind, for a modal.
    pub fn scrim(&self, list: &mut DrawList, area: Rect) {
        list.fill(area, self.style.scrim);
    }

    /// A card: a raised panel to put a modal's contents in.
    pub fn card(&self, list: &mut DrawList, rect: Rect) {
        list.panel(rect, self.style.surface, self.style.metrics.radius * 1.5);
        list.outline(
            rect,
            self.style.muted.alpha(0.2),
            1.5,
            self.style.metrics.radius * 1.5,
        );
    }

    /// A coloured pill, for a genre, a language, or a player's name.
    pub fn chip(&self, list: &mut DrawList, rect: Rect, text: &str, color: Color) {
        list.panel(rect, color.alpha(0.22), rect.h / 2.0);
        list.text(
            rect.inset_xy(rect.h * 0.35, 0.0),
            text,
            TextStyle::new(rect.h * 0.55, color)
                .centered()
                .overflow(Overflow::Ellipsis),
        );
    }
}
