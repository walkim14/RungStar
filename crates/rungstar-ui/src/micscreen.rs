//! Microphone setup: which singer each microphone belongs to, with a live level meter.
//!
//! UltraStar Deluxe's record screen is a grid of dropdowns and a tiny bar, and it does not tell
//! you whether the device you picked is producing anything — which is the one question it
//! exists to answer. Here every microphone shows its live level against the gate it has to
//! clear, so "my microphone does not work" is answerable without singing a song first.
//!
//! **One row per microphone by default, not per channel.** Almost every USB microphone reports
//! two channels and is mono on both, so a channel list showed two rows for one microphone and
//! invited putting two singers on it — which cannot work: the capture layer appends each
//! channel to its player's buffer, so two channels feeding one player would interleave two
//! streams and wreck the pitch detection.
//!
//! The case that genuinely wants a split is the cheap dual-USB karaoke set, where left and
//! right really are two microphones. That is a real setup and worth supporting, so it is a
//! setting rather than a deletion — off by default, because for everyone else it is two rows
//! for one microphone and a way to get it wrong.

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Rect};
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// One capture device as the screen sees it.
#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    /// One entry per channel: `0` for off, otherwise a one-based singer number.
    ///
    /// With channel splitting off only the first entry is used, and the rest are held at zero.
    pub assignment: Vec<u8>,
    /// Live peak level per channel, `0.0..=1.0`.
    pub levels: Vec<f32>,
    /// Whether a sample has ever arrived on each channel.
    pub heard: Vec<bool>,
}

impl Device {
    pub fn channels(&self) -> usize {
        self.assignment.len()
    }

    /// The name a channel goes by. Stereo devices get Left and Right, which is what is
    /// written on the hardware.
    pub fn channel_name(&self, channel: usize) -> String {
        match (self.channels(), channel) {
            (1, _) => "Mono".to_owned(),
            (2, 0) => "Left".to_owned(),
            (2, 1) => "Right".to_owned(),
            _ => format!("Channel {}", channel + 1),
        }
    }

    /// The singer this microphone belongs to, when channels are not split.
    pub fn player(&self) -> u8 {
        self.assignment
            .iter()
            .copied()
            .find(|p| *p != 0)
            .unwrap_or(0)
    }

    /// The loudest channel, which is the one worth listening to on a device that only carries
    /// signal on one of them.
    pub fn peak(&self) -> f32 {
        self.levels.iter().copied().fold(0.0, f32::max)
    }

    pub fn ever_heard(&self) -> bool {
        self.heard.iter().any(|h| *h)
    }
}

/// What the screen wants the application to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicOutcome {
    None,
    /// The assignment changed and capture should be restarted with it.
    Changed,
    /// Look for devices that have been plugged in since.
    Refresh,
}

/// The most singers the game will run, matching the capture layer's own ceiling.
pub const MAX_PLAYERS: usize = 6;

/// One selectable row: a whole microphone, or one channel of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    device: usize,
    /// `None` when the row is the whole device.
    channel: Option<usize>,
}

/// The microphone setup screen.
pub struct MicScreen {
    pub devices: Vec<Device>,
    /// The level a microphone must reach before anything scores.
    pub gate: f32,
    pub gamepad: bool,
    /// Whether each channel is assigned separately, for a dual-microphone device.
    pub split_channels: bool,
    cursor: usize,
    regions: Vec<(Rect, usize)>,
}

