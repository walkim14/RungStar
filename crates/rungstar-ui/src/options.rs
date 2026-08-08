//! Options pages, derived from [`Settings`] rather than written beside them.
//!
//! UltraStar Deluxe has ten options screens, each one hand-placed in every theme, each one
//! holding its own copy of the widget code and its own parallel array of display strings. An
//! option that is added and not wired into all of that simply does not appear.
//!
//! Here a page is a list of [`Item`]s that each know how to read, label and step one setting.
//! One screen renders every page, an option cannot exist without a label and a help string,
//! and a test can walk every item on every page and check it round-trips — which is what
//! catches the copy-paste error where two options end up editing the same field.

use crate::browse::Layout;
use crate::settings::{
    AppearanceSettings, Choice, ClickAssist, Detector, Difficulty, FrameLimit, LineBonus,
    LyricEffect, LyricStyle, MicBoost, NoteLines, OnSongClick, Preview, ScreenMode, Settings,
    Switch, Tabs, VideoSize, MAX_PLAYERS, THRESHOLDS,
};

/// Something on a page that is not a setting: a button the screen has to interpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Re-read every song file, ignoring the index.
    RescanLibrary,
    /// Forget the index entirely and build it again.
    RebuildIndex,
    AddSongFolder,
    ManageMicrophones,
    RebindControls,
    ResetToDefaults,
}

/// How one item is edited.
pub enum Control {
    /// Left and right step through a fixed list.
    Choice {
        get: fn(&Settings) -> usize,
        labels: fn() -> Vec<&'static str>,
        step: fn(&mut Settings, isize),
    },
    /// Left and right move a number within a range.
    Number {
        get: fn(&Settings) -> f64,
        set: fn(&mut Settings, f64),
        min: f64,
        max: f64,
        step: f64,
        format: fn(f64) -> String,
    },
    /// Free text, edited with the on-screen keyboard.
    Text { get: fn(&Settings) -> String },
    /// A button.
    Button(Action),
}

/// One row on an options page.
pub struct Item {
    pub label: &'static str,
    /// Shown under the page while this row is selected. Every option gets one: an option a
    /// player cannot understand is an option they will not touch.
    pub help: &'static str,
    pub control: Control,
}

impl Item {
    /// What the value column should say right now.
    pub fn value(&self, settings: &Settings) -> String {
        match &self.control {
            Control::Choice { get, labels, .. } => labels()
                .get(get(settings))
                .copied()
                .unwrap_or("?")
                .to_owned(),
            Control::Number { get, format, .. } => format(get(settings)),
            Control::Text { get } => get(settings),
            Control::Button(_) => String::new(),
        }
    }

    /// Whether left/right do anything, or whether this row is pressed instead.
    pub fn is_button(&self) -> bool {
        matches!(self.control, Control::Button(_))
    }

    /// Step the value. `direction` is `-1` for left, `1` for right.
    pub fn adjust(&self, settings: &mut Settings, direction: isize) {
        match &self.control {
            Control::Choice { step, .. } => step(settings, direction),
            Control::Number {
                get,
                set,
                min,
                max,
                step,
                ..
            } => {
                let next = get(settings) + *step * direction as f64;
                // Numbers clamp rather than wrap. A volume that jumps from 100 to 0 because
                // the stick was held a moment too long is a genuinely bad surprise.
                set(settings, next.clamp(*min, *max));
            }
            Control::Text { .. } | Control::Button(_) => {}
        }
    }

