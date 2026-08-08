//! The sing screen: notes, lyrics, per-singer panels, pause menu and results.
//!
//! Pure like every other screen — it is handed the state of the song and the singers and
//! produces a display list. The audio clock, the microphones and the scorer live in the
//! application, because those are the parts that touch devices.
//!
//! That split is what makes multiple singers a layout question rather than a rewrite: the
//! screen takes a slice of singers and splits the panel area by its length, so one singer and
//! six are the same code.

use crate::color::Color;
use crate::draw::{Align, DrawList, Font, ImageId, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Point, Rect};
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// What a note looks like on the staff. Mirrors the song format's note kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Normal,
    Golden,
    Freestyle,
    Rap,
    GoldenRap,
}

impl NoteKind {
    pub fn is_golden(self) -> bool {
        matches!(self, Self::Golden | Self::GoldenRap)
    }

    /// Freestyle notes score nothing, so they are drawn as an outline rather than a bar.
    pub fn is_freestyle(self) -> bool {
        matches!(self, Self::Freestyle)
    }
}

/// One note to draw.
#[derive(Debug, Clone, Copy)]
pub struct Note {
    pub start: f64,
    pub duration: f64,
    pub pitch: i32,
    pub kind: NoteKind,
}

impl Note {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

/// A stretch the player actually sang, for drawing over the target note.
#[derive(Debug, Clone, Copy)]
pub struct Sung {
    pub start: f64,
    pub duration: f64,
    pub pitch: i32,
    pub hit: bool,
}

/// One syllable of the line being sung.
#[derive(Debug, Clone)]
pub struct Syllable {
    pub text: String,
    pub start: f64,
    pub duration: f64,
    pub golden: bool,
}

/// Everything about one singer that the screen shows.
pub struct Singer {
    pub name: String,
    /// Points so far, already rounded.
    pub score: i32,
    /// `0.0..=1.0` through the maximum.
    pub fraction: f32,
    /// Peak input level of the last analysis window, `0.0..=1.0`.
    pub level: f32,
    /// The gate below which detection does not run.
    pub gate: f32,
    /// Semitone the singer is currently on, if any was detected.
    pub pitch: Option<i32>,
    /// Whether the last scored beat counted.
    pub hitting: Option<bool>,
    /// Whether a single sample has ever arrived from this singer's device.
    pub ever_heard: bool,
    pub has_microphone: bool,
    /// Line bonus rating, `0..=8`, and how long ago it was awarded in seconds.
    pub rating: Option<(i32, f32)>,
    /// What this singer has sung recently, for drawing over the notes.
    pub sung: Vec<Sung>,
}

impl Singer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            score: 0,
            fraction: 0.0,
            level: 0.0,
            gate: 0.1,
            pitch: None,
            hitting: None,
            ever_heard: false,
            has_microphone: false,
            rating: None,
            sung: Vec::new(),
        }
    }

    /// What is wrong with this singer's input, if anything.
    ///
    /// The three failures are separate because they fail independently, and the first version
    /// of this screen made them look identical: a dead microphone and a silent room both drew
    /// nothing at all.
    pub fn input_problem(&self) -> Option<&'static str> {
        if !self.has_microphone {
            Some("no microphone")
        } else if !self.ever_heard {
            Some("no audio arriving")
        } else if self.level < self.gate {
            Some("too quiet to score")
        } else {
            None
        }
    }
}

/// What the screen is showing on top of the song.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    Paused,
    /// The song has ended and the scores are up.
    Results,
}

/// The pause menu's entries.
pub const PAUSE_ENTRIES: [&str; 3] = ["Continue", "Restart", "Give up"];

/// What the player chose from the pause menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseChoice {
    Continue,
    Restart,
    Quit,
}

/// The sing screen.
pub struct SingScreen {
    pub title: String,
    pub artist: String,
    pub singers: Vec<Singer>,
    pub overlay: Overlay,
    /// Background artwork, when the song has one and backgrounds are on.
    pub background: Option<ImageId>,
    /// Seconds into the song, for the progress bar.
    pub position: f32,
    pub duration: f32,
    pub gamepad: bool,
    pub show_input_panel: bool,
    pause_cursor: usize,
}

/// Semitones shown on the staff either side of the song's own range.
const STAFF_MARGIN: i32 = 2;

/// How long a line-bonus rating stays on screen, in seconds.
const RATING_LIFETIME: f32 = 1.4;

