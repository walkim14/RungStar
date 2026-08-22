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
use crate::settings::{LyricEffect, VideoSize};
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
    /// Which part of a duet this belongs to. Always `0` for an ordinary song.
    pub part: usize,
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
    /// The pitch the detector reported.
    pub pitch: i32,
    pub hit: bool,
}

/// One part's worth of what is being sung right now.
///
/// An ordinary song has one of these; a duet has two, and each gets its own staff and its own
/// lyric line so neither singer has to read the other's part to find their own.
pub struct PartView<'a> {
    pub line: &'a NoteLine,
    pub syllables: &'a [Syllable],
    pub next_line: &'a str,
}

/// The notes of one line, and the beats it spans.
///
/// The screen draws a line at a time rather than a scrolling window. UltraStar does the same,
/// and the reason is not stylistic: with a window, notes slide past and there is no moment
/// where a note and what you sang against it are both on screen long enough to compare. A
/// static line with a sweeping playhead leaves the whole phrase visible, and the mark you
/// made on it stays put until the line is over.
#[derive(Debug, Clone, Default)]
pub struct NoteLine {
    pub notes: Vec<Note>,
    /// First beat of the line, before any lead-in.
    pub start: f64,
    /// Last beat of the line.
    pub end: f64,
}

impl NoteLine {
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// The note due at `beat`, or the nearest one if the beat falls in a gap.
    ///
    /// Used to decide which octave to draw a sung pitch in, so the marker appears against the
    /// note it is being scored against.
    pub fn note_at(&self, beat: f64) -> Option<&Note> {
        self.notes
            .iter()
            .find(|n| beat >= n.start && beat < n.end())
            .or_else(|| {
                self.notes.iter().min_by(|a, b| {
                    let da = (a.start - beat).abs().min((a.end() - beat).abs());
                    let db = (b.start - beat).abs().min((b.end() - beat).abs());
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
            })
    }
}

/// Fold a sung pitch into the octave nearest the target.
///
/// Matching is octave-agnostic — singing the right note an octave down scores, and should.
/// But the detector reports the octave it actually heard, so drawing that raw puts the marker
/// twelve semitones from the note it just scored against: the game says you hit it and the
/// picture says you did not. The scorer folds before comparing; the display has to fold the
/// same way or it is describing a different comparison.
pub fn fold_to_octave(sung: i32, target: i32) -> i32 {
    let mut folded = sung;
    while folded - target > 6 {
        folded -= 12;
    }
    while target - folded > 6 {
        folded += 12;
    }
    folded
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
    /// Stop the instrumental tail and show the scores, keeping them.
    SkipOutro,
}

/// The sing screen.
pub struct SingScreen {
    pub title: String,
    pub artist: String,
    pub singers: Vec<Singer>,
    pub overlay: Overlay,
    /// Background artwork, when the song has one and backgrounds are on.
    pub background: Option<ImageId>,
    /// The song's video, which replaces the artwork while it is playing.
    pub video: Option<ImageId>,
    /// The video's shape, so it is letterboxed rather than stretched.
    pub video_aspect: f32,
    /// How much of the screen the video fills.
    pub video_size: VideoSize,
    /// Seconds into the song, for the progress bar.
    pub position: f32,
    pub duration: f32,
    /// How many note-grid beats pass in a second, i.e. `#BPM * 4 / 60`.
    ///
    /// The screen works in beats everywhere else, but "long enough to be worth counting in"
    /// is a wall-clock judgement: the lead-in is two seconds at `#BPM` 120 and well under one
    /// at 300, so the same rule written in beats would count a pause in one song and let the
    /// singer sit through it in the other.
    pub beat_rate: f64,
    pub gamepad: bool,
    pub show_input_panel: bool,
    /// Whether the singer panels are shown. The jukebox has nobody singing.
    pub show_panels: bool,
    /// Whether the words are shown. A challenge takes them away.
    pub show_lyrics: bool,
    /// Whether the note staff is shown.
    pub show_notes: bool,
    /// The name of the challenge being played, when it is not the plain song.
    pub challenge: Option<String>,
    /// The rating bar that has to be cleared right now, and where each singer stands against
    /// it. `None` when the challenge cannot put anybody out.
    pub bar: Option<f32>,
    /// Singers already knocked out, in singer order.
    pub knocked_out: Vec<bool>,
    /// Whether the backing track is audible. Deaf cuts it, and silence with no explanation
    /// reads as the game having crashed.
    pub audible: bool,
    /// How the highlight moves along the line.
    pub effect: LyricEffect,
    /// Where each singer's newest mark is *drawn* to, easing toward where it was sung.
    ///
    /// Scoring produces one whole beat at a time, and a beat is a wide piece of a staff --
    /// a sixteen-beat line makes it a sixteenth of the width -- so a mark drawn straight from
    /// the score jumps across the screen in visible steps however fast the beats arrive. The
    /// number here is what the eye follows; the scored end is what it is heading for.
    grown: Vec<f64>,
    /// The beat the marks were last advanced at, so the ease is in song time rather than in
    /// frames. A frame rate is not a clock.
    grown_at: f64,
    /// Per part: which line is on screen, and the beat it arrived at.
    ///
    /// A line arrives the moment the previous one ends, which is also the moment its sweeping
    /// bar parks at the left edge — so this is how long the bar has been sitting still, which
    /// is the one thing needed to tell an instrumental break from an ordinary breath.
    parked: Vec<(f64, f64)>,
    /// The two parts of a duet, named as the song names them.
    ///
    /// Empty for an ordinary song, which is the difference the layout keys off: a duet gets a
    /// staff and a lyric line each, stacked, so both singers can see their own part without
    /// reading the other's.
    pub parts: Vec<String>,
    /// Which part each singer is on, by index into `parts`.
    pub singer_part: Vec<usize>,
    /// The song's whole pitch range, so a note sits at the same height from first line to
    /// last. Deriving the scale from whatever happens to be on screen makes notes jump
    /// vertically as the window moves, which is the one thing a pitch display must not do.
    pub pitch_low: i32,
    pub pitch_high: i32,
    pause_cursor: usize,
    /// Clickable pause-menu rows from the last frame. Recorded while drawing, so hit testing
    /// cannot drift from the picture.
    pause_regions: Vec<Rect>,
    /// Whether the last note has gone by, so the screen can offer to skip the outro.
    ///
    /// An instrumental tail is part of the song and worth hearing, but sitting through forty
    /// seconds of it with nothing left to sing is a different matter — and the alternative
    /// on offer was "Give up", which throws the score away.
    pub outro: bool,
    /// The area the results card covers. Kept for the pointer, which the layout is otherwise
    /// the only thing that knows about.
    results_card: Option<Rect>,
}

/// A note both parts of a duet sing, marked so it can be drawn in its own colour.
pub const BOTH_PARTS: usize = usize::MAX;

/// Semitones shown on the staff either side of the song's own range.
const STAFF_MARGIN: i32 = 2;

/// How long a line-bonus rating stays on screen, in seconds.
const RATING_LIFETIME: f32 = 1.4;

/// How far ahead of the first syllable the sweeping bar starts, in beats.
///
/// The note grid runs at four times the written BPM, so sixteen of these is one bar of
/// four-four — the count-in a person would give you. Four was the first attempt and was
/// useless: it began beside the first word and arrived with the notes, which tells you
/// nothing you could not already see.
const LEAD_IN_BEATS: f64 = 16.0;

/// Beats per second assumed until the song says otherwise: `#BPM` 120, the middle of what a
/// real library holds.
const DEFAULT_BEAT_RATE: f64 = 8.0;

/// A park shorter than this gets no count-in, in seconds.
///
/// Every line is parked for a moment while the previous one finishes, and a count that flashes
/// up for half a second is a distraction rather than a warning — it is over before it has been
/// read, and it happens on every line of the song.
const COUNTDOWN_MIN_SECONDS: f64 = 1.5;

/// The most of a park the count-in covers, in seconds.
///
/// Three, because that is what a person counts you in with, and because a longer count spends
/// most of an instrumental break telling you about a bar that has not moved for twenty
/// seconds and is not about to.
const COUNTDOWN_SECONDS: f64 = 3.0;

/// How quickly a mark's drawn end catches up with its scored end, in beats.
const MARK_EASE_BEATS: f64 = 0.6;
/// The furthest a drawn end may fall behind before it stops easing and jumps.
const MARK_MAX_LAG: f64 = 1.5;
/// An interval longer than this is a seek or a pause, not a frame, and is snapped through.
const MARK_MAX_STEP: f64 = 4.0;

/// Where a lyric count-in stands: how many marks it has, how many are still to come, and how
/// far through the one being counted it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Countdown {
    /// Marks in the row, which is how many seconds the count covers.
    pub marks: usize,
    /// Marks still to be spent, including the one running now. Never zero.
    pub left: usize,
    /// `1.0` at the start of the mark being counted, falling to `0.0` as it runs out.
    pub tick: f32,
}

/// Whether a parked lyric bar is worth counting in, and where that count stands.
///
/// `park` is how long the bar sits still altogether before it sets off; `remaining` how much of
/// that is left. Seconds rather than beats, because whether a wait is long enough to need
/// telling about is a question about the singer and not about the tempo.
pub fn countdown(park: f64, remaining: f64) -> Option<Countdown> {
    if !park.is_finite() || !remaining.is_finite() || park < COUNTDOWN_MIN_SECONDS {
        return None;
    }
    // A park shorter than the full count is covered as far as it goes rather than not at all:
    // the alternative is a two-second silence with nothing to say it is nearly over, which is
    // the case this exists for.
    let window = park.min(COUNTDOWN_SECONDS);
    if remaining <= 0.0 || remaining > window {
        return None;
    }
    let marks = window.ceil() as usize;
    let left = (remaining.ceil() as usize).clamp(1, marks);
    Some(Countdown {
        marks,
        left,
        tick: (remaining - (left as f64 - 1.0)).clamp(0.0, 1.0) as f32,
    })
}

/// A row of marks that empties from the right as the count runs down.
///
/// Marks rather than digits: a count-in is read out of the corner of the eye while the words
/// are being read properly, and a shortening row says "nearly" without being looked at. It also
/// cannot be drawn as an empty box by a font that has no digit for it, which is the failure the
/// star ratings once had.
fn draw_countdown(list: &mut DrawList, area: Rect, style: &Style, colour: Color, count: Countdown) {
    if count.marks == 0 {
        return;
    }
    let radius = (area.h * 0.26).max(2.0);
    let step = radius * 3.4;
    let width = step * (count.marks as f32 - 1.0) + radius * 2.0;
    let centre = area.center();
    let mut x = centre.x - width / 2.0;
    for slot in 0..count.marks {
        let spent = slot >= count.left;
        // The mark being counted shrinks away over its own second, so the row empties
        // smoothly rather than a whole mark at a time. It never quite vanishes before its
        // turn is over, or the row reads as one shorter than it is.
        let scale = if !spent && slot + 1 == count.left {
            count.tick.max(0.2)
        } else {
            1.0
        };
        let r = radius * scale;
        let ink = if spent {
            style.muted.alpha(0.3)
        } else {
            colour
        };
        list.panel(
            Rect::new(x + radius - r, centre.y - r, r * 2.0, r * 2.0),
            ink,
            r,
        );
        x += step;
    }
}

/// What colour a mark on the staff takes: whose it is, and whether it was a hit.
///
/// A miss used to be one shared red whoever sang it. That is fine for one singer and useless
/// for six: a row of identical marks says a note was missed and not by whom, when identifying
/// the singer is the entire reason the mark sits at the pitch they actually sang rather than on
/// the note. So a miss is the singer's own colour, darkened -- the *shape* already says it was
/// a miss (a thin bar off the note, against a full bubble on it), which leaves the colour free
/// to say who.
///
/// A mark several singers share belongs to none of them and takes a colour of the interface's
/// own rather than one of theirs: the accent when they all hit it, and a plain muted grey when
/// they all missed, because picking one singer's colour there would name the wrong person.
pub fn mark_colour(style: &Style, singers: &[usize], hit: bool) -> Color {
    let shared = singers.len() > 1;
    match (hit, shared) {
        (true, true) => style.accent,
        (true, false) => style.player(singers.first().copied().unwrap_or(0)),
        (false, true) => style.muted.darken(0.15),
        // Dark enough to sit under the hits without going black on a dark player colour, which
        // is why it is a darken rather than an alpha: over a song video a faded mark vanishes.
        (false, false) => style
            .player(singers.first().copied().unwrap_or(0))
            .darken(0.42),
    }
}

fn stage_pulse(beat: f64) -> f32 {
    if !beat.is_finite() {
        return 0.0;
    }
    // UltraStar's grid has four units per musical beat. A quick fifth-power decay reads as a
    // light hit rather than a brightness wobble, and reaches its quiet level well before the
    // next beat.
    let phase = (beat.rem_euclid(4.0) / 4.0) as f32;
    (1.0 - phase).powi(5)
}

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
            video: None,
            video_aspect: 16.0 / 9.0,
            video_size: VideoSize::default(),
            position: 0.0,
            duration: 0.0,
            beat_rate: DEFAULT_BEAT_RATE,
            gamepad: false,
            show_input_panel: false,
            show_panels: true,
            show_lyrics: true,
            show_notes: true,
            challenge: None,
            bar: None,
            knocked_out: Vec::new(),
            audible: true,
            effect: LyricEffect::default(),
            grown: Vec::new(),
            grown_at: f64::NEG_INFINITY,
            parked: Vec::new(),
            parts: Vec::new(),
            singer_part: Vec::new(),
            pitch_low: 0,
            pitch_high: 12,
            pause_cursor: 0,
            pause_regions: Vec::new(),
            outro: false,
            results_card: None,
        }
    }

    pub fn pause_cursor(&self) -> usize {
        self.pause_cursor
    }

    /// Handle an input. Returns a transition, and separately what the pause menu chose.
    pub fn handle(&mut self, input: Input) -> (Transition, Option<PauseChoice>) {
        if let Input::Hover(point) | Input::Click(point) = input {
            let clicked = matches!(input, Input::Click(_));
            return self.handle_pointer(point, clicked);
        }
        match self.overlay {
            Overlay::None => match input {
                // Back pauses rather than quitting. Leaving a song by accident in the middle
                // of a party is worse than one extra press.
                Input::Back | Input::Random => {
                    self.overlay = Overlay::Paused;
                    self.pause_cursor = 0;
                    (Transition::None, None)
                }
                // Confirm does nothing while there is still singing to do — a stray press
                // must not end a song — but once the last note has gone it skips to the
                // scores, keeping them.
                Input::Confirm | Input::Submit if self.outro => {
                    (Transition::None, Some(PauseChoice::SkipOutro))
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

    /// Move the pause cursor to whatever the pointer is over, and act on it if clicked.
    ///
    /// The song itself takes no pointer input — there is nothing on it to click — but a menu
    /// that ignores the mouse while every other menu accepts it just reads as broken.
    fn handle_pointer(&mut self, point: Point, clicked: bool) -> (Transition, Option<PauseChoice>) {
        match self.overlay {
            Overlay::Paused => {
                if let Some(index) = self.pause_regions.iter().position(|r| r.contains(point)) {
                    self.pause_cursor = index;
                    if clicked {
                        return self.handle(Input::Confirm);
                    }
                }
                (Transition::None, None)
            }
            Overlay::Results => {
                // A click anywhere dismisses. The card is only waiting to be acknowledged,
                // so aiming at it is not something to ask of somebody halfway through a
                // party.
                if clicked {
                    (Transition::Pop, None)
                } else {
                    (Transition::None, None)
                }
            }
            Overlay::None => (Transition::None, None),
        }
    }

    /// Whether this song is being sung as a duet.
    pub fn is_duet(&self) -> bool {
        self.parts.len() > 1
    }

    /// The colour of a part, taken from the first singer on it.
    fn part_colour(&self, part: usize, style: &Style) -> Color {
        let singer = self
            .singer_part
            .iter()
            .position(|p| *p == part)
            .unwrap_or(part);
        style.player(singer)
    }

    /// Both parts' notes on one page, tagged with whose they are.
    ///
    /// Each part picks its own current line, and in a duet those lines are rarely the same
    /// stretch of song — while one part sings, the other's "current" line is the one it sings
    /// next, which can be half a verse away. Merging the two spans directly produced a page
    /// running from one part's line to the other's, which squeezed the notes being sung now
    /// into a corner and showed the other singer's part long before it was due.
    ///
    /// So the page is one part's line — whichever is actually being sung — and the other
    /// part's notes appear only where they fall inside it.
    fn merge_parts(&self, parts: &[PartView<'_>], beat: f64) -> NoteLine {
        if parts.len() < 2 {
            return parts.first().map(|p| p.line.clone()).unwrap_or_default();
        }

        // The line being sung now, or the next one due if there is a gap between phrases.
        let window = parts
            .iter()
            .map(|p| p.line)
            .filter(|line| !line.is_empty())
            .min_by(|a, b| {
                let rank = |line: &NoteLine| {
                    if beat >= line.start && beat < line.end {
                        (0, 0.0)
                    } else if line.start > beat {
                        (1, line.start - beat)
                    } else {
                        (2, beat - line.end)
                    }
                };
                let (a_rank, a_gap) = rank(a);
                let (b_rank, b_gap) = rank(b);
                a_rank.cmp(&b_rank).then(
                    a_gap
                        .partial_cmp(&b_gap)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
            });
        let Some(window) = window else {
            return NoteLine::default();
        };

        let mut notes: Vec<Note> = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            for note in &part.line.notes {
                // Only what falls inside the page. The other part's later phrases stay off it.
                if note.end() <= window.start || note.start >= window.end {
                    continue;
                }
                match notes
                    .iter_mut()
                    .find(|other| other.start == note.start && other.pitch == note.pitch)
                {
                    Some(shared) => shared.part = BOTH_PARTS,
                    None => notes.push(Note {
                        part: index,
                        ..*note
                    }),
                }
            }
        }
        notes.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        NoteLine {
            notes,
            start: window.start,
            end: window.end,
        }
    }

    /// Draw a frame.
    ///
    /// `parts` carries one entry per part being sung — one for an ordinary song, two for a
    /// duet. `beat` is the drawing clock, which runs ahead of the scoring clock by the
    /// microphone delay.
    pub fn draw(
        &mut self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        parts: &[PartView<'_>],
        beat: f64,
    ) {
        let Some(first) = parts.first() else {
            return;
        };
        let line = first.line;
        let syllables = first.syllables;
        let next_line = first.next_line;
        // The video wins over the artwork while it is playing; both sit behind a scrim,
        // because the lyrics have to stay readable over any picture, including a white one or
        // a cut to a flashbulb.
        if let Some(video) = self.video {
            let rect = match self.video_size {
                // Filling the screen crops the edges, which is right for a background: a
                // border round a music video looks like a mistake.
                VideoSize::Full => area.cover_aspect(self.video_aspect),
                // Fitting shows all of it, letterboxed.
                VideoSize::Fit => area.fit_aspect(self.video_aspect),
            };
            list.clipped(area, |list| {
                list.image_tinted(rect, video, Color::WHITE, 0.0);
            });
            // Lighter than the artwork's scrim. A video is the background you actually chose
            // to watch, and dimming it to the point where the panels read as the picture
            // defeats having it.
            list.fill(area, style.background.alpha(0.3));
        } else if let Some(image) = self.background {
            list.image_tinted(area, image, Color::WHITE.alpha(0.35), 0.0);
            list.fill(area, style.background.alpha(0.55));
        }
        list.stage_pulse(stage_pulse(beat));

        let (header, body) = area.cut_top(style.gap(4.0));
        self.draw_header(list, header, style);

        // Singers take a strip down the side, so the staff keeps the middle whatever the
        // player count. Six panels on a Deck are still readable at this width.
        let panel_w = if self.show_panels {
            (area.w * 0.18).clamp(220.0, 340.0)
        } else {
            0.0
        };
        let (panels, middle) = body.cut_right(panel_w);
        if self.show_panels {
            self.draw_panels(list, panels.inset(style.gap(1.0)), style);
        }

        // One staff whatever the part count, with a duet's notes coloured by whose they are.
        // Two stacked staves was the first attempt and reads worse: it halves the height of
        // both, and a duet spends most of its time with only one part singing, so half the
        // screen sits empty while the other half is cramped.
        let lyric_height = style.gap(4.0) + style.gap(3.4) * parts.len() as f32;
        let (lyrics_area, staff_area) = middle.cut_bottom(lyric_height);

        let merged = self.merge_parts(parts, beat);
        self.advance_marks(beat);
        self.observe_lines(parts, beat);
        if self.show_notes {
            self.draw_staff(list, staff_area.inset(style.gap(1.5)), style, &merged, beat);
        } else {
            // Not simply nothing: an empty middle of the screen looks like a song that failed
            // to load. The panel that would have held the staff stays, and says why.
            self.draw_hidden(
                list,
                staff_area.inset(style.gap(1.5)),
                style,
                "Notes hidden",
            );
        }

        // Both parts' words, stacked at the bottom in their own colours, each with its own
        // bar. A singer finds their line by its colour rather than by working out which half
        // of the screen is theirs.
        let rows = lyrics_area.rows(parts.len().max(1), 0.0);
        for (index, (part, row)) in parts.iter().zip(rows).enumerate() {
            if !self.show_lyrics {
                if index == 0 {
                    self.draw_hidden(list, lyrics_area, style, "Words hidden");
                }
                continue;
            }
            let tint = (parts.len() > 1).then(|| self.part_colour(index, style));
            let parked = self.parked.get(index).map_or(beat, |(_, at)| *at);
            self.draw_lyrics(list, row, style, part, beat, tint, parked);
        }
        let _ = (line, syllables, next_line);

        if self.outro && self.overlay == Overlay::None {
            let hint = if self.gamepad { "A" } else { "Enter" };
            let strip = Rect::new(
                area.x,
                area.bottom() - style.gap(3.0),
                area.w,
                style.gap(2.2),
            );
            list.text(
                strip,
                format!("{hint} for your score"),
                TextStyle::new(style.scaled_text(0.85), style.muted).centered(),
            );
        }

        match self.overlay {
            Overlay::Paused => self.draw_pause(list, area, style),
            Overlay::Results => self.draw_results(list, area, style),
            Overlay::None => {}
        }
    }

    /// What a challenge has taken away, said rather than left blank.
    ///
    /// A blank middle of the screen is indistinguishable from a song that failed to load, and
    /// somebody who joined the party ten minutes ago has no way to tell which it is.
    fn draw_hidden(&self, list: &mut DrawList, area: Rect, style: &Style, what: &str) {
        list.panel(area, style.surface.alpha(0.25), style.metrics.radius);
        list.text(
            area,
            what,
            TextStyle::new(style.scaled_text(0.9), style.muted).centered(),
        );
    }

    /// The bar a rating has to clear, drawn across the top under the progress line.
    ///
    /// A rule that puts somebody out without showing how close they were is a rule nobody can
    /// play against, and "you are out" with no warning reads as a bug.
    fn draw_bar(&self, list: &mut DrawList, area: Rect, style: &Style, bar: f32) {
        let track = Rect::new(area.x, area.y, area.w, style.gap(0.9));
        list.panel(track, style.surface_sunken, track.h / 2.0);
        list.panel(
            Rect::new(track.x, track.y, track.w * bar.clamp(0.0, 1.0), track.h),
            style.danger.alpha(0.55),
            track.h / 2.0,
        );
        list.text(
            Rect::new(area.x, area.y + track.h, area.w, style.gap(1.8)),
            format!("Hold the line \u{2014} {}%", (bar * 100.0).round() as i32),
            TextStyle::new(style.scaled_text(0.72), style.muted).centered(),
        );
    }

    fn draw_header(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let inner = area.inset_xy(style.gap(2.0), 0.0);
        let (title_area, right) = inner.cut_left(inner.w * 0.6);
        list.text(
            title_area,
            format!("{} \u{2013} {}", self.artist, self.title),
            TextStyle::new(style.scaled_text(0.95), style.text)
                .bold()
                .overflow(Overflow::Ellipsis),
        );
        // What is being played, when it is not just the song. Somebody who walked in has to be
        // able to see why the words are missing.
        if let Some(challenge) = &self.challenge {
            let label = if self.audible {
                challenge.clone()
            } else {
                format!("{challenge} \u{2014} music off")
            };
            list.text(
                right,
                label,
                TextStyle::new(
                    style.scaled_text(0.85),
                    if self.audible {
                        style.accent
                    } else {
                        style.danger
                    },
                )
                .align(Align::End)
                .bold()
                .overflow(Overflow::Ellipsis),
            );
        }

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
        if let Some(bar) = self.bar {
            self.draw_bar(
                list,
                Rect::new(
                    inner.x,
                    area.bottom() + style.gap(0.4),
                    inner.w,
                    style.gap(2.6),
                ),
                style,
                bar,
            );
        }
    }

    fn draw_panels(&self, list: &mut DrawList, area: Rect, style: &Style) {
        let rows = area.rows(self.singers.len(), style.gap(1.0));
        for (index, (singer, row)) in self.singers.iter().zip(rows).enumerate() {
            let color = style.player(index);
            let opacity = if self.video.is_some() { 0.62 } else { 0.9 };
            list.panel(row, style.surface.alpha(opacity), style.metrics.radius);
            let inner = row.inset(style.gap(1.0));

            // Somebody knocked out keeps their panel and their score. Removing it would take
            // away the one thing they want to look at, which is how far they got.
            let out = self.knocked_out.get(index).copied().unwrap_or(false);
            let (name_row, rest) = inner.cut_top(style.scaled_text(0.85) * 1.4);
            let (name_row, out_tag) = if out {
                name_row.cut_left(name_row.w * 0.62)
            } else {
                (name_row, Rect::default())
            };
            list.text(
                name_row,
                &singer.name,
                TextStyle::new(
                    style.scaled_text(0.85),
                    if out { style.muted } else { color },
                )
                .bold()
                .overflow(Overflow::Ellipsis),
            );
            if out {
                list.text(
                    out_tag,
                    "OUT",
                    TextStyle::new(style.scaled_text(0.78), style.danger)
                        .bold()
                        .align(Align::End),
                );
            }

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

            // The meter is always there. Showing it only while something was wrong meant it
            // vanished the moment you sang loudly enough, which is the opposite of
            // reassuring: the reading disappeared exactly when it turned good.
            let wordy = self.show_input_panel || singer.input_problem().is_some();
            let height = if wordy {
                style.gap(2.4)
            } else {
                style.gap(1.0)
            };
            let strip = Rect::new(inner.x, inner.bottom() - height, inner.w, height);
            self.draw_input_strip(list, strip, style, singer, color, wordy);
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
        wordy: bool,
    ) {
        let (meter, label) = if wordy {
            area.cut_top(style.gap(0.9))
        } else {
            (area, Rect::default())
        };
        let track = meter.anchored(Anchor::Center, meter.w, style.gap(0.5), 0.0);
        let radius = track.h / 2.0;
        list.panel(track, style.surface_sunken, radius);

        // The stretch below the gate is shaded, so the threshold reads as a region to get past
        // rather than as a line things vanish behind.
        let gate = singer.gate.clamp(0.0, 1.0);
        if gate > 0.0 {
            list.panel(
                Rect::new(track.x, track.y, track.w * gate, track.h),
                style.warning.alpha(0.16),
                radius,
            );
        }

        let level = singer.level.clamp(0.0, 1.0);
        if level > 0.0 {
            // Below the gate the fill is grey — present, but visibly not counting. Above it,
            // the singer's colour. The bar never disappears; only its colour changes.
            list.panel(
                Rect::new(
                    track.x,
                    track.y,
                    (track.w * level).max(radius * 2.0),
                    track.h,
                ),
                if level >= gate { color } else { style.muted },
                radius,
            );
        }

        // A notch at the gate rather than a bar through it, so it marks the boundary without
        // looking like part of the reading.
        let gate_x = track.x + track.w * gate;
        list.fill(
            Rect::new(gate_x - 1.0, track.y - 1.5, 2.0, track.h + 3.0),
            style.warning.alpha(0.8),
        );

        if !wordy {
            return;
        }

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

    /// Move each singer's drawn mark end toward where they have actually been scored.
    ///
    /// An exponential ease in *beats*, not in frames, so it looks the same at 60 and at 144
    /// and the same in a slow song as in a fast one. Two guards matter: a mark never starts
    /// before its own run, so a new run grows out of its beginning rather than sliding in from
    /// the last one, and it is never allowed to fall more than a beat behind, so a pause or a
    /// dropped frame is caught up in one step instead of crawling.
    fn advance_marks(&mut self, beat: f64) {
        self.grown.resize(self.singers.len(), f64::NEG_INFINITY);
        // The first frame, and any frame after the clock moved backwards, has no interval to
        // ease over. Snapping there is right: there is nothing on screen yet to be smooth.
        let elapsed = beat - self.grown_at;
        let snap = !(0.0..=MARK_MAX_STEP).contains(&elapsed);
        self.grown_at = beat;

        for (index, singer) in self.singers.iter().enumerate() {
            let Some(last) = singer.sung.last() else {
                continue;
            };
            let target = last.start + last.duration;
            let drawn = &mut self.grown[index];
            // Past the target means the history was trimmed or the song restarted under it.
            if snap || *drawn > target {
                *drawn = target;
                continue;
            }
            // A gap where nobody sang is crossed at once rather than crawled over: there is
            // nothing to draw in it, and creeping across it would delay the new run.
            if *drawn < last.start {
                *drawn = last.start;
            }
            *drawn = drawn.max(target - MARK_MAX_LAG);
            *drawn += (target - *drawn) * (1.0 - (-elapsed / MARK_EASE_BEATS).exp());
        }
    }

    /// Note when each part's line arrived, which is when its sweeping bar parked.
    ///
    /// Kept here rather than handed in because it is a property of the display and not of the
    /// song: the line changes when the screen starts showing it, and nothing upstream knows or
    /// should have to know when that was.
    fn observe_lines(&mut self, parts: &[PartView<'_>], beat: f64) {
        self.parked.resize(parts.len(), (f64::NAN, beat));
        for (part, parked) in parts.iter().zip(&mut self.parked) {
            let line = part.syllables.first().map_or(f64::NAN, |s| s.start);
            // A line that is not the one we saw last frame has only just arrived. So has one
            // we have apparently been showing since after now, which is a restart or a seek
            // rather than a wait anybody has sat through.
            if parked.0 != line || beat < parked.1 {
                *parked = (line, beat);
            }
        }
    }

    fn draw_staff(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        line: &NoteLine,
        beat: f64,
    ) {
        // Barely there over a video: the staff is a guide, not a panel, and a song video is
        // the thing behind it rather than a thing beside it.
        let backing = if self.video.is_some() { 0.28 } else { 0.55 };
        list.panel(area, style.surface.alpha(backing), style.metrics.radius);
        if line.is_empty() {
            return;
        }

        // The line fills the width, with a lead-in so the playhead is already moving before
        // the first note and does not appear out of the left edge on the beat it is due.
        let span = (line.end - line.start).max(1.0);
        let pad = (span * 0.06).clamp(0.5, 4.0);
        let from = line.start - pad;
        let to = line.end + pad;

        // The scale is the song's, not this line's, so a note does not change height when the
        // line changes. A line covering three semitones would otherwise fill the staff and
        // make three semitones look like an octave.
        let lowest = self.pitch_low - STAFF_MARGIN;
        let highest = self.pitch_high + STAFF_MARGIN;
        let rows = (highest - lowest).max(1) as f32;

        let inner = area.inset(style.gap(1.0));
        let row_h = inner.h / rows;
        let x_of = |b: f64| inner.x + ((b - from) / (to - from)) as f32 * inner.w;
        let y_of = |pitch: i32| inner.y + (highest - pitch.clamp(lowest, highest)) as f32 * row_h;

        // One faint line per semitone the line actually uses, rather than all of them: a full
        // grid over a two-octave scale is noise.
        let (line_low, line_high) = line.notes.iter().fold((i32::MAX, i32::MIN), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        for semitone in (line_low - 1)..=(line_high + 1) {
            let y = y_of(semitone) + row_h / 2.0;
            list.fill(
                Rect::new(inner.x, y - 0.5, inner.w, 1.0),
                style.muted.alpha(0.10),
            );
        }

        let note_h = (row_h * 0.72).clamp(6.0, 26.0);
        // Clipped: a note that started before the window still has to be drawn, and without
        // this it is drawn from off the left edge across whatever is beside the staff.
        list.clipped(inner, |list| {
            for note in &line.notes {
                let x = x_of(note.start);
                let w = (x_of(note.end()) - x).max(4.0);
                let y = y_of(note.pitch) + (row_h - note_h) / 2.0;
                let rect = Rect::new(x, y, w, note_h);

                // Whose note this is. In a duet the target is drawn in that singer's colour so
                // you can see whose turn is coming without reading the words; a solo stays
                // neutral, because there is only one answer.
                let owner = if !self.is_duet() {
                    None
                } else if note.part == BOTH_PARTS {
                    Some(style.text)
                } else {
                    Some(self.part_colour(note.part, style))
                };

                if note.kind.is_freestyle() {
                    // Freestyle scores nothing, so it is drawn as an outline: a cue, not a
                    // target, and filling it would promise points that never arrive.
                    let colour = owner.unwrap_or(style.muted);
                    list.outline(rect, colour.alpha(0.55), 2.2, note_h / 2.0);
                    continue;
                }

                let active = note.start <= beat && beat < note.end();
                match owner {
                    // A faint fill in the part's colour with a firm outline round it: legible
                    // as "yours, not yet sung" without competing with the bar that fills it in
                    // once it has been.
                    Some(colour) => {
                        // Golden fills the bubble, as it does in a solo song — it is the
                        // loudest thing on the staff and should look it. The outline carries
                        // whose it is, which is enough; a gold streak through a part-coloured
                        // bubble was trying to say both things in the same place.
                        let fill = if note.kind.is_golden() {
                            style.warning.alpha(0.92)
                        } else {
                            colour.alpha(0.28)
                        };
                        if active {
                            list.glow(
                                rect,
                                if note.kind.is_golden() {
                                    style.warning.alpha(0.68)
                                } else {
                                    colour.alpha(0.60)
                                },
                            );
                        }
                        list.bubble(rect, fill, colour.alpha(0.92));
                    }
                    None => {
                        let (fill, rim) = if note.kind.is_golden() {
                            (style.warning.alpha(0.92), style.warning.lighten(0.28))
                        } else {
                            (style.surface_raised.alpha(0.96), style.muted.alpha(0.78))
                        };
                        if active {
                            list.glow(
                                rect,
                                if note.kind.is_golden() {
                                    style.warning.alpha(0.68)
                                } else {
                                    style.accent.alpha(0.52)
                                },
                            );
                        }
                        list.bubble(rect, fill, rim);
                    }
                }
            }
        });

        // What each singer sang, over the top and clipped to this line. It stays on screen
        // until the line turns, which is the point: you can look at what you just did.
        struct Span {
            start: f64,
            end: f64,
            pitch: i32,
            hit: bool,
            singer: usize,
            golden: bool,
        }
        //
        // The end of the newest one is eased rather than stepped, but it is never pushed
        // *past* what was scored to meet the playhead: extending it to the playhead hides the
        // microphone delay and then lurches back when the detection lands, which is worse
        // than the delay it was hiding.
        let mut spans: Vec<Span> = Vec::new();
        for (index, singer) in self.singers.iter().enumerate() {
            // One moving frontier per singer rather than an ease per run: every mark is
            // clipped to it, so a run grows in from its own beginning, stops exactly where it
            // was sung, and the one after it starts from nothing. Easing each run's end
            // separately instead leaves the last few units to snap on when the next begins.
            let frontier = self.grown.get(index).copied().unwrap_or(f64::INFINITY);
            for sung in &singer.sung {
                let sung_end = (sung.start + sung.duration).min(frontier);
                if sung_end <= sung.start || sung_end < line.start || sung.start > line.end {
                    continue;
                }
                let target = line.note_at(sung.start);
                let pitch = match target {
                    Some(note) if sung.hit => note.pitch,
                    Some(note) => fold_to_octave(sung.pitch, note.pitch),
                    None => sung.pitch,
                };
                spans.push(Span {
                    start: sung.start.max(line.start),
                    end: sung_end.min(line.end),
                    pitch,
                    hit: sung.hit,
                    singer: index,
                    golden: target.is_some_and(|note| note.kind.is_golden()),
                });
            }
        }

        // Split at every boundary and colour each slice by exactly who was singing during it.
        // Merging on any overlap was the earlier version, and one stray beat from a second
        // microphone recoloured a whole run as "both of you" — which is a lie told in the one
        // place the display is supposed to be reporting fact.
        let mut edges: Vec<f64> = spans.iter().flat_map(|s| [s.start, s.end]).collect();
        edges.push(line.start);
        edges.push(line.end);
        edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        edges.dedup_by(|a, b| (*a - *b).abs() < 0.01);

        for pair in edges.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            if to - from < 0.01 {
                continue;
            }
            let middle = (from + to) / 2.0;
            // Every span covering this slice, which is who was singing in it.
            let covering: Vec<&Span> = spans
                .iter()
                .filter(|s| s.start <= middle && middle < s.end)
                .collect();
            if covering.is_empty() {
                continue;
            }

            // One slice per pitch, since two singers on different notes are two marks.
            let mut pitches: Vec<i32> = covering.iter().map(|s| s.pitch).collect();
            pitches.sort_unstable();
            pitches.dedup();
            for pitch in pitches {
                let here: Vec<&&Span> = covering.iter().filter(|s| s.pitch == pitch).collect();
                let hit = here.iter().any(|s| s.hit);
                let mut singers: Vec<usize> = here
                    .iter()
                    .filter(|s| s.hit == hit)
                    .map(|s| s.singer)
                    .collect();
                singers.sort_unstable();
                singers.dedup();

                let x = x_of(from);
                let w = (x_of(to) - x).max(3.0);
                let y = y_of(pitch) + (row_h - note_h) / 2.0;
                let colour = mark_colour(style, &singers, hit);
                if hit {
                    let rect = Rect::new(x, y, w, note_h);
                    if from <= beat && to >= beat - 0.8 {
                        list.glow(rect, colour.alpha(0.62));
                    }
                    list.bubble(rect, colour, colour.lighten(0.42));
                    // A golden note is worth double, and covering it with the hit hid the one
                    // thing worth remembering about it afterwards.
                    if here.iter().any(|s| s.golden) {
                        list.outline(rect, style.warning, 2.2, note_h / 2.0);
                    }
                } else {
                    // A miss stays a thin mark at the pitch actually sung: it belongs to no
                    // bubble, and drawing it as one would claim it did. The shape is what says
                    // "missed"; the colour says who missed it.
                    list.bubble(
                        Rect::new(x, y + note_h * 0.26, w, note_h * 0.48),
                        colour.alpha(0.88),
                        colour.lighten(0.30),
                    );
                }
            }
        }

        // The playhead sweeps the line rather than the notes sweeping past it.
        let head = x_of(beat).clamp(inner.x, inner.right());
        list.fill(
            Rect::new(head - 1.5, inner.y, 3.0, inner.h),
            style.accent.alpha(0.85),
        );

        // Where each singer is right now, on the playhead, whether or not a note is due.
        for (index, singer) in self.singers.iter().enumerate() {
            if let Some(pitch) = singer.pitch {
                // Drawn in the octave the scorer compared it in, not the one the microphone
                // heard it in.
                let pitch = match line.note_at(beat) {
                    Some(target) => fold_to_octave(pitch, target.pitch),
                    None => pitch,
                };
                let y = y_of(pitch) + row_h / 2.0;
                let marker = Rect::new(head - 16.0, y - note_h * 0.28, 32.0, note_h * 0.56);
                let colour = style.player(index);
                list.glow(marker, colour.alpha(0.72));
                list.bubble(marker, colour, colour.lighten(0.42));
            }
        }
    }

    /// Draw one part's words.
    ///
    /// `tint` is the part's colour when this is a duet; `None` uses the theme's accent.
    /// `parked` is the beat this line arrived at, which is how long its bar has sat still.
    #[allow(clippy::too_many_arguments)]
    fn draw_lyrics(
        &self,
        list: &mut DrawList,
        area: Rect,
        style: &Style,
        part: &PartView<'_>,
        beat: f64,
        tint: Option<Color>,
        parked: f64,
    ) {
        let syllables = part.syllables;
        let next_line = part.next_line;
        // Each part of a duet is highlighted in its own singer's colour, so a line is found by
        // its colour rather than by working out which of them is yours.
        let highlight = tint.unwrap_or(style.accent);
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
        let left = (current.center().x - total / 2.0).max(current.x);

        // Where each syllable sits, so the sweeping bar and the text agree exactly.
        let mut spans: Vec<(f32, f32)> = Vec::with_capacity(syllables.len());
        let mut pen = left;
        for syllable in syllables {
            let width = crate::draw::approx_text_width(&syllable.text, size);
            spans.push((pen, width));
            pen += width;
        }

        // The bar sweeps the line at the speed the words are sung, and runs in from the edge
        // of the screen well before the first syllable is due. Knowing *when* to come in is
        // most of singing a song you only half-remember, and a run-up that starts beside the
        // first word — or at the same moment the notes appear — is over before it has been
        // noticed.
        let first = &syllables[0];
        let trail = (total * 0.12).clamp(size * 1.2, size * 4.0);
        let head = if beat < first.start {
            let progress =
                ((beat - (first.start - LEAD_IN_BEATS)) / LEAD_IN_BEATS).clamp(0.0, 1.0) as f32;
            area.x + (left - area.x) * progress
        } else {
            // Inside the line: interpolate through whichever syllable is due.
            let mut position = left + total;
            for (syllable, (x, width)) in syllables.iter().zip(&spans) {
                let end = syllable.start + syllable.duration;
                if beat < syllable.start {
                    position = *x;
                    break;
                }
                if beat < end {
                    let through = ((beat - syllable.start) / syllable.duration.max(0.001)) as f32;
                    position = x + width * through.clamp(0.0, 1.0);
                    break;
                }
            }
            position
        };

        // A soft wedge behind the bar rather than a bare line, so the eye can follow it over
        // a busy background.
        let trail_from = (head - trail).max(area.x);
        let trail_width = (head - trail_from).max(0.0);
        if trail_width > 0.0 {
            list.panel(
                Rect::new(
                    trail_from,
                    current.y + current.h * 0.12,
                    trail_width,
                    current.h * 0.76,
                ),
                highlight.alpha(0.16),
                current.h * 0.38,
            );
        }
        list.panel(
            Rect::new(
                (head - 2.0).max(area.x),
                current.y + current.h * 0.08,
                4.0,
                current.h * 0.84,
            ),
            highlight.alpha(0.9),
            2.0,
        );

        // Shift moves the whole line so the syllable being sung stays under the eye, which
        // is the point of it: you never look away to find where you are.
        let shift = if self.effect == LyricEffect::Shift {
            let active = syllables
                .iter()
                .zip(&spans)
                .find(|(s, _)| beat < s.start + s.duration)
                .map(|(_, (x, w))| x + w / 2.0)
                .unwrap_or(current.center().x);
            current.center().x - active
        } else {
            0.0
        };

        for (syllable, (x, width)) in syllables.iter().zip(&spans) {
            let x = x + shift;
            let ends = syllable.start + syllable.duration;
            let done = beat >= ends;
            let active = beat >= syllable.start && !done;
            let through = if active {
                ((beat - syllable.start) / syllable.duration.max(0.001)).clamp(0.0, 1.0) as f32
            } else {
                f32::from(u8::from(done))
            };

            let resting = if syllable.golden {
                style.warning
            } else {
                style.muted
            };
            let colour = if active {
                highlight
            } else if done {
                style.text
            } else {
                resting
            };

            // Zoom swells the syllable as it is sung and settles again, so the eye is pulled
            // to it rather than having to track a colour change.
            let drawn_size = if self.effect == LyricEffect::Zoom && active {
                size * (1.0 + 0.22 * (through * std::f32::consts::PI).sin())
            } else {
                size
            };
            let box_rect = Rect::new(x, current.y, *width, current.h);
            let text_style = |colour: Color, size: f32| {
                TextStyle::new(size, colour)
                    .font(Font::Lyrics)
                    .centered()
                    .valign(VAlign::Middle)
                    // Lyrics sit over artwork and video, where an outline is the difference
                    // between readable and not.
                    .outlined(style.background.alpha(0.85), 2.0)
            };

            if self.effect == LyricEffect::Slide && active {
                // The syllable fills from the left as it is sung, so a long note shows how
                // much of it is left rather than only that it is happening.
                list.text(box_rect, &syllable.text, text_style(resting, drawn_size));
                list.clipped(
                    Rect::new(x, current.y, width * through, current.h),
                    |list| {
                        list.text(box_rect, &syllable.text, text_style(highlight, drawn_size));
                    },
                );
            } else {
                list.text(box_rect, &syllable.text, text_style(colour, drawn_size));
            }

            // The bouncing ball, which is the one everybody recognises: it arcs from syllable
            // to syllable and is over the one due now.
            if self.effect == LyricEffect::Ball && active {
                let radius = size * 0.16;
                let arc = (through * std::f32::consts::PI).sin();
                let ball = Rect::new(
                    x + width * through - radius,
                    current.y + current.h * 0.06 - arc * size * 0.35,
                    radius * 2.0,
                    radius * 2.0,
                );
                list.panel(ball, highlight, radius);
            }
        }

        // A count-in for a wait long enough that "has this stopped?" becomes a fair question.
        //
        // It counts to the bar setting off rather than to the first word, because the run-in
        // is itself the last couple of seconds of warning and counting through it would say
        // the same thing twice. It takes the strip the next line sits in: during a break there
        // is nothing to sing, so the line after the one you are waiting for is the least
        // useful thing on the screen, and a count squeezed in beside it is neither.
        let count = (self.beat_rate > 0.0)
            .then(|| {
                let sets_off = first.start - LEAD_IN_BEATS;
                countdown(
                    (sets_off - parked) / self.beat_rate,
                    (sets_off - beat) / self.beat_rate,
                )
            })
            .flatten();
        if let Some(count) = count {
            draw_countdown(list, upcoming, style, highlight, count);
        } else if !next_line.is_empty() {
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

    fn draw_pause(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
        self.pause_regions.clear();
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
            self.pause_regions.push(row);
            widgets.row(list, row, entry, "", index == self.pause_cursor);
        }
    }

    fn draw_results(&mut self, list: &mut DrawList, area: Rect, style: &Style) {
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
        self.results_card = Some(card);
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
