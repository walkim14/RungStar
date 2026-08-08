//! The song editor.
//!
//! A piano roll with the waveform behind it. The notes are drawn where they are in time and
//! pitch, the audio is drawn underneath at the same scale, and the cursor is a note rather
//! than a pixel — because everything you do to a song is to a note, and a mouse-pixel editor
//! on a controller is unusable.
//!
//! **Nothing here changes the song.** Every key turns into an [`Op`] and the editor crate
//! decides whether it is allowed. That is what makes the rules — no note on top of another, no
//! note shorter than a beat — testable without a window.

use rungstar_editor::ops::{Op, Which};
use rungstar_editor::song::NoteKind;
use rungstar_editor::{Editor, Selection, Waveform};

use crate::draw::{Align, DrawList, Overflow, TextStyle, VAlign};
use crate::geom::{Anchor, Point, Rect};
use crate::keyboard::{Key, Keyboard};
use crate::screen::{Transition, Widgets};
use crate::songselect::Input;
use crate::theme::Style;

/// What the screen is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Notes,
    /// Typing the syllable of the note under the cursor.
    Typing,
    /// The menu of things that are not a keystroke: tempo, gap, medley, save.
    Menu,
    /// Confirming a way out that would lose work.
    Leaving,
}

/// What the screen wants the application to do.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorOutcome {
    None,
    /// Write the song back.
    Save,
    /// Play from this beat, so the edit can be heard.
    Play(f64),
    Stop,
    /// Leave, whether or not it was saved.
    Leave,
}

/// One row of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Save,
    GapEarlier,
    GapLater,
    DoubleTempo,
    HalveTempo,
    MedleyStart,
    MedleyEnd,
    ClearMedley,
    Preview,
    Track,
    Leave,
}

