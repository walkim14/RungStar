//! Every setting the game has, and reading and writing them as TOML.
//!
//! This covers UltraStar Deluxe's `UIni.pas` surface. There, each option is an index into a
//! parallel array of display strings, and the ten options screens each hand-place their own
//! copy of the same widgets — so adding an option means touching an enum, two arrays, a
//! screen and every theme.
//!
//! Here an option is a typed value that knows its own name and its own list of choices
//! ([`Choice`]), so an options page is *derived* from the settings rather than written
//! alongside them, and a new option cannot be added without a label. See [`crate::options`].
//!
//! Unknown keys are kept rather than dropped, so a config written by a newer build survives a
//! downgrade — the usual way a player loses their settings.

use serde::{Deserialize, Serialize};

/// A setting that cycles through a fixed list of named values.
///
/// Almost every UltraStar option is one of these: left and right step through the choices and
/// there is no free text. Making that a trait means the options screen is one widget rather
/// than one per setting.
pub trait Choice: Sized + Copy + PartialEq + 'static {
    /// Every value, in the order they should cycle.
    const VALUES: &'static [Self];

    /// What to show the player.
    fn label(self) -> &'static str;

    /// Position in `VALUES`, or `0` for a value somehow not in it.
    fn position(self) -> usize {
        Self::VALUES.iter().position(|v| *v == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::VALUES[(self.position() + 1) % Self::VALUES.len()]
    }

    fn previous(self) -> Self {
        let len = Self::VALUES.len();
        Self::VALUES[(self.position() + len - 1) % len]
    }

    /// All the labels, for a settings page.
    fn labels() -> Vec<&'static str> {
        Self::VALUES.iter().map(|v| v.label()).collect()
    }
}

/// Define a cycling setting: an enum plus its labels, its default and its serialisation.
macro_rules! choice {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$variant_meta:meta])* $variant:ident = $label:literal ),+ $(,)?
        } default $default:ident
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $( $(#[$variant_meta])* $variant ),+ }

        impl Choice for $name {
            const VALUES: &'static [Self] = &[ $( Self::$variant ),+ ];
            fn label(self) -> &'static str {
                match self { $( Self::$variant => $label ),+ }
            }
        }

        impl Default for $name {
            fn default() -> Self { Self::$default }
        }
    };
}

choice! {
    /// How much pitch error still counts as a hit. Straight from UltraStar: Easy allows two
    /// semitones either way, Hard demands the exact pitch class.
    Difficulty { Easy = "Easy", Medium = "Medium", Hard = "Hard" } default Medium
}

choice! {
    /// Which pitch detector runs.
    ///
    /// Classic is bit-compatible with UltraStar Deluxe, so scores stay comparable with a
    /// friend's install. Enhanced is more accurate but will not agree to the point.
    Detector { Classic = "Classic", Enhanced = "Enhanced" } default Classic
}

choice! {
    /// Amplification applied to the microphone before analysis.
    MicBoost { Off = "Off", Plus6 = "+6 dB", Plus12 = "+12 dB", Plus18 = "+18 dB" } default Off
}

choice! {
    /// Whether the note lines are drawn behind the lyrics.
    NoteLines { Off = "Off", On = "On" } default On
}

choice! {
    /// The shape of the lyric text.
    LyricStyle { Regular = "Regular", Bold = "Bold", Outline = "Outline" } default Bold
}

choice! {
    /// How the syllable highlight moves along a line.
    LyricEffect {
        Simple = "Simple",
        Zoom = "Zoom",
        Slide = "Slide",
        Ball = "Ball",
        Shift = "Shift",
    } default Slide
}

choice! {
    /// Whether a metronome click plays over the song, and whether missed notes are audible.
    ClickAssist { Off = "Off", On = "On" } default Off
}

choice! {
    Switch { Off = "Off", On = "On" } default On
}

choice! {
    /// What a click or Confirm on an already-selected song does.
    OnSongClick {
        Sing = "Sing",
        SelectPlayers = "Select players",
        Menu = "Open menu",
    } default Sing
}

choice! {
    /// Whether the per-line bonus is awarded. Off makes a song worth 9000, not 10000.
    LineBonus { Off = "Off", On = "On" } default On
}