impl Default for MicScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl MicScreen {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            gate: 0.1,
            gamepad: false,
            split_channels: false,
            cursor: 0,
            regions: Vec::new(),
        }
    }

    /// Every selectable row, in the order they are drawn.
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (device, config) in self.devices.iter().enumerate() {
            if self.split_channels {
                for channel in 0..config.channels() {
                    rows.push(Row {
                        device,
                        channel: Some(channel),
                    });
                }
            } else {
                rows.push(Row {
                    device,
                    channel: None,
                });
            }
        }
        rows
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Singer numbers in use, in order.
    pub fn assigned_players(&self) -> Vec<u8> {
        let mut seen: Vec<u8> = self
            .devices
            .iter()
            .flat_map(|d| d.assignment.iter().copied())
            .filter(|p| *p != 0)
            .collect();
        seen.sort_unstable();
        seen.dedup();
        seen
    }

    /// How many are playing, which is the highest singer number in use.
    pub fn singer_count(&self) -> usize {
        self.assigned_players().last().copied().unwrap_or(0) as usize
    }

    /// Singer numbers skipped over — player three assigned with no player two.
    ///
    /// Not fatal, but almost always a mistake, and silently renumbering somebody would be
    /// worse than saying so.
    pub fn skipped_players(&self) -> Vec<u8> {
        let assigned = self.assigned_players();
        (1..=self.singer_count() as u8)
            .filter(|p| !assigned.contains(p))
            .collect()
    }

    /// Singers on more than one input, who would be scored twice.
    pub fn duplicated_players(&self) -> Vec<u8> {
        let mut counts = [0usize; MAX_PLAYERS + 2];
        for device in &self.devices {
            for player in &device.assignment {
                if *player != 0 {
                    if let Some(slot) = counts.get_mut(*player as usize) {
                        *slot += 1;
                    }
                }
            }
        }
        (1..=MAX_PLAYERS as u8)
            .filter(|p| counts[*p as usize] > 1)
            .collect()
    }

    /// Step the singer on the row under the cursor.
    fn step(&mut self, forward: bool) -> MicOutcome {
        let rows = self.rows();
        let Some(row) = rows.get(self.cursor).copied() else {
            return MicOutcome::None;
        };
        let steps = MAX_PLAYERS + 1;
        let device = &mut self.devices[row.device];

        match row.channel {
            Some(channel) => {
                let current = device.assignment[channel] as usize;
                device.assignment[channel] = if forward {
                    ((current + 1) % steps) as u8
                } else {
                    ((current + steps - 1) % steps) as u8
                };
            }
            None => {
                // The whole device. The singer goes on whichever channel is actually carrying
                // signal — some microphones put audio only on the right — and the rest are
                // held off, because two channels feeding one player would interleave two
                // copies of the same voice.
                let current = device
                    .assignment
                    .iter()
                    .copied()
                    .find(|p| *p != 0)
                    .unwrap_or(0) as usize;
                let next = if forward {
                    (current + 1) % steps
                } else {
                    (current + steps - 1) % steps
                };
                let loudest = device
                    .levels
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(index, level)| if *level > 0.0 { index } else { 0 })
                    .unwrap_or(0);
                for slot in device.assignment.iter_mut() {
                    *slot = 0;
                }
                if let Some(slot) = device.assignment.get_mut(loudest) {
                    *slot = next as u8;
                }
            }
        }
        MicOutcome::Changed
    }

    pub fn handle(&mut self, input: Input) -> (Transition, MicOutcome) {
        let total = self.rows().len() + 1;
        match input {
            Input::Up => {
                self.cursor = (self.cursor + total - 1) % total;
                (Transition::None, MicOutcome::None)
            }
            Input::Down => {
                self.cursor = (self.cursor + 1) % total;
                (Transition::None, MicOutcome::None)
            }
            Input::Left => (Transition::None, self.step(false)),
            Input::Right => (Transition::None, self.step(true)),
            Input::Confirm => {
                if self.cursor + 1 == total {
                    (Transition::None, MicOutcome::Refresh)
                } else {
                    // Confirm steps it, so a row works without learning that left and right
                    // are the ones that do anything.
                    (Transition::None, self.step(true))
                }
            }
            Input::Back => (Transition::Pop, MicOutcome::None),
            Input::Hover(point) | Input::Click(point) => {
                if let Some((_, row)) = self.regions.iter().find(|(r, _)| r.contains(point)) {
                    self.cursor = *row;
                    if matches!(input, Input::Click(_)) {
                        return self.handle(Input::Confirm);
                    }
                }
                (Transition::None, MicOutcome::None)
            }
            _ => (Transition::None, MicOutcome::None),
        }
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.regions.clear();
        let widgets = Widgets::new(style);
        let count = self.singer_count();
        let status = match count {
            0 => "nobody assigned".to_owned(),
            1 => "1 singer".to_owned(),
            n => format!("{n} singers"),
        };
        let body = widgets.header(list, area, "Microphones", &status);
        let hints: &[(&str, &str)] = if self.gamepad {
            &[("A", "Assign"), ("LS", "Change"), ("B", "Back")]
        } else {
            &[
                ("Enter", "Assign"),
                ("\u{2190}\u{2192}", "Change"),
                ("Esc", "Back"),
            ]
        };
        let body = widgets.footer(list, body, hints);

        let (warning_area, body) = body.cut_bottom(style.gap(3.0));
        self.draw_warnings(list, warning_area, style);

        let inner = body.inset(style.gap(2.0));
        let row_h = style.gap(4.0);
        let rows = self.rows();

        if self.devices.is_empty() {
            widgets.empty_state(
                list,
                inner,
                "No microphones found",
                "Plug one in and choose Look again. Devices that only loop audio back — \
                 Steam's streaming microphone and the like — are skipped, because they deliver \
                 silence forever and look exactly like a broken setup.",
            );
            let row = inner.anchored(Anchor::Bottom, inner.w.min(600.0), row_h, 0.0);
            self.regions.push((row, 0));
            widgets.row(list, row, "Look again", "", true);
            return;
        }

        for (index, row) in rows.iter().enumerate() {
            let rect = Rect::new(inner.x, inner.y + row_h * index as f32, inner.w, row_h)
                .inset_xy(0.0, style.gap(0.3));
            self.regions.push((rect, index));
            self.draw_row(list, rect, style, *row, index == self.cursor);
        }

        let refresh = Rect::new(inner.x, inner.y + row_h * rows.len() as f32, inner.w, row_h)
            .inset_xy(0.0, style.gap(0.3));
        self.regions.push((refresh, rows.len()));
        widgets.row(list, refresh, "Look again", "", self.cursor == rows.len());
    }

    fn draw_row(&self, list: &mut DrawList, rect: Rect, style: &Style, row: Row, selected: bool) {
        let device = &self.devices[row.device];
        let (player, level, heard, name) = match row.channel {
            Some(channel) => (
                device.assignment.get(channel).copied().unwrap_or(0),
                device.levels.get(channel).copied().unwrap_or(0.0),
                device.heard.get(channel).copied().unwrap_or(false),
                format!("{}  ·  {}", device.name, device.channel_name(channel)),
            ),
            None => (
                device.player(),
                device.peak(),
                device.ever_heard(),
                device.name.clone(),
            ),
        };

        let colour = if player == 0 {
            style.muted
        } else {
            style.player(player as usize - 1)
        };
        list.panel(
            rect,
            if selected {
                style.surface_raised
            } else {
                style.surface
            },
            style.metrics.radius,
        );
        if selected {
            list.outline(
                rect,
                style.accent,
                style.metrics.outline,
                style.metrics.radius,
            );
        }

        let inner = rect.inset_xy(style.gap(1.2), style.gap(0.4));
        let (top, bottom) = inner.cut_top(inner.h * 0.55);

        // Device names are long and full of parentheses, so the name gets its own column and
        // is cut off at the edge of it rather than running under the assignment.
        let (name_box, assignment_box) = top.cut_left(top.w * 0.72);
        list.text(
            Rect::new(
                name_box.x,
                name_box.y,
                (name_box.w - style.gap(1.0)).max(0.0),
                name_box.h,
            ),
            name,
            TextStyle::new(style.text_size(), style.text)
                .valign(VAlign::Bottom)
                .overflow(Overflow::Ellipsis),
        );
        let assignment = if player == 0 {
            "Not in use".to_owned()
        } else {
            format!("Player {player}")
        };
        list.text(
            assignment_box,
            assignment,
            TextStyle::new(style.text_size(), colour)
                .align(Align::End)
                .valign(VAlign::Bottom),
        );

        // The live meter against the gate it has to clear. This is the whole point of the
        // screen: a microphone that is selected but silent is otherwise indistinguishable from
        // one that works in a quiet room.
        let (meter, label) = bottom.cut_left(bottom.w * 0.62);
        let track = meter.anchored(Anchor::Left, meter.w - style.gap(1.0), style.gap(0.6), 0.0);
        list.panel(track, style.surface_sunken, track.h / 2.0);
        let level = level.clamp(0.0, 1.0);
        if level > 0.0 {
            list.panel(
                Rect::new(track.x, track.y, track.w * level, track.h),
                if level >= self.gate {
                    colour
                } else {
                    style.muted
                },
                track.h / 2.0,
            );
        }
        let gate_x = track.x + track.w * self.gate.clamp(0.0, 1.0);
        list.fill(
            Rect::new(gate_x - 1.5, track.y - 3.0, 3.0, track.h + 6.0),
            style.warning,
        );

        let (note, tint) = if !heard {
            // Distinguishes a device that has never produced a sample — unplugged, or an
            // input the machine lists but nothing is connected to — from one that is simply
            // being sung into quietly.
            ("nothing arriving", style.danger)
        } else if player == 0 {
            ("not in use", style.muted)
        } else if level < self.gate {
            ("too quiet", style.warning)
        } else {
            ("hearing you", style.success)
        };
        list.text(
            label,
            note,
            TextStyle::new(style.scaled_text(0.78), tint).align(Align::End),
        );
    }

    fn draw_warnings(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let skipped = self.skipped_players();
        let doubled = self.duplicated_players();
        let count = self.singer_count();
        let (text, colour) = if !doubled.is_empty() {
            (
                format!(
                    "Player {} is on more than one input and would be scored twice.",
                    join_players(&doubled)
                ),
                style.danger,
            )
        } else if !skipped.is_empty() {
            (
                format!(
                    "Player {} is skipped. Numbering should start at one and leave no gaps.",
                    join_players(&skipped)
                ),
                style.warning,
            )
        } else if count == 0 {
            (
                "No microphone is assigned, so nothing will score.".to_owned(),
                style.warning,
            )
        } else {
            (
                format!(
                    "{count} singer{}. That is how many the game will score.",
                    if count == 1 { "" } else { "s" }
                ),
                style.success,
            )
        };
        list.text(
            area.inset_xy(style.gap(2.0), 0.0),
            text,
            TextStyle::new(style.scaled_text(0.85), colour),
        );
    }
}

fn join_players(players: &[u8]) -> String {
    players
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