impl Item {
    const ALL: [Item; 11] = [
        Item::Save,
        Item::GapEarlier,
        Item::GapLater,
        Item::DoubleTempo,
        Item::HalveTempo,
        Item::MedleyStart,
        Item::MedleyEnd,
        Item::ClearMedley,
        Item::Preview,
        Item::Track,
        Item::Leave,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Save => "Save",
            Self::GapEarlier => "Lyrics 10 ms earlier",
            Self::GapLater => "Lyrics 10 ms later",
            Self::DoubleTempo => "Double the tempo",
            Self::HalveTempo => "Halve the tempo",
            Self::MedleyStart => "Chorus starts here",
            Self::MedleyEnd => "Chorus ends here",
            Self::ClearMedley => "Forget the chorus",
            Self::Preview => "Preview starts here",
            Self::Track => "Other part of the duet",
            Self::Leave => "Close the editor",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Save => "Write the song back to its file.",
            Self::GapEarlier | Self::GapLater => {
                "Shift every word against the recording. Use this when the whole song is out, \
                 not when one note is."
            }
            Self::DoubleTempo | Self::HalveTempo => {
                "Every timestamp moves with it, so the song still lines up with the audio. \
                 Doubling is the fix for a file whose beats are all even numbers."
            }
            Self::MedleyStart | Self::MedleyEnd | Self::ClearMedley => {
                "Where \"sing from the chorus\" starts and stops."
            }
            Self::Preview => "The moment the browser plays when this song is under the cursor.",
            Self::Track => "A duet has two parts. This edits the other one.",
            Self::Leave => "Anything unsaved is offered back before it goes.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Note(usize, usize),
    Item(usize),
    Key(usize),
    Confirm(bool),
}

/// The editor screen.
pub struct EditorScreen {
    /// The song being edited. The screen owns it, because a screen that borrows the document
    /// cannot be on a stack with anything else.
    pub editor: Editor,
    /// The audio behind the notes, when it has been decoded.
    pub waveform: Waveform,
    /// Where playback is, in seconds, or `None` when it is stopped.
    pub playing: Option<f64>,
    /// How many beats fit across the staff. Smaller is closer in.
    pub zoom: f64,
    pub gamepad: bool,
    mode: Mode,
    keyboard: Keyboard,
    menu: usize,
    confirm_leave: bool,
    /// The left edge of the view, in beats.
    scroll: f64,
    regions: Vec<(Rect, Region)>,
}

/// The narrowest and widest the staff may be, in beats.
pub const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 8.0..=256.0;

impl EditorScreen {
    pub fn new(editor: Editor) -> Self {
        Self {
            editor,
            waveform: Waveform::default(),
            playing: None,
            zoom: 64.0,
            gamepad: false,
            mode: Mode::Notes,
            keyboard: Keyboard::new(),
            menu: 0,
            confirm_leave: false,
            scroll: 0.0,
            regions: Vec::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn wants_text(&self) -> bool {
        self.mode == Mode::Typing
    }

    pub fn handle(&mut self, input: Input) -> (Transition, EditorOutcome) {
        if let Input::Hover(point) | Input::Click(point) = input {
            return self.handle_pointer(point, matches!(input, Input::Click(_)));
        }
        match self.mode {
            Mode::Notes => self.handle_notes(input),
            Mode::Typing => self.handle_typing(input),
            Mode::Menu => self.handle_menu(input),
            Mode::Leaving => self.handle_leaving(input),
        }
    }

    fn handle_notes(&mut self, input: Input) -> (Transition, EditorOutcome) {
        let outcome = match input {
            // Left and right walk the notes; up and down are pitch, because that is what the
            // picture says they are.
            Input::Left => {
                self.editor.apply(Op::Step(-1));
                EditorOutcome::None
            }
            Input::Right => {
                self.editor.apply(Op::Step(1));
                EditorOutcome::None
            }
            Input::Up => {
                self.editor.apply(Op::Transpose(1));
                EditorOutcome::None
            }
            Input::Down => {
                self.editor.apply(Op::Transpose(-1));
                EditorOutcome::None
            }
            Input::PageUp => {
                self.editor.apply(Op::StepLine(-1));
                EditorOutcome::None
            }
            Input::PageDown => {
                self.editor.apply(Op::StepLine(1));
                EditorOutcome::None
            }
            // Confirm plays from the cursor. Hearing the edit is the whole loop of timing a
            // song, and it should be the easiest thing on the screen to do.
            Input::Confirm | Input::Submit => {
                if self.playing.is_some() {
                    self.playing = None;
                    EditorOutcome::Stop
                } else {
                    let at = self.cursor_seconds();
                    EditorOutcome::Play(at)
                }
            }
            Input::Search => {
                self.keyboard = Keyboard::with_text(
                    self.editor
                        .current()
                        .map(|n| n.text.clone())
                        .unwrap_or_default(),
                )
                .limit(64);
                self.mode = Mode::Typing;
                EditorOutcome::None
            }
            Input::ContextMenu => {
                self.mode = Mode::Menu;
                EditorOutcome::None
            }
            Input::Back => {
                if self.editor.dirty() {
                    self.mode = Mode::Leaving;
                    self.confirm_leave = false;
                    return (Transition::None, EditorOutcome::None);
                }
                return (Transition::Pop, EditorOutcome::Leave);
            }
            Input::Type(c) => return (Transition::None, self.shortcut(c)),
            _ => EditorOutcome::None,
        };
        self.follow();
        (Transition::None, outcome)
    }

    /// The single-letter shortcuts, which are what makes timing a song fast.
    ///
    /// A letter rather than a chord: this screen is used with one hand on the keyboard and one
    /// on the space bar, and every modifier is a reason to look down.
    fn shortcut(&mut self, c: char) -> EditorOutcome {
        match c.to_ascii_lowercase() {
            // The four that get used constantly, on the home row.
            'j' => self.editor.apply(Op::Move(-1)),
            'l' => self.editor.apply(Op::Move(1)),
            'i' => self.editor.apply(Op::Resize(1)),
            'k' => self.editor.apply(Op::Resize(-1)),
            's' => self.editor.apply(Op::Split(
                self.editor.current().map_or(1, |note| note.duration / 2),
            )),
            'm' => self.editor.apply(Op::Merge),
            'n' => self.editor.apply(Op::Insert),
            'x' => self.editor.apply(Op::Delete),
            'b' => self.editor.apply(Op::BreakLine),
            'v' => self.editor.apply(Op::JoinLine),
            'g' => self.editor.apply(Op::SetKind(NoteKind::Golden)),
            'r' => self.editor.apply(Op::SetKind(NoteKind::Regular)),
            'f' => self.editor.apply(Op::SetKind(NoteKind::Freestyle)),
            'e' => self.editor.apply(Op::Extend(1)),
            'w' => self.editor.apply(Op::Extend(-1)),
            'z' => self.editor.undo(),
            'y' => self.editor.redo(),
            '+' | '=' => {
                self.zoom = (self.zoom / 2.0).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
                true
            }
            '-' => {
                self.zoom = (self.zoom * 2.0).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
                true
            }
            _ => false,
        };
        self.follow();
        EditorOutcome::None
    }

    fn handle_typing(&mut self, input: Input) -> (Transition, EditorOutcome) {
        match input {
            Input::Back => self.mode = Mode::Notes,
            Input::Type(c) => self.keyboard.push(c),
            Input::Backspace => {
                self.keyboard.apply(Key::Backspace);
            }
            Input::Up => self.keyboard.navigate(0, -1),
            Input::Down => self.keyboard.navigate(0, 1),
            Input::Left => self.keyboard.navigate(-1, 0),
            Input::Right => self.keyboard.navigate(1, 0),
            Input::Submit => {
                self.editor
                    .apply(Op::SetText(self.keyboard.text().to_owned()));
                self.mode = Mode::Notes;
            }
            Input::Confirm => {
                if self.keyboard.press() {
                    self.editor
                        .apply(Op::SetText(self.keyboard.text().to_owned()));
                    self.mode = Mode::Notes;
                    return (Transition::None, EditorOutcome::None);
                }
            }
            _ => return (Transition::None, EditorOutcome::None),
        }
        // Live, so the syllable under the cursor changes as it is typed rather than at the
        // end — the point of the picture is seeing the word land on the note.
        if self.mode == Mode::Typing {
            self.editor
                .apply(Op::SetText(self.keyboard.text().to_owned()));
        }
        (Transition::None, EditorOutcome::None)
    }

    fn handle_menu(&mut self, input: Input) -> (Transition, EditorOutcome) {
        let count = Item::ALL.len();
        match input {
            Input::Up => self.menu = (self.menu + count - 1) % count,
            Input::Down => self.menu = (self.menu + 1) % count,
            Input::Back | Input::ContextMenu => self.mode = Mode::Notes,
            Input::Confirm | Input::Submit => {
                let item = Item::ALL[self.menu.min(count - 1)];
                self.mode = Mode::Notes;
                return (Transition::None, self.run(item));
            }
            _ => {}
        }
        (Transition::None, EditorOutcome::None)
    }

    fn run(&mut self, item: Item) -> EditorOutcome {
        match item {
            Item::Save => return EditorOutcome::Save,
            Item::GapEarlier => self.editor.apply(Op::Gap(-10)),
            Item::GapLater => self.editor.apply(Op::Gap(10)),
            Item::DoubleTempo => self.editor.apply(Op::ScaleBpm(2.0)),
            Item::HalveTempo => self.editor.apply(Op::ScaleBpm(0.5)),
            Item::MedleyStart => self.editor.apply(Op::Medley(Which::Start)),
            Item::MedleyEnd => self.editor.apply(Op::Medley(Which::End)),
            Item::ClearMedley => self.editor.apply(Op::ClearMedley),
            Item::Preview => self.editor.apply(Op::Preview),
            Item::Track => {
                let next = (self.editor.selection.track + 1) % self.editor.tracks().max(1);
                self.editor.apply(Op::Track(next))
            }
            Item::Leave => {
                if self.editor.dirty() {
                    self.mode = Mode::Leaving;
                    self.confirm_leave = false;
                    return EditorOutcome::None;
                }
                return EditorOutcome::Leave;
            }
        };
        self.follow();
        EditorOutcome::None
    }

    fn handle_leaving(&mut self, input: Input) -> (Transition, EditorOutcome) {
        match input {
            Input::Left | Input::Right | Input::Up | Input::Down => {
                self.confirm_leave = !self.confirm_leave
            }
            Input::Back | Input::ContextMenu => self.mode = Mode::Notes,
            Input::Confirm | Input::Submit => {
                self.mode = Mode::Notes;
                if self.confirm_leave {
                    return (Transition::Pop, EditorOutcome::Leave);
                }
                // The other answer saves rather than cancelling: somebody who pressed Escape
                // with unsaved work almost always meant to keep it.
                return (Transition::None, EditorOutcome::Save);
            }
            _ => {}
        }
        (Transition::None, EditorOutcome::None)
    }

    fn handle_pointer(&mut self, point: Point, clicked: bool) -> (Transition, EditorOutcome) {
        let hit = self
            .regions
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, region)| *region);
        match hit {
            Some(Region::Note(line, note)) => {
                if clicked {
                    self.editor.apply(Op::Put(Selection {
                        track: self.editor.selection.track,
                        line,
                        note,
                        run: 1,
                    }));
                }
            }
            Some(Region::Item(index)) => {
                self.menu = index;
                if clicked {
                    return self.handle_menu(Input::Confirm);
                }
            }
            Some(Region::Key(index)) => {
                self.keyboard.set_cursor(index);
                if clicked {
                    return self.handle(Input::Confirm);
                }
            }
            Some(Region::Confirm(leave)) => {
                self.confirm_leave = leave;
                if clicked {
                    return self.handle_leaving(Input::Confirm);
                }
            }
            None => {}
        }
        (Transition::None, EditorOutcome::None)
    }