choice! {
    /// Window mode.
    ScreenMode {
        Windowed = "Windowed",
        Borderless = "Borderless",
        Fullscreen = "Fullscreen",
    } default Borderless
}

choice! {
    /// Upper bound on frame rate. Uncapped exists for testing; on a handheld it only costs
    /// battery, which is why it is not the default.
    FrameLimit {
        Sixty = "60",
        Ninety = "90",
        Hundred20 = "120",
        Hundred44 = "144",
        Uncapped = "Uncapped",
    } default Sixty
}

impl FrameLimit {
    /// Frames per second, or `None` when uncapped.
    pub fn fps(self) -> Option<u32> {
        match self {
            Self::Sixty => Some(60),
            Self::Ninety => Some(90),
            Self::Hundred20 => Some(120),
            Self::Hundred44 => Some(144),
            Self::Uncapped => None,
        }
    }
}

choice! {
    /// How the song video is fitted to the screen.
    ///
    /// UltraStar also offers a half-size option, which draws the video in a box in the middle.
    /// That made sense when a video was a feature to show off; as a background behind lyrics it
    /// is just a small picture with a wide border, and it is easy to land on by accident while
    /// cycling. An old `Half` setting is read as `Full`.
    VideoSize {
        #[serde(alias = "Half")]
        Full = "Fill the screen",
        Fit = "Fit, with bars",
    } default Full
}

choice! {
    /// Whether covers, videos and backgrounds are shown while browsing.
    ///
    /// Separate from playback because decoding a video for every song under the cursor is the
    /// one browsing cost that actually hurts on a Deck.
    Preview { Off = "Off", CoverOnly = "Cover only", Full = "Cover and video" } default Full
}

choice! {
    /// Where the song list gets its order.
    Tabs { Off = "Off", On = "On" } default Off
}

/// Window sizes offered, smallest first.
///
/// A list rather than two independent numbers: choosing 1281 by 799 is not a thing anybody
/// wants, and letting it be chosen means every combination has to work. These are the common
/// 16:9 and 16:10 sizes plus the Steam Deck's own, which is neither.
pub const RESOLUTIONS: [(u32, u32); 9] = [
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (2560, 1440),
    (3440, 1440),
    (3840, 2160),
];

/// The index of the resolution nearest `(width, height)`.
///
/// Nearest rather than exact, because the saved size can come from a display that is no
/// longer attached, and a settings row that shows nothing is worse than one showing the
/// closest thing.
pub fn nearest_resolution(width: u32, height: u32) -> usize {
    RESOLUTIONS
        .iter()
        .enumerate()
        .min_by_key(|(_, (w, h))| {
            let dw = w.abs_diff(width) as u64;
            let dh = h.abs_diff(height) as u64;
            dw * dw + dh * dh
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// Everything, grouped the way the options screens are.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub game: GameSettings,
    pub graphics: GraphicsSettings,
    pub sound: SoundSettings,
    pub lyrics: LyricSettings,
    pub appearance: AppearanceSettings,
    pub advanced: AdvancedSettings,
    pub network: NetworkSettings,
    /// Keys this build did not recognise, kept so a downgrade does not discard them.
    #[serde(flatten)]
    pub unknown: toml::Table,
}

/// Anything to do with the outside world.
///
/// The USDB username lives here; the password does not, and will not. It goes to the OS
/// keyring, because a settings file that quietly contains somebody's password is how it ends
/// up in a backup, a screenshot and a bug report — and it is a password people reuse.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkSettings {
    pub usdb_user: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    pub players: u8,
    pub difficulty: Difficulty,
    /// UI language code, e.g. `en`.
    pub language: String,
    pub tabs: Tabs,
    pub on_song_click: OnSongClick,
    pub line_bonus: LineBonus,
    /// Song directories. Empty means "the default one under the user's data directory".
    pub song_roots: Vec<String>,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            players: 1,
            difficulty: Difficulty::default(),
            language: "en".to_owned(),
            tabs: Tabs::default(),
            on_song_click: OnSongClick::default(),
            line_bonus: LineBonus::default(),
            song_roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsSettings {
    pub screen_mode: ScreenMode,
    pub width: u32,
    pub height: u32,
    pub frame_limit: FrameLimit,
    pub vsync: Switch,
    pub video_enabled: Switch,
    pub video_size: VideoSize,
    pub preview: Preview,
    /// Whether backgrounds and covers are shown behind the interface at all.
    pub backgrounds: Switch,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            screen_mode: ScreenMode::default(),
            // The Steam Deck's native size. On a desktop the window manager resizes it
            // immediately, and starting smaller than the display is recoverable where
            // starting larger is not.
            width: 1280,
            height: 800,
            frame_limit: FrameLimit::default(),
            vsync: Switch::On,
            video_enabled: Switch::On,
            video_size: VideoSize::default(),
            preview: Preview::default(),
            backgrounds: Switch::On,
        }
    }
}

