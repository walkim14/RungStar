//! Measuring the microphone delay, with the working shown.
//!
//! The first version of this was a button that froze the game for four seconds and then put a
//! sentence in the status line. It was impossible to tell whether it was doing anything,
//! impossible to tell which microphone it had measured, and when it failed — which it does
//! whenever the sweep cannot reach the microphone — the explanation had already scrolled away.
//!
//! So it is a screen. It says which microphone of how many, which pass of how many, shows a
//! level meter so a dead device is obvious while it happens rather than afterwards, and keeps
//! every result on screen at the end next to what was actually set.

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Rect};
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// One pass, as the screen sees it.
#[derive(Debug, Clone, Copy)]
pub struct Pass {
    pub millis: f32,
    /// How well it matched, 0 to 1. Below the threshold means the sweep was not in what the
    /// microphone heard.
    pub confidence: f32,
    pub heard: bool,
    /// The loudest thing recorded, as a fraction of full scale.
    pub level: f32,
}

/// What one microphone came to.
#[derive(Debug, Clone)]
pub struct Report {
    pub name: String,
    /// Which device of this name. Two identical microphones are two microphones.
    pub occurrence: u32,
    /// The delay in milliseconds, or why there is not one.
    pub delay: Result<f32, String>,
    pub passes: Vec<Pass>,
}

impl Report {
    fn loudest(&self) -> f32 {
        self.passes.iter().map(|p| p.level).fold(0.0, f32::max)
    }

    /// What to do about it, when there is no answer.
    ///
    /// The two faults look identical from a match score alone and have entirely different
    /// fixes, so they are told apart by whether the microphone recorded anything at all.
    fn advice(&self) -> &'static str {
        if self.loudest() < 0.02 {
            "it recorded almost nothing — check it is plugged in and not muted"
        } else {
            "it works, but could not hear the sweep — use speakers, not headphones"
        }
    }
}

/// What the measurement is doing right now.
#[derive(Debug, Clone)]
pub struct Doing {
    pub device: String,
    pub device_index: usize,
    pub devices: usize,
    pub pass: usize,
    pub passes: usize,
    /// Whether the sweep is playing, or the room is being listened to first.
    pub playing: bool,
    pub level: f32,
}

/// The microphone delay screen.
#[derive(Default)]
pub struct CalibrateScreen {
    /// What is happening, while it is happening.
    pub doing: Option<Doing>,
    /// Finished microphones, in the order they were measured.
    pub reports: Vec<Report>,
    /// What was written to the setting, once there is one.
    pub applied: Option<u32>,
    /// Why nothing could be measured at all — no speakers, no microphone.
    pub trouble: Option<String>,
    pub gamepad: bool,
}

impl CalibrateScreen {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the measurement has finished, one way or another.
    pub fn finished(&self) -> bool {
        self.doing.is_none()
    }

    pub fn handle(&mut self, input: Input) -> Transition {
        // Only once it has finished. Closing it half way through would leave the sweep playing
        // into a screen that is no longer explaining it.
        if !self.finished() {
            return Transition::None;
        }
        match input {
            Input::Back | Input::Confirm | Input::Submit => Transition::Pop,
            _ => Transition::None,
        }
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);