    /// Where the cursor is, in seconds, for playing from there.
    pub fn cursor_seconds(&self) -> f64 {
        let beat = self
            .editor
            .current()
            .map_or(0.0, |note| f64::from(note.start));
        // A moment of run-up, or the first word is already going when the sound starts.
        (self.editor.seconds_at(beat) - 0.5).max(0.0)
    }

    /// Keep the cursor on screen.
    fn follow(&mut self) {
        let Some(note) = self.editor.current() else {
            return;
        };
        let start = f64::from(note.start);
        let end = f64::from(note.start + note.duration);
        // A margin either side, so the cursor is never against the edge with no context.
        let margin = self.zoom * 0.15;
        if start - margin < self.scroll {
            self.scroll = (start - margin).max(0.0);
        }
        if end + margin > self.scroll + self.zoom {
            self.scroll = end + margin - self.zoom;
        }
    }

    pub fn draw(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.regions.clear();
        let widgets = Widgets::new(style);
        let title = format!(
            "{} \u{2013} {}",
            self.editor.song().headers.artist,
            self.editor.song().headers.title
        );
        let mut status = format!("{} BPM", self.editor.song().bpm().value());
        if self.editor.tracks() > 1 {
            status = format!("part {}  \u{b7}  {status}", self.editor.selection.track + 1);
        }
        if self.editor.dirty() {
            status.push_str("  \u{b7}  unsaved");
        }
        let body = widgets.header(list, area, &title, &status);
        let body = widgets.footer(list, body, &self.hints());

        // What was refused, if anything: a rule that stops an edit without saying why reads as
        // a broken key.
        let (message, body) = body.cut_bottom(style.gap(2.4));
        if let Some(refused) = &self.editor.refused {
            list.text(
                message.inset_xy(style.gap(2.0), 0.0),
                refused,
                TextStyle::new(style.scaled_text(0.85), style.danger),
            );
        }

        let inner = body.inset(style.gap(1.5));
        // `cut_bottom` hands back the strip first and what is left second. The waveform is
        // the strip along the bottom; the roll is everything above it.
        let (wave, roll) = inner.cut_bottom(inner.h * 0.22);
        self.draw_waveform(list, wave, style);
        self.draw_roll(list, roll, style);

        match self.mode {
            Mode::Notes => {}
            Mode::Typing => {
                let mut overlay = Vec::new();
                self.draw_typing(list, area, style, &mut overlay);
                self.regions.extend(overlay);
            }
            Mode::Menu => {
                let mut overlay = Vec::new();
                self.draw_menu(list, area, style, &mut overlay);
                self.regions.extend(overlay);
            }
            Mode::Leaving => {
                let mut overlay = Vec::new();
                self.draw_leaving(list, area, style, &mut overlay);
                self.regions.extend(overlay);
            }
        }
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        let pad = self.gamepad;
        let confirm = if pad { "A" } else { "Enter" };
        let back = if pad { "B" } else { "Esc" };
        match self.mode {
            Mode::Notes => vec![
                (if pad { "LS" } else { "\u{2190}\u{2192}" }, "Note"),
                (if pad { "RS" } else { "\u{2191}\u{2193}" }, "Pitch"),
                ("J L", "Move"),
                ("I K", "Length"),
                (if pad { "X" } else { "F" }, "Words"),
                (confirm, "Play"),
                (if pad { "Y" } else { "M" }, "More"),
                (back, "Close"),
            ],
            Mode::Typing => vec![(confirm, "Press key"), (back, "Done")],
            Mode::Menu => vec![(confirm, "Choose"), (back, "Back")],
            Mode::Leaving => vec![(confirm, "Choose"), (back, "Keep editing")],
        }
    }