/// One capture device and which singer each of its channels belongs to.
///
/// Stored by name rather than by index, because a device's position in the list moves when
/// anything else is plugged in — which is how a saved setup ends up pointing at a webcam.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MicAssignment {
    pub name: String,
    /// One entry per channel: `0` for off, otherwise a one-based singer number.
    pub channels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundSettings {
    pub mic_boost: MicBoost,
    /// Index into the eight-entry volume gate table. Below it, nothing scores.
    pub threshold: u8,
    pub click_assist: ClickAssist,
    pub beat_click: Switch,
    /// Hear yourself through the speakers. Off by default: with speakers rather than
    /// headphones it feeds back.
    pub passthrough: Switch,
    pub detector: Detector,
    /// Master, music and preview volumes, `0..=100`.
    pub master_volume: u8,
    pub preview_volume: u8,
    /// Play every song at the same loudness.
    ///
    /// On by default. A library assembled from a thousand uploads has fifteen decibels between
    /// its loudest and quietest songs, and reaching for the volume between every one of them
    /// is the job the game should be doing.
    pub even_volume: Switch,
    /// How loud the menu music is, `0..=100`. Zero turns it off.
    ///
    /// Well below the songs. It plays under everything and its job is to make an empty menu
    /// feel less empty, not to be listened to.
    pub music_volume: u8,
    /// How loud the interface sounds are, `0..=100`. Zero silences them.
    ///
    /// Below the music by default. A menu blip is heard hundreds of times an hour and the
    /// music is the thing anybody came for.
    pub effects_volume: u8,
    /// Seconds a preview takes to fade in. Zero cuts straight in.
    pub preview_fade: f32,
    /// Milliseconds the microphone lags the speakers. The scoring clock is shifted by this.
    pub mic_delay_ms: u32,
    /// Milliseconds the audio lags the picture, for a display with its own processing lag.
    pub av_delay_ms: i32,
    /// Which singer each microphone channel feeds. Empty means "work it out".
    pub microphones: Vec<MicAssignment>,
    /// Assign each channel of a microphone separately.
    ///
    /// Off by default: almost every USB microphone reports two channels and is mono on both,
    /// so splitting shows two rows for one microphone and invites putting two singers on it.
    /// On for the dual-USB karaoke sets where left and right really are two microphones.
    pub split_channels: Switch,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            mic_boost: MicBoost::default(),
            threshold: 1,
            click_assist: ClickAssist::default(),
            beat_click: Switch::Off,
            passthrough: Switch::Off,
            detector: Detector::default(),
            master_volume: 80,
            preview_volume: 60,
            effects_volume: 45,
            even_volume: Switch::On,
            music_volume: 30,
            preview_fade: 0.4,
            // UltraStar's own default, and close to what a USB microphone plus a desktop
            // audio stack actually costs.
            mic_delay_ms: 140,
            av_delay_ms: 0,
            microphones: Vec::new(),
            split_channels: Switch::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricSettings {
    pub style: LyricStyle,
    pub effect: LyricEffect,
    pub note_lines: NoteLines,
    /// Show the upcoming line as well as the current one.
    pub show_next_line: Switch,
}

impl Default for LyricSettings {
    fn default() -> Self {
        Self {
            style: LyricStyle::default(),
            effect: LyricEffect::default(),
            note_lines: NoteLines::default(),
            show_next_line: Switch::On,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub theme: String,
    pub skin: String,
    pub accent: String,
    /// Multiplies every text size. For a Deck held at arm's length, and for accessibility.
    pub text_scale: f32,
    pub browse_layout: crate::browse::Layout,
    pub menu_music: Switch,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "Rung".to_owned(),
            skin: "dark".to_owned(),
            accent: "magenta".to_owned(),
            text_scale: 1.0,
            browse_layout: crate::browse::Layout::default(),
            menu_music: Switch::On,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedSettings {
    pub screen_fade: Switch,
    pub confirm_delete: Switch,
    /// Show a running score during the song rather than only at the end.
    pub live_scores: Switch,
    /// Rumble on golden notes and line bonuses.
    pub rumble: Switch,
    /// Draw the input diagnostics panel over the sing screen.
    pub input_panel: Switch,
    pub show_fps: Switch,
    /// What the on-screen hints call the controller's buttons.
    pub glyphs: crate::glyphs::Glyphs,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            screen_fade: Switch::On,
            confirm_delete: Switch::On,
            live_scores: Switch::On,
            rumble: Switch::On,
            input_panel: Switch::Off,
            show_fps: Switch::Off,
            glyphs: crate::glyphs::Glyphs::default(),
        }
    }
}

/// Why settings could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("reading settings from `{path}`: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("writing settings to `{path}`: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing settings: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("serialising settings: {0}")]
    Serialise(#[from] toml::ser::Error),
}

/// The eight microphone gate levels, as a fraction of full scale. UltraStar's own table.
pub const THRESHOLDS: [f32; 8] = [0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.40, 0.60];

/// The most singers the game will run. UltraStar's source calls its own 8- and 12-player
/// paths untested, and six is what the hardware realistically supports.
pub const MAX_PLAYERS: u8 = 6;

impl Settings {
    /// Read from a file, falling back to defaults when it does not exist.
    ///
    /// A missing config is the normal first-run case, not an error. A *corrupt* one is an
    /// error, because silently resetting someone's settings is worse than telling them.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, SettingsError> {
        let path = path.as_ref();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut settings: Self = toml::from_str(&text)?;
                settings.clamp();
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(SettingsError::Read {
                path: path.display().to_string(),
                source,
            }),
        }
    }

    /// Write to a file, creating the directory if needed.
    ///
    /// Writes to a temporary file and renames, so a crash or a full disk during the write
    /// cannot leave a half-written config that fails to parse on the next launch.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), SettingsError> {
        let path = path.as_ref();
        let text = toml::to_string_pretty(self)?;
        let fail = |source| SettingsError::Write {
            path: path.display().to_string(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(fail)?;
        }
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, text).map_err(fail)?;
        std::fs::rename(&temporary, path).map_err(fail)
    }

    /// Force every value into a range the rest of the game can rely on.
    ///
    /// Config files get hand-edited, and a zero here becomes a division by zero three crates
    /// away. Clamping on load means nothing downstream has to check.
    pub fn clamp(&mut self) {
        self.game.players = self.game.players.clamp(1, MAX_PLAYERS);
        self.graphics.width = self.graphics.width.clamp(640, 7680);
        self.graphics.height = self.graphics.height.clamp(480, 4320);
        self.sound.threshold = self.sound.threshold.min(THRESHOLDS.len() as u8 - 1);
        self.sound.master_volume = self.sound.master_volume.min(100);
        self.sound.preview_volume = self.sound.preview_volume.min(100);
        self.sound.preview_fade = self.sound.preview_fade.clamp(0.0, 5.0);
        self.sound.mic_delay_ms = self.sound.mic_delay_ms.min(1000);
        self.sound.av_delay_ms = self.sound.av_delay_ms.clamp(-1000, 1000);
        self.appearance.text_scale = self.appearance.text_scale.clamp(0.7, 1.6);
        if self.game.language.is_empty() {
            self.game.language = "en".to_owned();
        }
    }

    /// The microphone gate as a fraction of full scale.
    pub fn threshold(&self) -> f32 {
        THRESHOLDS[(self.sound.threshold as usize).min(THRESHOLDS.len() - 1)]
    }

    /// Whether a clip should play under the browser cursor.
    pub fn preview_enabled(&self) -> bool {
        self.graphics.preview != Preview::Off && self.sound.preview_volume > 0
    }

    /// Pitch tolerance in semitones for the chosen difficulty. Easy 2, Medium 1, Hard 0.
    pub fn tolerance(&self) -> i32 {
        match self.game.difficulty {
            Difficulty::Easy => 2,
            Difficulty::Medium => 1,
            Difficulty::Hard => 0,
        }
    }
}
