//! The main menu and the options screens.
//!
//! Both are the same shape — a cursor over rows, drawn with the shared widgets — which is the
//! point: UltraStar Deluxe has ten hand-written options screens that each place their own
//! widgets, and they disagree with each other about what a selected row looks like.

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Rect};
use crate::menu::Cursor;
use crate::options::{Action, Control, Page};
use crate::screen::{Route, Transition, Widgets};
use crate::settings::Settings;
use crate::songselect::Input;
use crate::theme::Style;

/// One entry on the main menu.
struct Entry {
    label: &'static str,
    detail: &'static str,
    route: Option<Route>,
    quit: bool,
}

/// The first screen.
pub struct MainMenu {
    cursor: Cursor,
    entries: Vec<Entry>,
    /// Clickable rows from the last frame, so hit testing cannot drift from the picture.
    regions: Vec<Rect>,
    /// Label the hints for a gamepad rather than a keyboard.
    pub gamepad: bool,
}

impl Default for MainMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl MainMenu {
    pub fn new() -> Self {
        let entries = vec![
            Entry {
                label: "Sing",
                detail: "Choose a song and sing it",
                route: Some(Route::SongSelect),
                quit: false,
            },
            Entry {
                label: "Singers",
                detail: "Who is playing, and their scores",
                route: Some(Route::Players),
                quit: false,
            },
            Entry {
                label: "Statistics",
                detail: "Best scores, best singers, most-sung songs",
                route: Some(Route::Stats),
                quit: false,
            },
            Entry {
                label: "Options",
                detail: "Microphones, graphics, difficulty and appearance",
                route: Some(Route::Options),
                quit: false,
            },
            Entry {
                label: "About",
                detail: "Version, licence and credits",
                route: Some(Route::About),
                quit: false,
            },
            Entry {
                label: "Quit",
                detail: "Leave the game",
                route: None,
                quit: true,
            },
        ];
        Self {
            cursor: Cursor::new(entries.len()),
            entries,
            regions: Vec::new(),
            gamepad: false,
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor.index()
    }

    pub fn handle(&mut self, input: Input) -> Transition {
        if let Input::Hover(point) | Input::Click(point) = input {
            if let Some(index) = self.regions.iter().position(|r| r.contains(point)) {
                self.cursor.set(index);
                if matches!(input, Input::Click(_)) {
                    return self.handle(Input::Confirm);
                }
            }
            return Transition::None;
        }
        match input {
            Input::Up => self.cursor.move_by(-1),
            Input::Down => self.cursor.move_by(1),
            Input::Confirm => {
                let entry = &self.entries[self.cursor.index()];
                if entry.quit {
                    return Transition::Quit;
                }
                if let Some(route) = &entry.route {
                    return Transition::Push(route.clone());
                }
            }
            // Back on the first screen is a quit request, but the application confirms it
            // rather than acting on it — an accidental B press should not end the party.
            Input::Back => return Transition::Quit,
            _ => {}
        }
        Transition::None
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style, subtitle: &str) {
        self.regions.clear();
        let widgets = Widgets::new(style);
        let hints: &[(&str, &str)] = if self.gamepad {
            &[("A", "Choose"), ("B", "Quit")]
        } else {
            &[("Enter", "Choose"), ("Esc", "Quit")]
        };
        let body = widgets.footer(list, area, hints);

        // Wordmark on the left, menu on the right: the menu stays a readable width on an
        // ultrawide instead of stretching across it.
        let content = body.anchored(
            Anchor::Center,
            body.w.min(1500.0),
            body.h.min(700.0),
            style.gap(2.0),
        );
        let (left, right) = content.cut_left(content.w * 0.5);

        let title_box = left.anchored(Anchor::Left, left.w, style.gap(12.0), style.gap(2.0));
        let (title, sub) = title_box.cut_top(style.gap(7.0));
        list.text(
            title,
            "RungStar",
            TextStyle::new(style.scaled_text(3.4), style.text)
                .bold()
                .valign(VAlign::Bottom),
        );
        list.text(
            sub,
            subtitle,
            TextStyle::new(style.text_size(), style.muted).valign(VAlign::Top),
        );

        let row_h = style.gap(4.5);
        let menu = right.anchored(
            Anchor::Center,
            right.w.min(560.0),
            row_h * self.entries.len() as f32,
            style.gap(2.0),
        );
        for (index, entry) in self.entries.iter().enumerate() {
            let row = Rect::new(menu.x, menu.y + row_h * index as f32, menu.w, row_h)
                .inset_xy(0.0, style.gap(0.4));
            let selected = index == self.cursor.index();
            self.regions.push(row);
            list.panel(
                row,
                if selected {
                    style.accent
                } else {
                    style.surface
                },
                style.metrics.radius,
            );
            let (text, secondary) = if selected {
                (style.on_accent, style.on_accent.alpha(0.8))
            } else {
                (style.text, style.muted)
            };
            let inner = row.inset_xy(style.gap(1.5), style.gap(0.5));
            let (top, bottom) = inner.cut_top(inner.h * 0.56);
            list.text(
                top,
                entry.label,
                TextStyle::new(style.scaled_text(1.15), text)
                    .bold()
                    .valign(VAlign::Bottom),
            );
            list.text(
                bottom,
                entry.detail,
                TextStyle::new(style.scaled_text(0.8), secondary)
                    .valign(VAlign::Top)
                    .overflow(Overflow::Ellipsis),
            );
        }
    }
}

/// What an options screen wants the application to do, beyond changing a setting.
#[derive(Debug, Clone, PartialEq)]
pub enum OptionsOutcome {
    None,
    Pop,
    /// A button was pressed and the application has to carry it out.
    Run(Action),
    /// A setting changed. The application saves, and re-resolves the theme if it has to.
    Changed,
}

/// One page of settings.
pub struct OptionsScreen {
    pages: Vec<Page>,
    page_cursor: Cursor,
    item_cursor: Cursor,
    /// `true` while the page list has focus rather than the items on it.
    on_page_list: bool,
    page_regions: Vec<Rect>,
    /// Item rows from the last frame, with the index each one shows.
    item_regions: Vec<(Rect, usize)>,
    pub gamepad: bool,
}

impl Default for OptionsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl OptionsScreen {
    pub fn new() -> Self {
        let pages = Page::all();
        let items = pages[0].items.len();
        Self {
            page_cursor: Cursor::new(pages.len()),
            item_cursor: Cursor::new(items),
            pages,
            on_page_list: true,
            page_regions: Vec::new(),
            item_regions: Vec::new(),
            gamepad: false,
        }
    }