    /// The notes, as a piano roll.
    fn draw_roll(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        list.panel(area, style.surface.alpha(0.5), style.metrics.radius);
        let lines = self.editor.lines();
        if lines.is_empty() {
            list.text(
                area,
                "This song has no notes in it",
                TextStyle::new(style.text_size(), style.muted).centered(),
            );
            return;
        }

        // The pitch scale is the whole song's, as on the sing screen and for the same reason:
        // a scale that follows the view makes a note change height as you scroll, and then you
        // cannot tell whether it is higher than the one before it.
        let (low, high) = lines
            .iter()
            .flat_map(|line| line.notes.iter())
            .fold((i32::MAX, i32::MIN), |(lo, hi), note| {
                (lo.min(note.pitch), hi.max(note.pitch))
            });
        let (low, high) = if low > high {
            (0, 12)
        } else {
            (low - 1, high + 1)
        };
        let rows = (high - low + 1).max(1) as f32;

        let inner = area.inset(style.gap(1.0));
        let row_h = inner.h / rows;
        let x_of = |beat: f64| inner.x + ((beat - self.scroll) / self.zoom) as f32 * inner.w;
        let y_of = |pitch: i32| inner.y + (high - pitch.clamp(low, high)) as f32 * row_h;

        let selection = self.editor.selection.clone();
        let note_h = (row_h * 0.8).clamp(4.0, 22.0);
        let mut regions = Vec::new();
        list.clipped(inner, |list| {
            for (line_index, line) in lines.iter().enumerate() {
                for (note_index, note) in line.notes.iter().enumerate() {
                    let x = x_of(f64::from(note.start));
                    let w = (x_of(f64::from(note.start + note.duration)) - x).max(3.0);
                    if x > inner.right() || x + w < inner.x {
                        continue;
                    }
                    let rect = Rect::new(x, y_of(note.pitch) + (row_h - note_h) / 2.0, w, note_h);
                    regions.push((rect, Region::Note(line_index, note_index)));

                    let selected =
                        line_index == selection.line && selection.notes().contains(&note_index);
                    let colour = match note.kind {
                        NoteKind::Golden | NoteKind::GoldenRap => style.warning,
                        NoteKind::Freestyle => style.muted,
                        _ => style.accent,
                    };
                    if note.kind == NoteKind::Freestyle {
                        list.outline(rect, colour.alpha(0.6), 2.0, note_h / 2.0);
                    } else {
                        list.panel(
                            rect,
                            colour.alpha(if selected { 1.0 } else { 0.55 }),
                            note_h / 2.0,
                        );
                    }
                    if selected {
                        list.outline(rect.inset(-2.0), style.text, 2.0, note_h / 2.0 + 2.0);
                    }
                    // The syllable, when there is room for it. Below the note rather than in
                    // it, because a note one beat long has no room inside.
                    if !note.text.trim().is_empty() && w > style.gap(2.0) {
                        list.text(
                            Rect::new(rect.x, rect.bottom(), rect.w.max(style.gap(6.0)), row_h),
                            &note.text,
                            TextStyle::new(style.scaled_text(0.7), style.text)
                                .overflow(Overflow::Ellipsis),
                        );
                    }
                }
                // The line break, so the phrases are visible as phrases.
                if let Some(brk) = line.line_break {
                    let x = x_of(f64::from(brk.previous_line_out_time));
                    list.fill(Rect::new(x, inner.y, 1.0, inner.h), style.muted.alpha(0.35));
                }
            }

            // Where playback is.
            if let Some(seconds) = self.playing {
                let beat = self.editor.beat_at(seconds);
                let x = x_of(beat);
                if x >= inner.x && x <= inner.right() {
                    list.fill(Rect::new(x - 1.0, inner.y, 2.0, inner.h), style.success);
                }
            }
        });
        self.regions.extend(regions);
    }