impl SingScreen {
    pub fn new(artist: impl Into<String>, title: impl Into<String>, singers: usize) -> Self {
        Self {
            title: title.into(),
            artist: artist.into(),
            singers: (0..singers.max(1))
                .map(|i| Singer::new(format!("Player {}", i + 1)))
                .collect(),
            overlay: Overlay::None,
            background: None,
            position: 0.0,
            duration: 0.0,
            gamepad: false,
            show_input_panel: false,
            pause_cursor: 0,
        }
    }

    pub fn pause_cursor(&self) -> usize {
        self.pause_cursor
    }

    /// Handle an input. Returns a transition, and separately what the pause menu chose.
    pub fn handle(&mut self, input: Input) -> (Transition, Option<PauseChoice>) {
        match self.overlay {
            Overlay::None => match input {
                // Back pauses rather than quitting. Leaving a song by accident in the middle
                // of a party is worse than one extra press.
                Input::Back | Input::Random => {
                    self.overlay = Overlay::Paused;
                    self.pause_cursor = 0;
                    (Transition::None, None)
                }
                _ => (Transition::None, None),
            },
            Overlay::Paused => match input {
                Input::Up => {
                    self.pause_cursor =
                        (self.pause_cursor + PAUSE_ENTRIES.len() - 1) % PAUSE_ENTRIES.len();
                    (Transition::None, None)
                }
                Input::Down => {
                    self.pause_cursor = (self.pause_cursor + 1) % PAUSE_ENTRIES.len();
                    (Transition::None, None)
                }
                Input::Confirm => {
                    let choice = match self.pause_cursor {
                        0 => PauseChoice::Continue,
                        1 => PauseChoice::Restart,
                        _ => PauseChoice::Quit,
                    };
                    if choice == PauseChoice::Continue {
                        self.overlay = Overlay::None;
                    }
                    (Transition::None, Some(choice))
                }
                Input::Back => {
                    self.overlay = Overlay::None;
                    (Transition::None, Some(PauseChoice::Continue))
                }
                _ => (Transition::None, None),
            },
            Overlay::Results => match input {
                Input::Confirm | Input::Back => (Transition::Pop, None),
                _ => (Transition::None, None),
            },
        }
    }