    pub fn page_index(&self) -> usize {
        self.page_cursor.index()
    }

    pub fn item_index(&self) -> usize {
        self.item_cursor.index()
    }

    pub fn on_page_list(&self) -> bool {
        self.on_page_list
    }

    fn page(&self) -> &Page {
        &self.pages[self.page_cursor.index()]
    }

    /// The help text for whatever is under the cursor.
    pub fn help(&self) -> &str {
        if self.on_page_list {
            "Pick a group of settings. Everything changes as you move, and is saved as you go."
        } else {
            self.page().items[self.item_cursor.index()].help
        }
    }

    pub fn handle(&mut self, input: Input, settings: &mut Settings) -> OptionsOutcome {
        if let Input::Hover(point) | Input::Click(point) = input {
            let clicked = matches!(input, Input::Click(_));
            if let Some(index) = self.page_regions.iter().position(|r| r.contains(point)) {
                self.page_cursor.set(index);
                self.item_cursor = Cursor::new(self.pages[index].items.len());
                // Pointing at a group previews it; the items only take focus on a click, so
                // sweeping the pointer across the list does not steal the cursor.
                self.on_page_list = !clicked;
                return OptionsOutcome::None;
            }
            if let Some((_, index)) = self.item_regions.iter().find(|(r, _)| r.contains(point)) {
                let index = *index;
                self.item_cursor.set(index);
                self.on_page_list = false;
                if clicked {
                    return self.handle(Input::Confirm, settings);
                }
            }
            return OptionsOutcome::None;
        }
        if self.on_page_list {
            match input {
                Input::Up => self.page_cursor.move_by(-1),
                Input::Down => self.page_cursor.move_by(1),
                Input::Right | Input::Confirm => {
                    self.on_page_list = false;
                    self.item_cursor = Cursor::new(self.page().items.len());
                }
                Input::Back => return OptionsOutcome::Pop,
                _ => {}
            }
            if matches!(input, Input::Up | Input::Down) {
                self.item_cursor = Cursor::new(self.page().items.len());
            }
            return OptionsOutcome::None;
        }

        match input {
            Input::Up => self.item_cursor.move_by(-1),
            Input::Down => self.item_cursor.move_by(1),
            Input::Back => {
                self.on_page_list = true;
                return OptionsOutcome::None;
            }
            Input::Left | Input::Right => {
                let item = &self.pages[self.page_cursor.index()].items[self.item_cursor.index()];
                if item.is_button() {
                    return OptionsOutcome::None;
                }
                item.adjust(settings, if matches!(input, Input::Right) { 1 } else { -1 });
                settings.clamp();
                return OptionsOutcome::Changed;
            }
            Input::Confirm => {
                let item = &self.pages[self.page_cursor.index()].items[self.item_cursor.index()];
                if let Control::Button(action) = item.control {
                    return OptionsOutcome::Run(action);
                }
                // Confirm on a choice steps it forward, so the row can be used without
                // learning that left and right are the ones that do anything.
                item.adjust(settings, 1);
                settings.clamp();
                return OptionsOutcome::Changed;
            }
            _ => {}
        }
        OptionsOutcome::None
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style, settings: &Settings) {
        self.page_regions.clear();
        self.item_regions.clear();
        let widgets = Widgets::new(style);
        let body = widgets.header(list, area, "Options", "");
        let hints: &[(&str, &str)] = match (self.on_page_list, self.gamepad) {
            (true, true) => &[("A", "Open"), ("B", "Back")],
            (true, false) => &[("Enter", "Open"), ("Esc", "Back")],
            (false, true) => &[("A", "Change"), ("LS", "Adjust"), ("B", "Groups")],
            (false, false) => &[
                ("Enter", "Change"),
                ("\u{2190}\u{2192}", "Adjust"),
                ("Esc", "Groups"),
            ],
        };
        let body = widgets.footer(list, body, hints);

        // Help sits under the list rather than beside it, so a long explanation does not
        // squeeze the values into an unreadable column.
        // `cut_bottom` returns the strip first and what is left second, in that order.
        let (help_area, body) = body.cut_bottom(style.gap(4.0));
        list.text(
            help_area.inset_xy(style.gap(2.0), 0.0),
            self.help(),
            TextStyle::new(style.scaled_text(0.85), style.muted).valign(VAlign::Top),
        );

        let (pages_area, items_area) = body.cut_left((body.w * 0.28).min(420.0));
        self.draw_pages(list, pages_area.inset(style.gap(1.5)), style);
        self.draw_items(list, items_area.inset(style.gap(1.5)), style, settings);
    }