    /// Progress through the range, `0.0..=1.0`, for drawing a slider. `None` when the item is
    /// not a number.
    pub fn fraction(&self, settings: &Settings) -> Option<f32> {
        match &self.control {
            Control::Number { get, min, max, .. } if max > min => {
                // Clamped, because a value stored as `f32` and stepped back through `f64`
                // lands a hair outside its own range, and a slider drawn from that spills
                // past the end of its track.
                Some((((get(settings) - min) / (max - min)) as f32).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

/// A named group of items.
pub struct Page {
    pub title: &'static str,
    pub items: Vec<Item>,
}

/// Build a [`Control::Choice`] for a field whose type implements [`Choice`].
macro_rules! choice_item {
    ($ty:ty, $($field:ident).+, $label:literal, $help:literal) => {
        Item {
            label: $label,
            help: $help,
            control: Control::Choice {
                get: |s: &Settings| Choice::position(s.$($field).+),
                labels: || <$ty as Choice>::labels(),
                step: |s: &mut Settings, d: isize| {
                    let current: $ty = s.$($field).+;
                    s.$($field).+ = if d >= 0 {
                        Choice::next(current)
                    } else {
                        Choice::previous(current)
                    };
                },
            },
        }
    };
}

/// Build a [`Control::Number`] for a numeric field, converting through `f64`.
macro_rules! number_item {
    (
        $($field:ident).+ as $ty:ty, $label:literal, $help:literal,
        $min:expr, $max:expr, $step:expr, $format:expr
    ) => {
        Item {
            label: $label,
            help: $help,
            control: Control::Number {
                get: |s: &Settings| s.$($field).+ as f64,
                set: |s: &mut Settings, v: f64| { s.$($field).+ = v.round() as $ty; },
                min: $min,
                max: $max,
                step: $step,
                format: $format,
            },
        }
    };
}

fn plain(v: f64) -> String {
    format!("{v:.0}")
}

fn percent(v: f64) -> String {
    format!("{v:.0}%")
}

fn milliseconds(v: f64) -> String {
    format!("{v:.0} ms")
}

fn seconds(v: f64) -> String {
    format!("{v:.1} s")
}

/// The microphone gate, shown as the level it actually corresponds to.
///
/// UltraStar shows this as an index from one to eight, which tells a player nothing about why
/// their quiet singing is not scoring.
fn threshold_label(v: f64) -> String {
    let index = (v as usize).min(THRESHOLDS.len() - 1);
    format!("{:.0}%", THRESHOLDS[index] * 100.0)
}

impl Page {
    /// Every page, in the order the options hub lists them.
    pub fn all() -> Vec<Page> {
        vec![
            Self::game(),
            Self::graphics(),
            Self::sound(),
            Self::lyrics(),
            Self::appearance(),
            Self::advanced(),
        ]
    }

    pub fn game() -> Page {
        Page {
            title: "Game",
            items: vec![
                number_item!(
                    game.players as u8,
                    "Singers",
                    "How many people are singing. Each needs a microphone, or a channel of one.",
                    1.0,
                    MAX_PLAYERS as f64,
                    1.0,
                    plain
                ),
                choice_item!(
                    Difficulty,
                    game.difficulty,
                    "Difficulty",
                    "How far off the note you may be and still score. Easy allows two \
                     semitones, Hard demands the exact one."
                ),
                choice_item!(
                    LineBonus,
                    game.line_bonus,
                    "Line bonus",
                    "Award up to 1000 points for singing whole lines cleanly. Off caps a \
                     song at 9000."
                ),
                choice_item!(
                    OnSongClick,
                    game.on_song_click,
                    "Selecting a song",
                    "What happens when you confirm the song already under the cursor."
                ),
                choice_item!(
                    Tabs,
                    game.tabs,
                    "Category tabs",
                    "Group the song list by its sort key instead of showing one flat list."
                ),
                Item {
                    label: "Song folders",
                    help: "Where your songs live. Any folder containing a .txt file counts \
                           as a song; there is no naming convention to follow.",
                    control: Control::Text {
                        get: |s| {
                            if s.game.song_roots.is_empty() {
                                "Default folder".to_owned()
                            } else {
                                s.game.song_roots.join(", ")
                            }
                        },
                    },
                },
                Item {
                    label: "Add a song folder",
                    help: "Choose another folder to search for songs.",
                    control: Control::Button(Action::AddSongFolder),
                },
                Item {
                    label: "Rescan songs",
                    help: "Look for songs that were added or changed. Only files whose size \
                           or timestamp moved are read again.",
                    control: Control::Button(Action::RescanLibrary),
                },
                Item {
                    label: "Rebuild the index",
                    help: "Read every song from scratch. Slow, and only needed if the list \
                           looks wrong.",
                    control: Control::Button(Action::RebuildIndex),
                },
            ],
        }
    }

    pub fn graphics() -> Page {
        Page {
            title: "Graphics",
            items: vec![
                choice_item!(
                    ScreenMode,
                    graphics.screen_mode,
                    "Window",
                    "Windowed, borderless or exclusive fullscreen."
                ),
                Item {
                    label: "Resolution",
                    help: "Window size. Ignored in fullscreen, which uses whatever the                            display is. A list of sizes rather than two independent numbers,                            because nobody wants 1281 by 799 and offering it means it has to                            work.",
                    control: Control::Choice {
                        get: |s: &Settings| {
                            crate::settings::nearest_resolution(
                                s.graphics.width,
                                s.graphics.height,
                            )
                        },
                        labels: || {
                            crate::settings::RESOLUTIONS
                                .iter()
                                .map(|(w, h)| -> &'static str {
                                    // Leaked, so the labels are `&'static str` like every
                                    // other choice's. Nine strings, once, for the process.
                                    Box::leak(format!("{w} x {h}").into_boxed_str())
                                })
                                .collect()
                        },
                        step: |s: &mut Settings, d: isize| {
                            let list = crate::settings::RESOLUTIONS;
                            let current = crate::settings::nearest_resolution(
                                s.graphics.width,
                                s.graphics.height,
                            );
                            let next =
                                (current as isize + d).rem_euclid(list.len() as isize) as usize;
                            s.graphics.width = list[next].0;
                            s.graphics.height = list[next].1;
                        },
                    },
                },
                choice_item!(
                    FrameLimit,
                    graphics.frame_limit,
                    "Frame limit",
                    "Capping the frame rate costs nothing visible and saves a lot of battery \
                     on a handheld."
                ),
                choice_item!(
                    Switch,
                    graphics.vsync,
                    "Vertical sync",
                    "Avoid tearing by waiting for the display."
                ),
                choice_item!(
                    Switch,
                    graphics.video_enabled,
                    "Song videos",
                    "Play the song's video behind the lyrics. Not implemented yet \u{2014} the \n                     decoder is not wired up, so this currently changes nothing."
                ),
                choice_item!(
                    VideoSize,
                    graphics.video_size,
                    "Video size",
                    "How much of the screen the video fills. Takes effect once video playback \n                     is implemented."
                ),
                choice_item!(
                    Preview,
                    graphics.preview,
                    "Browsing preview",
                    "Show artwork, and optionally video, for the song under the cursor. \
                     Decoding video while browsing is the one cost that shows on a Deck."
                ),
                choice_item!(
                    Switch,
                    graphics.backgrounds,
                    "Backgrounds",
                    "Show song artwork behind the interface."
                ),
            ],
        }
    }

    pub fn sound() -> Page {
        Page {
            title: "Sound",
            items: vec![
                number_item!(
                    sound.master_volume as u8, "Volume",
                    "How loud the song plays. Singing is scored from the microphone, so this \n                     changes what you hear and not what you score.",
                    0.0, 100.0, 5.0, percent
                ),
                choice_item!(MicBoost, sound.mic_boost, "Microphone boost", "Amplify a quiet microphone before it is analysed."),
                Item {
                    label: "Silence gate",
                    help: "How loud you must be before anything is scored. Raise it if noise \
                           is scoring for you; lower it if quiet singing is not.",
                    control: Control::Number {
                        get: |s| s.sound.threshold as f64,
                        set: |s, v| s.sound.threshold = v.round() as u8,
                        min: 0.0,
                        max: THRESHOLDS.len() as f64 - 1.0,
                        step: 1.0,
                        format: threshold_label,
                    },
                },
                choice_item!(
                    Detector, sound.detector, "Pitch detection",
                    "Classic matches UltraStar Deluxe exactly, so scores stay comparable. \
                     Enhanced is more accurate but will not agree to the point."
                ),
                number_item!(
                    sound.mic_delay_ms as u32, "Microphone delay",
                    "How far your microphone lags the music. The scoring clock is shifted by \
                     this, so getting it wrong shifts every hit.",
                    0.0, 500.0, 10.0, milliseconds
                ),
                number_item!(
                    sound.av_delay_ms as i32, "Audio delay",
                    "For a television that processes the picture and arrives late.",
                    -500.0, 500.0, 10.0, milliseconds
                ),
                choice_item!(Switch, sound.passthrough, "Hear yourself", "Play the microphone back through the speakers. Feeds back without headphones."),
                choice_item!(ClickAssist, sound.click_assist, "Click assist", "A click on every beat, to help you find the timing."),
                choice_item!(Switch, sound.beat_click, "Beat click", "A click on each note as it should be sung."),
                number_item!(
                    sound.preview_volume as u8, "Preview volume", "Loudness of the clip that plays while browsing.",
                    0.0, 100.0, 5.0, percent
                ),
                Item {
                    label: "Preview fade",
                    help: "How long a preview takes to fade in.",
                    control: Control::Number {
                        get: |s| s.sound.preview_fade as f64,
                        set: |s, v| s.sound.preview_fade = v as f32,
                        min: 0.0,
                        max: 3.0,
                        step: 0.1,
                        format: seconds,
                    },
                },
                Item {
                    label: "Microphones",
                    help: "Which device each singer uses. One stereo device can carry two \
                           singers, one per channel.",
                    control: Control::Button(Action::ManageMicrophones),
                },
            ],
        }
    }

    pub fn lyrics() -> Page {
        Page {
            title: "Lyrics",
            items: vec![
                choice_item!(
                    LyricStyle,
                    lyrics.style,
                    "Style",
                    "How the lyric text is drawn: plain, heavy, or outlined for video backgrounds."
                ),
                choice_item!(
                    LyricEffect,
                    lyrics.effect,
                    "Highlight",
                    "How the highlight moves along the line as you sing."
                ),
                choice_item!(
                    NoteLines,
                    lyrics.note_lines,
                    "Note lines",
                    "Draw the pitch staff behind the notes."
                ),
                choice_item!(
                    Switch,
                    lyrics.show_next_line,
                    "Show next line",
                    "Show the upcoming line as well as the current one."
                ),
            ],
        }
    }

    pub fn appearance() -> Page {
        Page {
            title: "Appearance",
            items: vec![
                Item {
                    label: "Theme",
                    help: "Which installed theme supplies the colours and fonts.",
                    control: Control::Text {
                        get: |s: &Settings| s.appearance.theme.clone(),
                    },
                },
                Item {
                    label: "Skin",
                    help:
                        "Dark, light, or high contrast. High contrast suits a screen in daylight.",
                    control: Control::Text {
                        get: |s: &Settings| s.appearance.skin.clone(),
                    },
                },
                Item {
                    label: "Accent colour",
                    help: "The highlight colour. Text drawn on it is chosen for contrast, so \
                           any accent stays readable.",
                    control: Control::Text {
                        get: |s: &Settings| s.appearance.accent.clone(),
                    },
                },
                choice_item!(
                    Layout,
                    appearance.browse_layout,
                    "Song list",
                    "Whether songs appear as a list, a grid of covers, or a spinning carousel."
                ),
                Item {
                    label: "Text size",
                    help: "Scales every label. Larger helps at arm's length on a handheld.",
                    control: Control::Number {
                        get: |s| s.appearance.text_scale as f64,
                        set: |s, v| s.appearance.text_scale = v as f32,
                        min: 0.7,
                        max: 1.6,
                        step: 0.05,
                        format: |v| format!("{:.0}%", v * 100.0),
                    },
                },
                choice_item!(
                    Switch,
                    appearance.menu_music,
                    "Menu music",
                    "Play background music while browsing songs and in the menus."
                ),
            ],
        }
    }

    pub fn advanced() -> Page {
        Page {
            title: "Advanced",
            items: vec![
                choice_item!(
                    Switch,
                    advanced.live_scores,
                    "Live scores",
                    "Show each singer's score during the song."
                ),
                choice_item!(
                    Switch,
                    advanced.rumble,
                    "Rumble",
                    "Vibrate the controller on golden notes and line bonuses."
                ),
                choice_item!(
                    Switch,
                    advanced.screen_fade,
                    "Screen fades",
                    "Cross-fade when moving between screens. Turn it off if you prefer instant."
                ),
                choice_item!(
                    Switch,
                    advanced.confirm_delete,
                    "Confirm deletions",
                    "Ask before deleting a song or a profile."
                ),
                choice_item!(
                    Switch,
                    advanced.input_panel,
                    "Microphone monitor",
                    "Show input level and detected pitch while singing. Turn this on when a \
                     microphone is not scoring and you cannot tell why."
                ),
                choice_item!(
                    Switch,
                    advanced.show_fps,
                    "Show frame rate",
                    "Draw the frame rate in the corner."
                ),
                Item {
                    label: "Controls",
                    help: "Rebind the keyboard and controller.",
                    control: Control::Button(Action::RebindControls),
                },
                Item {
                    label: "Reset everything",
                    help: "Put every setting back to how it shipped.",
                    control: Control::Button(Action::ResetToDefaults),
                },
            ],
        }
    }
}

/// `AppearanceSettings` is edited by pickers rather than by stepping, because the choices come
/// from whatever themes are installed rather than from a fixed list.
impl AppearanceSettings {
    pub fn set_theme(&mut self, theme: impl Into<String>) {
        self.theme = theme.into();
    }
    pub fn set_skin(&mut self, skin: impl Into<String>) {
        self.skin = skin.into();
    }
    pub fn set_accent(&mut self, accent: impl Into<String>) {
        self.accent = accent.into();
    }
}