    /// The audio, at the same horizontal scale as the notes.
    fn draw_waveform(&self, list: &mut DrawList, area: Rect, style: &Style) {
        list.panel(area, style.surface_sunken.alpha(0.7), style.metrics.radius);
        let inner = area.inset(style.gap(0.6));
        if self.waveform.is_empty() {
            list.text(
                inner,
                "no audio to draw",
                TextStyle::new(style.scaled_text(0.75), style.muted)
                    .centered()
                    .valign(VAlign::Middle),
            );
            return;
        }
        let from = self.editor.seconds_at(self.scroll);
        let to = self.editor.seconds_at(self.scroll + self.zoom);
        // One column per two design units: finer than that is more columns than pixels on any
        // display this runs on, and each one is a draw command.
        let columns = ((inner.w / 2.0) as usize).clamp(1, 1200);
        let peaks = self.waveform.columns(from, to, columns);
        let width = inner.w / columns.max(1) as f32;
        let middle = inner.center().y;
        for (index, peak) in peaks.iter().enumerate() {
            let height = (inner.h * peak).max(1.0);
            list.fill(
                Rect::new(
                    inner.x + width * index as f32,
                    middle - height / 2.0,
                    width.max(1.0),
                    height,
                ),
                style.accent.alpha(0.5),
            );
        }
    }

