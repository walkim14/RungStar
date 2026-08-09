//! The statistics screen: four views over what everybody has sung.
//!
//! One screen with four tabs rather than four screens, because they are the same table with a
//! different question at the top — UltraStar has them as separate screens and they all look
//! slightly different as a result.
//!
//! The rows are handed in already sorted. Deciding what "best" means is the profile store's
//! job, and it is a question about SQL rather than about drawing.

use rungstar_profile::stats::{Order, View};

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::Rect;
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// One line of a table: what it is, and the number beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The thing being counted — a song, a singer, an artist.
    pub label: String,
    /// Who did it, when that is not the label itself.
    pub detail: String,
    /// The number, already formatted, because what it means changes per view.
    pub value: String,
}

/// The statistics screen.
pub struct StatsScreen {
    pub view: View,
    pub order: Order,
    /// Rows for the view being shown, sorted by whoever supplied them.
    pub rows: Vec<Row>,
    pub gamepad: bool,
    /// Set when the view or the order changed and the rows need fetching again.
    stale: bool,
    scroll: usize,
    regions: Vec<(Rect, usize)>,
}

impl Default for StatsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsScreen {
    pub fn new() -> Self {
        Self {
            view: View::default(),
            order: Order::default(),
            rows: Vec::new(),
            gamepad: false,
            stale: true,
            scroll: 0,
            regions: Vec::new(),
        }
    }