    fn draw_pages(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        let row_h = style.gap(3.4);
        for (index, page) in self.pages.iter().enumerate() {
            let row = Rect::new(area.x, area.y + row_h * index as f32, area.w, row_h)
                .inset_xy(0.0, style.gap(0.25));
            let selected = index == self.page_cursor.index();
            self.page_regions.push(row);
            widgets.row(list, row, page.title, "", selected);
            // The group keeps a marker while the items have focus, so it stays clear which
            // page the values on the right belong to.
            if selected && !self.on_page_list {
                list.outline(
                    row,
                    style.accent,
                    style.metrics.outline,
                    style.metrics.radius,
                );
            }
        }
    }

    fn draw_items(&mut self, list: &mut DrawList, area: Rect, style: &Style, settings: &Settings) {
        let page = &self.pages[self.page_cursor.index()];
        let row_h = style.gap(3.4);
        let visible = ((area.h / row_h).floor() as usize).max(1);
        // Scroll so the cursor stays on screen, keeping as much context above it as fits.
        let first = self
            .item_cursor
            .index()
            .saturating_sub(visible.saturating_sub(2))
            .min(page.items.len().saturating_sub(visible));

        // Copied out before the closure, which cannot also borrow `self`.
        let on_page_list = self.on_page_list;
        let cursor = self.item_cursor.index();
        let mut regions: Vec<(Rect, usize)> = Vec::new();
        list.clipped(area, |list| {
            for (offset, item) in page.items.iter().skip(first).take(visible).enumerate() {
                let row = Rect::new(area.x, area.y + row_h * offset as f32, area.w, row_h)
                    .inset_xy(0.0, style.gap(0.25));
                let selected = !on_page_list && first + offset == cursor;
                regions.push((row, first + offset));
                let value = item.value(settings);

                list.panel(
                    row,
                    if selected {
                        style.accent
                    } else {
                        style.surface
                    },
                    style.metrics.radius,
                );
                let (text, secondary) = if selected {
                    (style.on_accent, style.on_accent.alpha(0.85))
                } else {
                    (style.text, style.muted)
                };
                let inner = row.inset_xy(style.gap(1.2), 0.0);
                list.text(
                    inner,
                    item.label,
                    TextStyle::new(style.text_size(), text).overflow(Overflow::Ellipsis),
                );

                match item.fraction(settings) {
                    // A number gets a bar as well as its value: "140 ms" means nothing on its
                    // own, but a bar a third of the way along says how much room is left.
                    Some(fraction) => {
                        let (value_box, bar_box) = inner.cut_right(inner.w * 0.28);
                        list.text(
                            value_box,
                            &value,
                            TextStyle::new(style.text_size(), text).align(Align::End),
                        );
                        let bar = bar_box.cut_right(bar_box.w * 0.5).0.anchored(
                            Anchor::Center,
                            bar_box.w * 0.45,
                            row.h,
                            0.0,
                        );
                        Widgets::new(style).slider(list, bar, fraction, selected);
                    }
                    None if item.is_button() => {
                        list.text(
                            inner,
                            "\u{203a}",
                            TextStyle::new(style.text_size(), secondary).align(Align::End),
                        );
                    }
                    None => {
                        list.text(
                            inner,
                            &value,
                            TextStyle::new(style.text_size(), text)
                                .align(Align::End)
                                .overflow(Overflow::Ellipsis),
                        );
                    }
                }
            }
        });

        // A hint that the list continues, rather than leaving it to be discovered.
        let total = page.items.len();
        if total > visible {
            let shown = format!("{}/{}", self.item_cursor.index() + 1, total);
            list.text(
                Rect::new(
                    area.x,
                    area.bottom() - style.gap(2.0),
                    area.w,
                    style.gap(2.0),
                ),
                shown,
                TextStyle::new(style.scaled_text(0.75), style.muted).align(Align::End),
            );
        }
        self.item_regions = regions;
    }
}
