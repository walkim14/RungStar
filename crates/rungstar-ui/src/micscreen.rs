//! Microphone setup: which device each singer uses, with a live level meter.
//!
//! UltraStar Deluxe's record screen is a grid of dropdowns and a tiny bar, and it does not
//! tell you whether the device you picked is producing anything — which is the one question
//! it exists to answer. Here every channel shows its live level against the gate it has to
//! clear, so "my microphone does not work" is answerable without singing a song first.
//!
//! Channels, not devices, are assigned to singers. A stereo pair carries two people, which is
//! how the cheap dual-USB karaoke sets work and the only way to reach six singers without six
//! separate devices.

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
}

/// What the screen wants the application to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicOutcome {
    None,
    /// The assignment changed and capture should be restarted with it.
    Changed,
    /// Rescan for devices that have been plugged in since.
    Refresh,
}

/// The microphone setup screen.
pub struct MicScreen {
    pub devices: Vec<Device>,
    /// How many singers there are to assign.
    pub players: usize,
    /// The level a channel must reach before anything scores.
    pub gate: f32,
    pub gamepad: bool,
    /// Flattened cursor over every channel of every device, plus the refresh row at the end.
    cursor: usize,
    regions: Vec<(Rect, usize)>,
}

impl MicScreen {
    pub fn new(players: usize) -> Self {
        Self {
            devices: Vec::new(),
            players: players.max(1),
            gate: 0.1,
            gamepad: false,
            cursor: 0,
            regions: Vec::new(),
        }
    }

    /// Every selectable row: one per channel, then the refresh button.
    fn rows(&self) -> usize {
        self.devices.iter().map(Device::channels).sum::<usize>() + 1
    }