    /// Whether the application should fetch rows for the current view.
    pub fn needs_rows(&self) -> bool {
        self.stale
    }

    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.rows = rows;
        self.scroll = 0;
        self.stale = false;
    }

    pub fn handle(&mut self, input: Input) -> Transition {
        match input {
            Input::Left => {
                self.view = self.view.previous();
                self.stale = true;
            }
            Input::Right | Input::CycleFilter => {
                self.view = self.view.next();
                self.stale = true;
            }
            Input::Up => self.scroll = self.scroll.saturating_sub(1),
            Input::Down => {
                if self.scroll + 1 < self.rows.len() {
                    self.scroll += 1;
                }
            }
            Input::PageUp => self.scroll = self.scroll.saturating_sub(8),
            Input::PageDown => {
                self.scroll = (self.scroll + 8).min(self.rows.len().saturating_sub(1))
            }
            // Best first is what everybody wants; worst first is genuinely funny at a party.
            Input::Sort | Input::Confirm => {
                self.order = self.order.flip();
                self.stale = true;
            }
            Input::Back => return Transition::Pop,
            Input::Hover(point) | Input::Click(point) => {
                if matches!(input, Input::Click(_)) {
                    if let Some((_, tab)) = self.regions.iter().find(|(r, _)| r.contains(point)) {
                        self.view = View::ALL[*tab];
                        self.stale = true;
                    }
                }
            }
            _ => {}
        }
        Transition::None
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.regions.clear();
        let widgets = Widgets::new(style);
        let body = widgets.header(list, area, "Statistics", self.order.label());
        let hints: &[(&str, &str)] = if self.gamepad {
            &[("LS/RS", "View"), ("A", "Reverse"), ("B", "Back")]
        } else {
            &[
                ("\u{2190}\u{2192}", "View"),
                ("Enter", "Reverse"),
                ("Esc", "Back"),
            ]
        };
        let body = widgets.footer(list, body, hints);

        // Tabs, so which of the four is showing is visible rather than remembered.
        let (tabs, rest) = body.cut_top(style.gap(4.0));
        let width = (tabs.w / 4.0).min(260.0);
        for (index, view) in View::ALL.iter().enumerate() {
            let rect = Rect::new(
                tabs.x + style.gap(2.0) + width * index as f32,
                tabs.y + style.gap(0.4),
                width - style.gap(0.6),
                tabs.h - style.gap(1.2),
            );
            self.regions.push((rect, index));
            let selected = *view == self.view;
            list.panel(
                rect,
                if selected {
                    style.accent
                } else {
                    style.surface
                },
                style.metrics.radius,
            );
            list.text(
                rect,
                view.title(),
                TextStyle::new(
                    style.text_size(),
                    if selected {
                        style.on_accent
                    } else {
                        style.muted
                    },
                )
                .centered()
                .bold(),
            );
        }

        let inner = rest.inset(style.gap(2.0));
        if self.rows.is_empty() {
            widgets.empty_state(
                list,
                inner,
                "Nothing sung yet",
                "Scores appear here once somebody has finished a song. If you have an \
                 UltraStar database, Options can import its history.",
            );
            return;
        }

        // The column headings, because a table of numbers without them is a puzzle.
        let (heading, table) = inner.cut_top(style.row_height(&[style.scaled_text(0.75)]));
        let (left, right) = self.view.columns();
        let label_style = TextStyle::new(style.scaled_text(0.75), style.muted);
        // Split where the rows split, so each heading sits over its own column.
        let (value_heading, label_heading) = heading.cut_right(heading.w * 0.22);
        list.text(label_heading, left, label_style.clone());
        list.text(value_heading, right, label_style.align(Align::End));
        list.fill(
            Rect::new(heading.x, heading.bottom() - 1.0, heading.w, 1.0),
            style.muted.alpha(0.2),
        );

        // From the two lines a row holds rather than from a spacing constant: text scales
        // with the Text size setting and spacing does not, so a fixed row put the detail line
        // through the title above it at anything past 1.0.
        let title_size = style.text_size();
        let detail_size = style.scaled_text(0.78);
        let row_h = style.row_height(&[title_size, detail_size]) + style.gap(0.4);
        let visible = ((table.h / row_h).floor() as usize).max(1);
        let first = self
            .scroll
            .min(self.rows.len().saturating_sub(visible.min(self.rows.len())));

        list.clipped(table, |list| {
            for (offset, row) in self.rows.iter().skip(first).take(visible).enumerate() {
                let rect = Rect::new(table.x, table.y + row_h * offset as f32, table.w, row_h)
                    .inset_xy(0.0, style.gap(0.2));
                // The top three get the accent, because a leaderboard whose winner looks like
                // everybody else is not a leaderboard.
                let place = first + offset;
                let podium = place < 3 && self.order == Order::Best;
                list.panel(
                    rect,
                    if podium {
                        style.surface_raised
                    } else {
                        style.surface
                    },
                    style.metrics.radius,
                );

                let cell = rect.inset_xy(style.gap(1.2), 0.0);
                let (number, rest) = cell.cut_left(style.gap(2.6));
                list.text(
                    number,
                    format!("{}", place + 1),
                    TextStyle::new(
                        style.scaled_text(0.85),
                        if podium { style.accent } else { style.muted },
                    )
                    .bold(),
                );

                // The number keeps its own column, so a long song title is cut off before it
                // reaches the score rather than being drawn underneath it.
                let (value_box, rest) = rest.cut_right(rest.w * 0.22);
                let lines = if row.detail.is_empty() {
                    vec![rest]
                } else {
                    style.stack(rest, &[title_size, detail_size])
                };
                list.text(
                    lines[0],
                    &row.label,
                    TextStyle::new(title_size, style.text)
                        .valign(if row.detail.is_empty() {
                            VAlign::Middle
                        } else {
                            VAlign::Bottom
                        })
                        .overflow(Overflow::Ellipsis),
                );
                if let Some(under) = lines.get(1) {
                    list.text(
                        *under,
                        &row.detail,
                        TextStyle::new(detail_size, style.muted)
                            .valign(VAlign::Top)
                            .overflow(Overflow::Ellipsis),
                    );
                }
                list.text(
                    value_box,
                    &row.value,
                    TextStyle::new(title_size, style.text).align(Align::End),
                );
            }
        });

        if self.rows.len() > visible {
            list.text(
                Rect::new(
                    inner.x,
                    inner.bottom() - style.gap(2.0),
                    inner.w,
                    style.gap(2.0),
                ),
                format!(
                    "{} of {}",
                    first + visible.min(self.rows.len()),
                    self.rows.len()
                ),
                TextStyle::new(style.scaled_text(0.75), style.muted).align(Align::End),
            );
        }
    }
}