    fn draw_typing(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.7).min(1000.0),
            (area.h * 0.6).min(600.0),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(2.0));
        let (heading, rest) = inner.cut_top(style.gap(3.2));
        list.text(
            heading,
            "Syllable",
            TextStyle::new(style.scaled_text(1.2), style.text).bold(),
        );
        let (field, keys) = rest.cut_top(style.gap(4.0));
        list.panel(
            field.inset_xy(0.0, style.gap(0.4)),
            style.surface_sunken,
            style.metrics.radius,
        );
        list.text(
            field.inset_xy(style.gap(1.4), 0.0),
            format!("{}\u{2502}", self.keyboard.text()),
            TextStyle::new(style.text_size(), style.text).overflow(Overflow::Ellipsis),
        );
        self.draw_keys(list, keys, style, regions);
    }

    fn draw_keys(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let keys = self.keyboard.keys();
        let rows = self.keyboard.rows().max(1);
        let size = (area.w / crate::keyboard::COLUMNS as f32).min(area.h / rows as f32);
        let origin_x = area.center().x - size * crate::keyboard::COLUMNS as f32 / 2.0;
        for (index, key) in keys.iter().enumerate() {
            let (row, column) = Keyboard::position(index);
            let cell = Rect::new(
                origin_x + column as f32 * size,
                area.y + row as f32 * size,
                size,
                size,
            )
            .inset(size * 0.12);
            regions.push((cell, Region::Key(index)));
            let selected = index == self.keyboard.cursor();
            list.panel(
                cell,
                if selected {
                    style.accent
                } else {
                    style.surface_raised
                },
                style.metrics.radius * 0.7,
            );
            list.text(
                cell,
                key.label(),
                TextStyle::new(
                    if key.wide() { size * 0.24 } else { size * 0.45 },
                    if selected {
                        style.on_accent
                    } else {
                        style.text
                    },
                )
                .centered(),
            );
        }
    }

    fn draw_menu(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let row_h = style.gap(3.0);
        // Sized from what is in it rather than from a guess: the heading, the rows, the help
        // line and the card's own inset. Guessing left the last two rows below the window.
        let wanted = style.gap(3.2) + row_h * (Item::ALL.len() as f32 + 2.6);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.5).min(760.0),
            wanted.min(area.h * 0.94),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(1.6));
        let (heading, rest) = inner.cut_top(row_h);
        list.text(
            heading,
            "The whole song",
            TextStyle::new(style.scaled_text(1.2), style.text).bold(),
        );
        let (help, rows) = rest.cut_bottom(row_h * 1.6);
        list.text(
            help,
            Item::ALL[self.menu.min(Item::ALL.len() - 1)].help(),
            TextStyle::new(style.scaled_text(0.8), style.muted).valign(VAlign::Middle),
        );
        // Clipped as well as sized, so a very short window shortens the list rather than
        // drawing it over the footer.
        let mut placed = Vec::new();
        list.clipped(rows, |list| {
            for (index, item) in Item::ALL.iter().enumerate() {
                let rect = Rect::new(rows.x, rows.y + row_h * index as f32, rows.w, row_h)
                    .inset_xy(0.0, style.gap(0.2));
                if rect.y > rows.bottom() {
                    break;
                }
                placed.push((rect, Region::Item(index)));
                let value = match item {
                    Item::Track if self.editor.tracks() < 2 => "not a duet",
                    Item::Save if !self.editor.dirty() => "nothing to save",
                    _ => "",
                };
                widgets.row(list, rect, item.label(), value, index == self.menu);
            }
        });
        regions.extend(placed);
    }

    fn draw_leaving(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        regions: &mut Vec<(Rect, Region)>,
    ) {
        let widgets = Widgets::new(style);
        widgets.scrim(list, area);
        let card = area.anchored(
            Anchor::Center,
            (area.w * 0.5).min(760.0),
            style.gap(16.0),
            0.0,
        );
        widgets.card(list, card);
        let inner = card.inset(style.gap(2.5));
        let (heading, rest) = inner.cut_top(style.gap(4.0));
        list.text(
            heading,
            "Save this song?",
            TextStyle::new(style.scaled_text(1.2), style.text)
                .bold()
                .centered(),
        );
        let (buttons, body) = rest.cut_bottom(style.gap(5.0));
        list.text(
            body,
            "It has been changed since it was last written.",
            TextStyle::new(style.text_size(), style.muted).centered(),
        );
        let gap = style.gap(1.5);
        let width = (buttons.w - gap) / 2.0;
        // Save on the left and selected first: somebody who pressed Escape with unsaved work
        // almost always meant to keep it.
        for (index, (label, leaving)) in [("Save", false), ("Discard", true)].iter().enumerate() {
            let rect = Rect::new(
                buttons.x + (width + gap) * index as f32,
                buttons.y + gap,
                width,
                buttons.h - gap * 2.0,
            );
            regions.push((rect, Region::Confirm(*leaving)));
            let selected = self.confirm_leave == *leaving;
            list.panel(
                rect,
                match (selected, leaving) {
                    (true, true) => style.danger,
                    (true, false) => style.accent,
                    _ => style.surface,
                },
                style.metrics.radius,
            );
            list.text(
                rect,
                *label,
                TextStyle::new(
                    style.text_size(),
                    if selected {
                        style.on_accent
                    } else {
                        style.text
                    },
                )
                .centered()
                .bold(),
            );
        }
        let _ = Align::End;
    }
}