    /// Draw a frame.
    ///
    /// `notes` and `syllables` describe the track being sung; `beat` is the drawing clock,
    /// which runs ahead of the scoring clock by the microphone delay.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        notes: &[Note],
        syllables: &[Syllable],
        next_line: &str,
        beat: f64,
    ) {
        if let Some(image) = self.background {
            // Artwork behind, heavily dimmed: the lyrics have to stay readable over any
            // picture, including a white one.
            list.image_tinted(area, image, Color::WHITE.alpha(0.35), 0.0);
            list.fill(area, style.background.alpha(0.55));
        }

        let (header, body) = area.cut_top(style.gap(4.0));
        self.draw_header(list, header, style);

        // Singers take a strip down the side, so the staff keeps the middle whatever the
        // player count. Six panels on a Deck are still readable at this width.
        let panel_w = (area.w * 0.18).clamp(220.0, 340.0);
        let (panels, middle) = body.cut_right(panel_w);
        self.draw_panels(list, panels.inset(style.gap(1.0)), style);

        let (lyrics_area, staff_area) = middle.cut_bottom(style.gap(9.0));
        self.draw_staff(list, staff_area.inset(style.gap(1.5)), style, notes, beat);
        self.draw_lyrics(list, lyrics_area, style, syllables, next_line, beat);

        match self.overlay {
            Overlay::Paused => self.draw_pause(list, area, style),
            Overlay::Results => self.draw_results(list, area, style),
            Overlay::None => {}
        }
    }

    fn draw_header(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let inner = area.inset_xy(style.gap(2.0), 0.0);
        let (title_area, _) = inner.cut_left(inner.w * 0.6);
        list.text(
            title_area,
            format!("{} \u{2013} {}", self.artist, self.title),
            TextStyle::new(style.scaled_text(0.95), style.text)
                .bold()
                .overflow(Overflow::Ellipsis),
        );

        // A progress bar rather than a clock: how much is left matters, the timestamp does not.
        if self.duration > 0.0 {
            let track = Rect::new(
                inner.x,
                area.bottom() - style.gap(0.6),
                inner.w,
                style.gap(0.35),
            );
            list.panel(track, style.surface_sunken, track.h / 2.0);
            let done = (self.position / self.duration).clamp(0.0, 1.0);
            list.panel(
                Rect::new(track.x, track.y, track.w * done, track.h),
                style.accent,
                track.h / 2.0,
            );
        }
    }

    fn draw_panels(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let rows = area.rows(self.singers.len(), style.gap(1.0));
        for (index, (singer, row)) in self.singers.iter().zip(rows).enumerate() {
            let color = style.player(index);
            list.panel(row, style.surface.alpha(0.9), style.metrics.radius);
            let inner = row.inset(style.gap(1.0));

            let (name_row, rest) = inner.cut_top(style.scaled_text(0.85) * 1.4);
            list.text(
                name_row,
                &singer.name,
                TextStyle::new(style.scaled_text(0.85), color)
                    .bold()
                    .overflow(Overflow::Ellipsis),
            );

            let (score_row, rest) = rest.cut_top(style.scaled_text(1.8) * 1.2);
            list.text(
                score_row,
                singer.score.to_string(),
                TextStyle::new(style.scaled_text(1.8), style.text)
                    .bold()
                    .align(Align::End),
            );

            // Score bar, so a glance says who is ahead without reading four numbers.
            let (bar_row, rest) = rest.cut_top(style.gap(0.8));
            let track = bar_row.anchored(Anchor::Center, bar_row.w, style.gap(0.35), 0.0);
            list.panel(track, style.surface_sunken, track.h / 2.0);
            list.panel(
                Rect::new(
                    track.x,
                    track.y,
                    track.w * singer.fraction.clamp(0.0, 1.0),
                    track.h,
                ),
                color,
                track.h / 2.0,
            );

            // A fading rating from the last line bonus.
            if let Some((rating, age)) = singer.rating {
                if age < RATING_LIFETIME {
                    let fade = 1.0 - age / RATING_LIFETIME;
                    let (rating_row, _) = rest.cut_top(style.scaled_text(0.9) * 1.3);
                    list.text(
                        rating_row,
                        rating_words(rating),
                        TextStyle::new(style.scaled_text(0.9), color.alpha(fade))
                            .bold()
                            .align(Align::End),
                    );
                }
            }

            if self.show_input_panel || singer.input_problem().is_some() {
                let strip = Rect::new(
                    inner.x,
                    inner.bottom() - style.gap(2.4),
                    inner.w,
                    style.gap(2.4),
                );
                self.draw_input_strip(list, strip, style, singer, color);
            }
        }
    }

    /// The strip that answers "is my microphone working, and am I on the note".
    ///
    /// Three questions with three readouts, because they fail independently and the first
    /// version of this screen made a dead microphone and a silent room look identical.
    fn draw_input_strip(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        singer: &Singer,
        color: Color,
    ) {
        let (meter, label) = area.cut_top(style.gap(0.7));
        let track = meter.anchored(Anchor::Center, meter.w, style.gap(0.4), 0.0);
        list.panel(track, style.surface_sunken, track.h / 2.0);
        let level = singer.level.clamp(0.0, 1.0);
        if level > 0.0 {
            list.panel(
                Rect::new(track.x, track.y, track.w * level, track.h),
                if level >= singer.gate {
                    color
                } else {
                    style.muted
                },
                track.h / 2.0,
            );
        }
        // A mark at the gate, so "too quiet" is visible rather than inferred.
        let gate_x = track.x + track.w * singer.gate.clamp(0.0, 1.0);
        list.fill(
            Rect::new(gate_x - 1.0, track.y - 2.0, 2.0, track.h + 4.0),
            style.warning,
        );

        let (text, tint) = match singer.input_problem() {
            Some(problem) => (problem.to_owned(), style.warning),
            None => match (singer.pitch, singer.hitting) {
                (Some(pitch), Some(true)) => {
                    (format!("{} \u{2713}", note_name(pitch)), style.success)
                }
                (Some(pitch), Some(false)) => {
                    (format!("{} \u{2717}", note_name(pitch)), style.danger)
                }
                (Some(pitch), None) => (note_name(pitch).to_owned(), style.muted),
                (None, _) => ("listening".to_owned(), style.muted),
            },
        };
        list.text(
            label,
            text,
            TextStyle::new(style.scaled_text(0.7), tint)
                .align(Align::End)
                .overflow(Overflow::Ellipsis),
        );
    }

    fn draw_staff(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        notes: &[Note],
        beat: f64,
    ) {
        list.panel(area, style.surface.alpha(0.55), style.metrics.radius);
        if notes.is_empty() {
            return;
        }

        // A window that scrolls with the song rather than one line at a time, so a note's
        // approach is visible and its length is comparable to its neighbours'.
        let window = 16.0;
        let start = beat - window * 0.25;
        let end = beat + window * 0.75;

        let visible: Vec<&Note> = notes
            .iter()
            .filter(|n| n.end() > start && n.start < end)
            .collect();
        if visible.is_empty() {
            return;
        }
        let (lowest, highest) = visible.iter().fold((i32::MAX, i32::MIN), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        let lowest = lowest - STAFF_MARGIN;
        let highest = highest + STAFF_MARGIN;
        let span = (highest - lowest).max(1) as f32;

        let inner = area.inset(style.gap(1.0));
        let row_h = inner.h / span;
        let x_of = |b: f64| inner.x + ((b - start) / (end - start)) as f32 * inner.w;
        let y_of = |pitch: i32| inner.y + (highest - pitch) as f32 * row_h;

        // Faint lines behind the notes, one per semitone.
        for semitone in lowest..=highest {
            let y = y_of(semitone) + row_h / 2.0;
            list.fill(
                Rect::new(inner.x, y - 0.5, inner.w, 1.0),
                style.muted.alpha(0.10),
            );
        }

        // The playhead, fixed where the notes should be met.
        let head = x_of(beat);
        list.fill(
            Rect::new(head - 1.5, inner.y, 3.0, inner.h),
            style.accent.alpha(0.8),
        );

        // Clipped: a note that started before the window still has to be drawn, and without
        // this it is drawn from off the left edge across whatever is beside the staff.
        let note_h = (row_h * 0.7).max(4.0);
        list.clipped(inner, |list| {
            for note in &visible {
                let x = x_of(note.start);
                let w = (x_of(note.end()) - x).max(3.0);
                let y = y_of(note.pitch) + (row_h - note_h) / 2.0;
                let rect = Rect::new(x, y, w, note_h);
                let color = if note.kind.is_golden() {
                    style.warning
                } else {
                    style.muted.alpha(0.75)
                };
                if note.kind.is_freestyle() {
                    // Freestyle scores nothing, so it is drawn as an outline: it is a cue, not a
                    // target, and filling it in would promise points that never arrive.
                    list.outline(rect, color.alpha(0.4), 2.0, note_h / 2.0);
                } else {
                    list.panel(rect, color, note_h / 2.0);
                }
            }

            // What each singer actually sang, over the top.
            for (index, singer) in self.singers.iter().enumerate() {
                let color = style.player(index);
                for sung in &singer.sung {
                    if sung.start + sung.duration < start || sung.start > end {
                        continue;
                    }
                    let x = x_of(sung.start);
                    let w = (x_of(sung.start + sung.duration) - x).max(3.0);
                    let y = y_of(sung.pitch) + (row_h - note_h) / 2.0;
                    let rect = Rect::new(x, y + note_h * 0.25, w, note_h * 0.5);
                    list.panel(
                        rect,
                        if sung.hit { color } else { style.danger },
                        rect.h / 2.0,
                    );
                }
                // Where this singer is right now, whether or not a note is due — silence and a
                // dead microphone looked identical without it.
                if let Some(pitch) = singer.pitch {
                    let y = y_of(pitch.clamp(lowest, highest)) + row_h / 2.0;
                    list.fill(Rect::new(head - 14.0, y - 2.0, 28.0, 4.0), color);
                }
            }
        });
    }

    fn draw_lyrics(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        syllables: &[Syllable],
        next_line: &str,
        beat: f64,
    ) {
        let (current, upcoming) = area.cut_top(area.h * 0.62);
        if syllables.is_empty() {
            return;
        }

        // Measured by character count rather than by the font, because the screen has no
        // font. Slight over-estimate, so the line is never wider than the room it was given.
        //
        // A long line has to shrink to fit: centring one wider than the screen puts its first
        // words off the left edge, which is where a lyric is least recoverable.
        let mut size = style.scaled_text(1.5);
        let width_at = |size: f32| -> f32 {
            syllables
                .iter()
                .map(|s| crate::draw::approx_text_width(&s.text, size))
                .sum()
        };
        let available = current.w - style.gap(2.0);
        let total = width_at(size);
        if total > available && total > 0.0 {
            // Down to half size; past that the line is unreadable anyway and clipping the
            // ends is the better failure.
            size *= (available / total).max(0.5);
        }
        let total = width_at(size);
        let mut x = (current.center().x - total / 2.0).max(current.x);

        for syllable in syllables {
            let width = crate::draw::approx_text_width(&syllable.text, size);
            let sung = beat >= syllable.start + syllable.duration;
            let active = beat >= syllable.start && !sung;
            let color = if active {
                style.accent
            } else if sung {
                style.text
            } else if syllable.golden {
                style.warning
            } else {
                style.muted
            };
            let box_rect = Rect::new(x, current.y, width, current.h);
            list.text(
                box_rect,
                &syllable.text,
                TextStyle::new(size, color)
                    .font(Font::Lyrics)
                    .centered()
                    .valign(VAlign::Middle)
                    // Lyrics sit over artwork and video, where an outline is the difference
                    // between readable and not.
                    .outlined(style.background.alpha(0.85), 2.0),
            );
            x += width;
        }

        if !next_line.is_empty() {
            list.text(
                upcoming,
                next_line,
                TextStyle::new(style.scaled_text(1.0), style.muted)
                    .font(Font::Lyrics)
                    .centered()
                    .overflow(Overflow::Ellipsis),
            );
        }
    }

    fn draw_pause(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let row_h = style.gap(4.0);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.32).min(560.0),
            row_h * (PAUSE_ENTRIES.len() as f32 + 2.0),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(1.5));
        list.text(
            Rect::new(inner.x, inner.y, inner.w, row_h),
            "Paused",
            TextStyle::new(style.scaled_text(1.4), style.text)
                .bold()
                .centered(),
        );
        for (index, entry) in PAUSE_ENTRIES.iter().enumerate() {
            let row = Rect::new(
                inner.x,
                inner.y + row_h * (index as f32 + 1.2),
                inner.w,
                row_h,
            )
            .inset_xy(0.0, style.gap(0.3));
            widgets.row(list, row, entry, "", index == self.pause_cursor);
        }
    }

    fn draw_results(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let row_h = style.gap(5.0);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.5).min(900.0),
            row_h * (self.singers.len() as f32 + 3.0),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(2.0));

        let (title, rest) = inner.cut_top(row_h);
        list.text(
            title,
            format!("{} \u{2013} {}", self.artist, self.title),
            TextStyle::new(style.scaled_text(1.3), style.text)
                .bold()
                .centered()
                .overflow(Overflow::Ellipsis),
        );

        // Ranked, because in a party the order is the whole point.
        let mut ranked: Vec<&Singer> = self.singers.iter().collect();
        ranked.sort_by_key(|s| std::cmp::Reverse(s.score));

        for (place, singer) in ranked.iter().enumerate() {
            let row = Rect::new(rest.x, rest.y + row_h * place as f32, rest.w, row_h)
                .inset_xy(0.0, style.gap(0.4));
            let original = self
                .singers
                .iter()
                .position(|s| std::ptr::eq(s, *singer))
                .unwrap_or(place);
            let color = style.player(original);
            list.panel(row, style.surface_raised, style.metrics.radius);
            let cell = row.inset_xy(style.gap(1.5), 0.0);
            list.text(
                cell,
                &singer.name,
                TextStyle::new(style.text_size(), color)
                    .bold()
                    .overflow(Overflow::Ellipsis),
            );
            list.text(
                cell,
                format!("{}   {}", rating_title(singer.score), singer.score),
                TextStyle::new(style.text_size(), style.text).align(Align::End),
            );
        }

        let hint = if self.gamepad { "A" } else { "Enter" };
        list.text(
            Rect::new(
                inner.x,
                card.bottom() - style.gap(3.0),
                inner.w,
                style.gap(2.0),
            ),
            format!("{hint} to continue"),
            TextStyle::new(style.scaled_text(0.85), style.muted).centered(),
        );
    }
}

/// UltraStar's rating tiers on the 0..=10000 total.
pub fn rating_title(score: i32) -> &'static str {
    match score {
        s if s < 2010 => "Tone Deaf",
        s if s < 4010 => "Amateur",
        s if s < 5010 => "Wannabe",
        s if s < 6010 => "Hopeful",
        s if s < 7510 => "Rising Star",
        s if s < 8510 => "Lead Singer",
        s if s < 9010 => "Superstar",
        _ => "Ultrastar",
    }
}

/// The words for a line-bonus rating, `0..=8`.
fn rating_words(rating: i32) -> &'static str {
    match rating.clamp(0, 8) {
        0 | 1 => "Awful",
        2 | 3 => "Poor",
        4 => "Bad",
        5 => "Not bad",
        6 => "Good",
        7 => "Great",
        _ => "Perfect",
    }
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// The name of a pitch class. Matching is octave-agnostic, so only the class is meaningful.
pub fn note_name(pitch: i32) -> &'static str {
    NOTE_NAMES[pitch.rem_euclid(12) as usize]
}

/// Where the pointer is, for a screen that mostly ignores it.
impl SingScreen {
    pub fn hit(&self, _area: Rect, _point: Point) -> Option<usize> {
        None
    }
}