    /// Resolve a row into the device and channel it points at.
    fn locate(&self, row: usize) -> Option<(usize, usize)> {
        let mut seen = 0;
        for (device, config) in self.devices.iter().enumerate() {
            if row < seen + config.channels() {
                return Some((device, row - seen));
            }
            seen += config.channels();
        }
        None
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Which singers have at least one channel, so the screen can say who is missing one.
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

    /// Singers with no channel at all. These are the ones who cannot score.
    pub fn unassigned_players(&self) -> Vec<u8> {
        let assigned = self.assigned_players();
        (1..=self.players as u8)
            .filter(|p| !assigned.contains(p))
            .collect()
    }

    /// A singer bound to more than one channel, which would double-count them.
    pub fn duplicated_players(&self) -> Vec<u8> {
        let mut counts = [0usize; 8];
        for device in &self.devices {
            for player in &device.assignment {
                if (*player as usize) < counts.len() && *player != 0 {
                    counts[*player as usize] += 1;
                }
            }
        }
        (1..=self.players as u8)
            .filter(|p| counts[*p as usize] > 1)
            .collect()
    }

    pub fn handle(&mut self, input: Input) -> (Transition, MicOutcome) {
        let rows = self.rows();
        match input {
            Input::Up => {
                self.cursor = (self.cursor + rows - 1) % rows;
                (Transition::None, MicOutcome::None)
            }
            Input::Down => {
                self.cursor = (self.cursor + 1) % rows;
                (Transition::None, MicOutcome::None)
            }
            Input::Left | Input::Right => {
                let Some((device, channel)) = self.locate(self.cursor) else {
                    return (Transition::None, MicOutcome::None);
                };
                // Cycles off, then singer one, two and so on. Off is included because a
                // stereo device with only one microphone plugged in must be able to say so.
                let current = self.devices[device].assignment[channel] as usize;
                let steps = self.players + 1;
                let next = if matches!(input, Input::Right) {
                    (current + 1) % steps
                } else {
                    (current + steps - 1) % steps
                };
                self.devices[device].assignment[channel] = next as u8;
                (Transition::None, MicOutcome::Changed)
            }
            Input::Confirm => {
                if self.cursor + 1 == rows {
                    (Transition::None, MicOutcome::Refresh)
                } else {
                    // Confirm on a channel steps it, so the row works without learning that
                    // left and right are the ones that do anything.
                    self.handle(Input::Right)
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
        let status = format!("{} singers", self.players);
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
        if self.devices.is_empty() {
            widgets.empty_state(
                list,
                inner,
                "No microphones found",
                "Plug one in and choose Look again. Devices that only loop audio back — \
                 Steam's streaming microphone and the like — are skipped, because they \
                 deliver silence forever and look exactly like a broken setup.",
            );
            // The refresh row still has to be reachable with nothing listed.
            let row = inner.anchored(Anchor::Bottom, inner.w.min(600.0), style.gap(3.4), 0.0);
            self.regions.push((row, 0));
            widgets.row(list, row, "Look again", "", true);
            return;
        }

        let row_h = style.gap(3.6);
        let mut y = inner.y;
        let mut row = 0;
        for device in &self.devices {
            let header = Rect::new(inner.x, y, inner.w, style.gap(2.6));
            list.text(
                header,
                &device.name,
                TextStyle::new(style.scaled_text(0.95), style.muted)
                    .bold()
                    .valign(VAlign::Bottom)
                    .overflow(Overflow::Ellipsis),
            );
            y += header.h;

            for channel in 0..device.channels() {
                let rect = Rect::new(inner.x, y, inner.w, row_h).inset_xy(0.0, style.gap(0.25));
                self.regions.push((rect, row));
                self.draw_channel(list, rect, style, device, channel, row == self.cursor);
                y += row_h;
                row += 1;
            }
            y += style.gap(1.0);
        }

        let refresh = Rect::new(inner.x, y, inner.w, row_h).inset_xy(0.0, style.gap(0.25));
        self.regions.push((refresh, row));
        widgets.row(list, refresh, "Look again", "", row == self.cursor);
    }

    fn draw_channel(
        &self,
        list: &mut DrawList,
        rect: Rect,
        style: &Style,
        device: &Device,
        channel: usize,
        selected: bool,
    ) {
        let player = device.assignment.get(channel).copied().unwrap_or(0);
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

        let inner = rect.inset_xy(style.gap(1.2), 0.0);
        let (label_box, rest) = inner.cut_left(inner.w * 0.22);
        list.text(
            label_box,
            device.channel_name(channel),
            TextStyle::new(style.text_size(), style.text),
        );

        // The live meter, against the gate it has to clear. This is the whole point of the
        // screen: a device that is selected but silent is otherwise indistinguishable from a
        // device that is working and a room that is quiet.
        let (assign_box, meter_box) = rest.cut_right(rest.w * 0.34);
        let track = meter_box.inset_xy(style.gap(0.5), 0.0).anchored(
            Anchor::Center,
            meter_box.w - style.gap(1.0),
            style.gap(0.6),
            0.0,
        );
        list.panel(track, style.surface_sunken, track.h / 2.0);

        let level = device
            .levels
            .get(channel)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
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

        let heard = device.heard.get(channel).copied().unwrap_or(false);
        let note = if player == 0 {
            "off".to_owned()
        } else if !heard {
            "silent".to_owned()
        } else if level < self.gate {
            "too quiet".to_owned()
        } else {
            "singing".to_owned()
        };
        let note_colour = if player == 0 {
            style.muted
        } else if !heard {
            style.danger
        } else if level < self.gate {
            style.warning
        } else {
            style.success
        };
        list.text(
            Rect::new(track.x, rect.y, track.w, rect.h),
            note,
            TextStyle::new(style.scaled_text(0.72), note_colour)
                .align(Align::End)
                .valign(VAlign::Top),
        );

        let assignment = if player == 0 {
            "\u{2014}".to_owned()
        } else {
            format!("Player {player}")
        };
        list.text(
            assign_box,
            assignment,
            TextStyle::new(style.text_size(), colour).align(Align::End),
        );
    }

    fn draw_warnings(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let missing = self.unassigned_players();
        let doubled = self.duplicated_players();
        let (text, colour) = if !doubled.is_empty() {
            (
                format!(
                    "Player {} is on more than one channel and would be scored twice.",
                    join_players(&doubled)
                ),
                style.danger,
            )
        } else if !missing.is_empty() {
            (
                format!(
                    "Player {} has no microphone and cannot score.",
                    join_players(&missing)
                ),
                style.warning,
            )
        } else {
            ("Every singer has a channel.".to_owned(), style.success)
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