        let heading = style.scaled_text(1.3);
        let body = style.text_size();
        let small = style.scaled_text(0.8);
        let rows = self.reports.len().max(1) + self.reports.len();
        let wanted = style.gap(6.0)
            + style.row_height(&[heading])
            + style.row_height(&[small]) * 3.0
            + style.row_height(&[body, small]) * rows as f32;
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.72).min(900.0),
            wanted.min(area.h * 0.9),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(2.0));

        let (title, rest) = inner.cut_top(style.row_height(&[heading]));
        list.text(
            title,
            "Microphone delay",
            TextStyle::new(heading, style.text).bold(),
        );

        // `cut_bottom` gives back the strip first and what is left second, which is the way
        // round that keeps catching me: taking the strip here made the body an eight-unit
        // sliver sitting on top of the footer.
        let (footer, rest) = rest.cut_bottom(style.row_height(&[small]));
        let (_, body_area) = rest.cut_bottom(style.gap(0.5));

        match (&self.trouble, &self.doing) {
            (Some(why), _) => self.draw_trouble(list, body_area, style, why),
            (None, Some(doing)) => self.draw_doing(list, body_area, style, &doing.clone()),
            (None, None) => self.draw_results(list, body_area, style),
        }

        let hint = if !self.finished() {
            "Please be quiet while it listens"
        } else if self.gamepad {
            "B to close"
        } else {
            "Esc to close"
        };
        list.text(
            footer,
            hint,
            TextStyle::new(small, style.muted).align(Align::Center),
        );
    }

    fn draw_trouble(&self, list: &mut DrawList, area: Rect, style: &Style, why: &str) {
        Widgets::new(style).empty_state(list, area, "Could not measure", why);
    }

    fn draw_doing(&mut self, list: &mut DrawList, area: Rect, style: &Style, doing: &Doing) {
        let body = style.text_size();
        let small = style.scaled_text(0.8);

        let (which, rest) = area.cut_top(style.row_height(&[body, small]));
        let lines = style.stack(which, &[body, small]);
        list.text(
            lines[0],
            &doing.device,
            TextStyle::new(body, style.text)
                .bold()
                .valign(VAlign::Bottom)
                .overflow(Overflow::Ellipsis),
        );
        let stage = if doing.playing {
            "listening for the sweep"
        } else {
            "waiting for quiet"
        };
        list.text(
            lines[1],
            format!(
                "microphone {} of {}  \u{b7}  pass {} of {}  \u{b7}  {stage}",
                doing.device_index, doing.devices, doing.pass, doing.passes
            ),
            TextStyle::new(small, style.muted).valign(VAlign::Top),
        );

        // A meter, because a microphone that is delivering nothing is the commonest reason
        // this fails and the only way to see it is to watch the level while it runs.
        let (meter, rest) = rest.cut_top(style.gap(2.5));
        let track = meter.anchored(Anchor::Left, meter.w, style.gap(0.8), 0.0);
        list.panel(track, style.surface_sunken, track.h / 2.0);
        let level = doing.level.clamp(0.0, 1.0);
        if level > 0.0 {
            list.panel(
                Rect::new(track.x, track.y, track.w * level, track.h),
                if level < 0.01 {
                    style.muted
                } else {
                    style.accent
                },
                track.h / 2.0,
            );
        }

        // Whatever is already finished, so the first microphone's answer is on screen while
        // the second is still being measured.
        self.draw_reports(list, rest, style);
    }

    fn draw_results(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        let small = style.scaled_text(0.8);
        let (summary, rest) = area.cut_top(style.row_height(&[small]) * 2.0);
        // Each microphone below carries its own figure, so what the summary is for is the
        // one number not shown beside a device: the fallback anything unmeasured runs at.
        let measured = self
            .reports
            .iter()
            .filter(|report| report.delay.is_ok())
            .count();
        let said = match self.applied {
            Some(millis) if measured > 1 => format!(
                "Each microphone keeps its own delay. {millis} ms covers anything not measured."
            ),
            Some(millis) => format!("Microphone delay set to {millis} ms, and kept as its own."),
            None => "Nothing was measured, so the delay is unchanged.".to_owned(),
        };
        list.text(
            summary,
            said,
            TextStyle::new(small, style.muted).overflow(Overflow::Ellipsis),
        );
        self.draw_reports(list, rest, style);
    }

    fn draw_reports(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let body = style.text_size();
        let small = style.scaled_text(0.8);
        let row_h = style.row_height(&[body, small]) + style.gap(0.4);

        list.clipped(area, |list| {
            for (index, report) in self.reports.iter().enumerate() {
                let rect = Rect::new(area.x, area.y + row_h * index as f32, area.w, row_h);
                if rect.bottom() > area.bottom() + row_h {
                    break;
                }
                let lines = style.stack(rect, &[body, small]);
                let (value, colour) = match &report.delay {
                    Ok(millis) => (format!("{millis:.0} ms"), style.success),
                    Err(_) => ("not measured".to_owned(), style.warning),
                };
                list.text(
                    lines[0],
                    &report.name,
                    TextStyle::new(body, style.text)
                        .valign(VAlign::Bottom)
                        .overflow(Overflow::Ellipsis),
                );
                list.text(
                    lines[0],
                    value,
                    TextStyle::new(body, colour)
                        .align(Align::End)
                        .valign(VAlign::Bottom),
                );
                // The working: how many passes heard it, and how far apart they were. Two
                // measurements twenty milliseconds apart is a different situation from five
                // that agree, and the number alone hides it.
                let detail = match &report.delay {
                    Ok(_) => {
                        let heard = report.passes.iter().filter(|p| p.heard).count();
                        let spread = spread_of(report);
                        format!(
                            "{heard} of {} passes heard it, within {spread:.0} ms",
                            report.passes.len()
                        )
                    }
                    Err(why) => format!("{why} — {}", report.advice()),
                };
                list.text(
                    lines[1],
                    detail,
                    TextStyle::new(small, style.muted)
                        .valign(VAlign::Top)
                        .overflow(Overflow::Ellipsis),
                );
            }
        });
    }
}

/// How far apart the passes that heard the sweep were.
fn spread_of(report: &Report) -> f32 {
    let heard: Vec<f32> = report
        .passes
        .iter()
        .filter(|p| p.heard)
        .map(|p| p.millis)
        .collect();
    match (
        heard.iter().copied().fold(f32::MAX, f32::min),
        heard.iter().copied().fold(f32::MIN, f32::max),
    ) {
        (low, high) if low <= high => high - low,
        _ => 0.0,
    }
}
