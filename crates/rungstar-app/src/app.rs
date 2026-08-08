//! RungStar: the game.
//!
//! Owns the window, the screen stack and the library, and does the three things a screen
//! cannot do for itself — run a query, load a cover, save the settings. Screens are pure state
//! that produce a display list; this file is where that meets a device.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;

use rungstar_audio::AudioClip;
use rungstar_library::{
    scan_with_progress, Database, Progress, ScanOptions, SearchQuery, SongEntry,
};
use rungstar_platform::font::FontSet;
use rungstar_platform::render::Renderer;
use rungstar_platform::{Playback, SdlCapture};
use rungstar_ui::draw::{DrawList, ImageId, TextStyle};
use rungstar_ui::geom::Rect;
use rungstar_ui::menus::{MainMenu, OptionsOutcome, OptionsScreen};
use rungstar_ui::micscreen::{MicOutcome, MicScreen};
use rungstar_ui::options::Action;
use rungstar_ui::screen::{Route, Transition, Widgets};
use rungstar_ui::settings::{ScreenMode, Settings, Switch};
use rungstar_ui::singscreen::{Overlay, PauseChoice, SingScreen};
use rungstar_ui::songselect::{Input, SongAction, SongSelect};
use rungstar_ui::theme::{Style, Theme};
use rungstar_ui::Color;

mod session;

mod paths {
    use std::path::PathBuf;

    /// Where settings and the song index live.
    ///
    /// A directory beside the executable wins when it is writable, so a copy on a USB stick
    /// stays self-contained — the way UltraStar's portable mode works, and the way it should
    /// behave on a Steam Deck where the game may live on an SD card.
    pub fn data_directory() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let portable = dir.join("rungstar-data");
                if portable.is_dir() {
                    return portable;
                }
            }
        }
        let base = if cfg!(windows) {
            std::env::var("APPDATA").ok().map(PathBuf::from)
        } else {
            std::env::var("XDG_DATA_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join(".local/share"))
                })
        };
        base.unwrap_or_else(|| PathBuf::from(".")).join("rungstar")
    }
}

/// Which screen is on top.
enum Screen {
    Main(MainMenu),
    Songs(Box<SongSelect>),
    Options(Box<OptionsScreen>),
    /// Singing. The session owns the devices; the screen only draws.
    Sing(Box<SingScreen>, Box<session::Session>),
    /// Microphone setup, with capture running so the meters are live.
    Mics(Box<MicScreen>, Box<session::Monitor>),
    About,
}

/// Everything the running game holds.
struct App {
    settings: Settings,
    theme: Theme,
    style: Style,
    library: Database,
    stack: Vec<Screen>,
    covers: CoverCache,
    data_dir: PathBuf,
    /// Set when a scan or a query has changed what the browser should be showing.
    status: String,
    /// A song the browser asked for, waiting for the frame loop to open the devices.
    pending_sing: Option<i64>,
    /// Set when the microphone screen has been asked for.
    pending_mics: bool,
    /// Set when something changed that the window or the audio has to be told about.
    settings_dirty: bool,
    /// A scan running on another thread, and the last progress it reported.
    scan: Option<ScanJob>,
    /// The clip playing under the browser cursor.
    preview: Option<Preview>,
    running: bool,
}

/// A snatch of the song under the cursor.
///
/// Held back by a delay, because starting a clip for every song a fast scroll passes over
/// would be a stutter of half-second fragments rather than a preview.
struct Preview {
    song: i64,
    playback: Option<Playback>,
    started: Instant,
    /// Where in the song the clip was seeked to.
    ///
    /// Needed because `position()` reports the absolute point in the song, and a preview
    /// starts a quarter of the way in — so "how long has this been playing" is the difference,
    /// not the position. Comparing the position itself against the preview length faded every
    /// preview out on its first frame.
    from: f64,
    /// Set when opening failed, so it is not attempted again every frame.
    failed: bool,
}

/// How long the cursor must rest on a song before its preview starts.
const PREVIEW_DELAY: std::time::Duration = std::time::Duration::from_millis(450);

/// How long a preview plays before it fades out, in seconds.
const PREVIEW_LENGTH: f32 = 30.0;

/// A scan running off the main thread.
///
/// A first scan of a real library takes long enough that doing it inline freezes the window --
/// eight thousand songs is fourteen seconds on a cold file cache, and a frozen window is
/// indistinguishable from a crash. The scan writes through its own connection; the index is in
/// WAL mode, so the browser keeps reading while it does.
struct ScanJob {
    progress: std::sync::mpsc::Receiver<Progress>,
    handle: Option<std::thread::JoinHandle<Result<usize, String>>>,
    latest: Progress,
    started: Instant,
}

/// Covers loaded on demand, with a bound on how many are kept.
///
/// Browsing thirty thousand songs would otherwise load thirty thousand textures. The bound is
/// generous enough that scrolling back a page never re-reads, and small enough to be a fixed
/// cost rather than a leak.
struct CoverCache {
    loaded: HashMap<i64, Option<ImageId>>,
    order: Vec<i64>,
    limit: usize,
}

impl CoverCache {
    fn new() -> Self {
        Self {
            loaded: HashMap::new(),
            order: Vec::new(),
            limit: 96,
        }
    }

    fn get(&self, id: i64) -> Option<ImageId> {
        self.loaded.get(&id).copied().flatten()
    }

    fn knows(&self, id: i64) -> bool {
        self.loaded.contains_key(&id)
    }

    /// Record a result, evicting the oldest when the cache is full.
    fn insert(&mut self, id: i64, image: Option<ImageId>, renderer: &mut Renderer) {
        if self.loaded.insert(id, image).is_none() {
            self.order.push(id);
        }
        while self.order.len() > self.limit {
            let oldest = self.order.remove(0);
            if let Some(Some(texture)) = self.loaded.remove(&oldest) {
                renderer.drop_image(texture);
            }
        }
    }
}

/// Read a cover image from disk into a texture.
///
/// Only the formats a song folder actually contains are handled, and a failure is cached as
/// "no cover" so a broken file is not re-read on every frame.
fn load_cover(song: &SongEntry, renderer: &mut Renderer) -> Option<ImageId> {
    let directory = song.directory()?;
    let file = song.cover_file.as_ref()?;
    let path = directory.join(file);
    let reader = image::ImageReader::open(&path).ok()?;
    let decoded = reader.with_guessed_format().ok()?.decode().ok()?;
    // Downscale before uploading: a cover is drawn at a few hundred pixels and libraries are
    // full of 1000x1000 scans, which would be sixty times the texture memory for no gain.
    let decoded = decoded.thumbnail(512, 512);
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    renderer.add_image(width, height, rgba.as_raw()).ok()
}

impl App {
    fn new(data_dir: PathBuf) -> Result<Self> {
        let mut settings =
            Settings::load(data_dir.join("settings.toml")).context("reading settings")?;
        settings.clamp();

        let theme = load_theme(&data_dir, &settings);
        let style = theme.resolve(&settings.appearance.skin, &settings.appearance.accent);

        std::fs::create_dir_all(&data_dir).context("creating the data directory")?;
        let library = Database::open(data_dir.join("library.db")).context("opening the index")?;

        Ok(Self {
            settings,
            theme,
            style,
            library,
            stack: vec![Screen::Main(MainMenu::new())],
            covers: CoverCache::new(),
            data_dir,
            status: String::new(),
            pending_sing: None,
            pending_mics: false,
            settings_dirty: true,
            scan: None,
            preview: None,
            running: true,
        })
    }

    fn song_roots(&self) -> Vec<PathBuf> {
        if self.settings.game.song_roots.is_empty() {
            vec![self.data_dir.join("songs")]
        } else {
            self.settings
                .game
                .song_roots
                .iter()
                .map(PathBuf::from)
                .collect()
        }
    }

    /// Bring the index in line with the disk, on this thread.
    ///
    /// Used by `--check`, where blocking is the point. Everything interactive uses
    /// [`App::start_scan`].
    fn rescan(&mut self, verify: bool) {
        let roots = self.song_roots();
        for root in &roots {
            let _ = std::fs::create_dir_all(root);
        }
        let mut options = ScanOptions::new(roots);
        options.verify = verify;
        let started = Instant::now();
        match scan_with_progress(&mut self.library, &options, |_| {}) {
            Ok(report) => {
                self.status = format!(
                    "{} songs, scanned in {:.1} s",
                    report.total_indexed(),
                    started.elapsed().as_secs_f32()
                );
            }
            Err(error) => self.status = format!("scan failed: {error}"),
        }
    }

    /// Start a scan on another thread.
    fn start_scan(&mut self, verify: bool) {
        if self.scan.is_some() {
            return;
        }
        let roots = self.song_roots();
        for root in &roots {
            let _ = std::fs::create_dir_all(root);
        }
        let mut options = ScanOptions::new(roots);
        options.verify = verify;
        let database_path = self.data_dir.join("library.db");

        let (sender, progress) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut database = Database::open(&database_path).map_err(|e| e.to_string())?;
            let report = scan_with_progress(&mut database, &options, move |p| {
                // A closed receiver means the game is shutting down: there is nothing to
                // report to and nothing to do about it.
                let _ = sender.send(p);
            })
            .map_err(|e| e.to_string())?;
            Ok(report.total_indexed())
        });

        self.scan = Some(ScanJob {
            progress,
            handle: Some(handle),
            latest: Progress::default(),
            started: Instant::now(),
        });
        self.status = "looking for songs\u{2026}".to_owned();
    }

    /// Collect progress, and pick up the result when the scan has finished.
    fn poll_scan(&mut self) {
        let Some(job) = &mut self.scan else {
            return;
        };
        while let Ok(progress) = job.progress.try_recv() {
            job.latest = progress;
        }
        let finished = job
            .handle
            .as_ref()
            .map(std::thread::JoinHandle::is_finished)
            .unwrap_or(true);
        if !finished {
            self.status = match job.latest.fraction() {
                Some(fraction) => format!(
                    "reading songs\u{2026} {}%  ({} of {})",
                    (fraction * 100.0).round(),
                    job.latest.done,
                    job.latest.total
                ),
                None => "looking for songs\u{2026}".to_owned(),
            };
            return;
        }

        let elapsed = job.started.elapsed().as_secs_f32();
        let outcome = job.handle.take().map(std::thread::JoinHandle::join);
        self.scan = None;
        self.status = match outcome {
            Some(Ok(Ok(count))) => format!("{count} songs, scanned in {elapsed:.1} s"),
            Some(Ok(Err(error))) => format!("scan failed: {error}"),
            _ => "the scan stopped unexpectedly".to_owned(),
        };
        // The rows are new, so whatever the browser is showing is stale.
        if let Some(Screen::Songs(songs)) = self.stack.last_mut() {
            songs.invalidate();
        }
    }

    /// Keep the preview in step with the browser cursor.
    ///
    /// Started only after the cursor has rested, so scrolling past a hundred songs plays none
    /// of them. Stopped the moment the browser is not on top, so a preview never talks over
    /// the song being sung.
    fn update_preview(&mut self, audio: &sdl3::AudioSubsystem) {
        let wanted = match self.stack.last() {
            Some(Screen::Songs(songs)) if self.settings.preview_enabled() => {
                songs.selected().map(|s| s.id)
            }
            _ => None,
        };

        let current = self.preview.as_ref().map(|p| p.song);
        if current != wanted {
            // A different song, or none. Drop whatever was playing and start the timer again.
            self.preview = wanted.map(|song| Preview {
                song,
                playback: None,
                started: Instant::now(),
                from: 0.0,
                failed: false,
            });
            return;
        }
        let Some(song) = wanted else {
            return;
        };

        // Read before borrowing the preview mutably: both want `self`.
        let volume = self.preview_volume();
        let due = self.preview.as_ref().is_some_and(|p| {
            p.playback.is_none() && !p.failed && p.started.elapsed() >= PREVIEW_DELAY
        });
        if due {
            match self.open_preview(audio, song) {
                Ok((playback, from)) => {
                    if let Some(preview) = &mut self.preview {
                        preview.from = from;
                        preview.playback = Some(playback);
                    }
                }
                Err(reason) => {
                    // Marked as tried, so a song that cannot preview is not retried on every
                    // frame for as long as the cursor sits on it.
                    if let Some(preview) = &mut self.preview {
                        preview.failed = true;
                    }
                    tracing::debug!("no preview for song {song}: {reason}");
                }
            }
            return;
        }

        let from = self.preview.as_ref().map(|p| p.from).unwrap_or(0.0);
        if let Some(playback) = self.preview.as_mut().and_then(|p| p.playback.as_mut()) {
            let _ = playback.pump();
            // Fade out at the end rather than cutting, and never restart: a preview that
            // loops under a cursor left resting is maddening. Measured from where the clip
            // was seeked to, not from the start of the song.
            let played = (playback.position() - from) as f32;
            if played > PREVIEW_LENGTH {
                let fade = (1.0 - (played - PREVIEW_LENGTH) / 1.5).clamp(0.0, 1.0);
                playback.set_volume(volume * fade);
                if fade <= 0.0 {
                    let _ = playback.pause();
                }
            }
        }
    }

    fn preview_volume(&self) -> f32 {
        self.settings.sound.preview_volume as f32 / 100.0
            * (self.settings.sound.master_volume as f32 / 100.0)
    }

    /// Open the clip for a song, starting at its preview point.
    ///
    /// Returns the reason on failure rather than an `Option`, because "most songs do not
    /// preview" is unanswerable when a missing file, a refused device and a bad seek all look
    /// the same from outside.
    fn open_preview(
        &self,
        audio: &sdl3::AudioSubsystem,
        id: i64,
    ) -> Result<(Playback, f64), String> {
        let entry = self
            .library
            .song(id)
            .map_err(|e| e.to_string())?
            .ok_or("the song is no longer in the library")?;
        let directory = entry.directory().ok_or("the song has no folder")?;
        let name = entry
            .audio_file
            .as_ref()
            .ok_or("the song names no audio file")?;
        let path = resolve_beside(directory, name)
            .ok_or_else(|| format!("{name} is not beside the song"))?;

        let clip = AudioClip::open(&path).map_err(|e| e.to_string())?;
        clip.wait_for(0.4, std::time::Duration::from_millis(500));
        if let Some(error) = clip.error() {
            return Err(error);
        }

        let mut playback = Playback::new(audio, clip).map_err(|e| e.to_string())?;
        // `#PREVIEWSTART` when the song names one, otherwise a quarter in, which is past the
        // intro on almost everything and is what UltraStar does.
        let length = playback.duration().max(entry.duration_secs);
        let start = entry
            .preview_start
            .filter(|s| *s > 0.0 && *s < length)
            .unwrap_or(length * 0.25);
        // Seeking to a point needs that much audio decoded. Decoding runs around a thousand
        // times faster than playback, so this is milliseconds — but without it the first pump
        // reads nothing and the clip appears silent.
        playback
            .clip()
            .wait_for(start + 1.0, std::time::Duration::from_secs(2));
        playback.seek(start).map_err(|e| e.to_string())?;
        playback.set_volume(self.preview_volume());
        playback.start().map_err(|e| e.to_string())?;
        Ok((playback, start))
    }

    /// Stop any preview, for when a song is about to start.
    fn stop_preview(&mut self) {
        self.preview = None;
    }

    /// Carry out whatever the song menu chose.
    fn handle_song_menu(&mut self) {
        let Some(Screen::Songs(songs)) = self.stack.last_mut() else {
            return;
        };
        let Some((action, id)) = songs.take_choice() else {
            return;
        };
        match action {
            SongAction::Sing => self.pending_sing = Some(id),
            SongAction::SingFromChorus => {
                self.pending_sing = Some(id);
                self.status = "starting from the chorus is not wired up yet".to_owned();
            }
            SongAction::ToggleFavourite => {
                self.status = "favourites arrive with player profiles".to_owned();
            }
            SongAction::ShowDetails => {
                if let Ok(Some(song)) = self.library.song(id) {
                    self.status = format!(
                        "{} \u{2014} {} BPM, {} notes, {}",
                        song.path.display(),
                        song.bpm,
                        song.note_count,
                        if song.is_playable() {
                            "playable"
                        } else {
                            "no audio"
                        }
                    );
                }
            }
            SongAction::OpenFolder => self.open_folder(id),
        }
    }

    /// Show a song's folder in the desktop's file manager.
    fn open_folder(&mut self, id: i64) {
        let Ok(Some(song)) = self.library.song(id) else {
            return;
        };
        let Some(directory) = song.directory() else {
            return;
        };
        let command = if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        // `explorer` returns a non-zero exit code even when it worked, so the child is
        // deliberately spawned and not waited on.
        match std::process::Command::new(command).arg(directory).spawn() {
            Ok(_) => self.status = format!("opened {}", directory.display()),
            Err(error) => self.status = format!("could not open the folder: {error}"),
        }
    }

    /// Whether the screen on top is editing text.
    fn wants_text(&self) -> bool {
        matches!(self.stack.last(), Some(Screen::Songs(songs)) if songs.wants_text())
    }

    /// Whether a scan is running, for the screens that say so.
    fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    fn save_settings(&self) {
        if let Err(error) = self.settings.save(self.data_dir.join("settings.toml")) {
            tracing::warn!("could not save settings: {error}");
        }
    }

    fn restyle(&mut self) {
        self.theme.metrics.text_scale = self.settings.appearance.text_scale;
        self.style = self.theme.resolve(
            &self.settings.appearance.skin,
            &self.settings.appearance.accent,
        );
    }

    /// Push whatever the settings now say into the window and the audio.
    ///
    /// A setting that needs a restart to take effect is a setting the player will conclude is
    /// broken, so this runs whenever one changes.
    fn apply_settings(&mut self, window: &mut sdl3::video::Window) {
        use sdl3::video::FullscreenType;
        let graphics = self.settings.graphics.clone();

        let fullscreen = match graphics.screen_mode {
            ScreenMode::Fullscreen => FullscreenType::True,
            _ => FullscreenType::Off,
        };
        let want_fullscreen = fullscreen != FullscreenType::Off;
        if (window.fullscreen_state() != FullscreenType::Off) != want_fullscreen {
            let _ = window.set_fullscreen(want_fullscreen);
        }
        if !want_fullscreen {
            let _ = window.set_bordered(graphics.screen_mode != ScreenMode::Borderless);
            let (width, height) = window.size();
            if width != graphics.width || height != graphics.height {
                let _ = window.set_size(graphics.width, graphics.height);
            }
        }

        let volume = self.settings.sound.master_volume as f32 / 100.0;
        if let Some(Screen::Sing(_, session)) = self.stack.last_mut() {
            session.set_volume(volume);
        }
        let preview_volume = self.preview_volume();
        if let Some(playback) = self.preview.as_mut().and_then(|p| p.playback.as_mut()) {
            playback.set_volume(preview_volume);
        }
    }

    /// Run whatever query the song screen is asking for.
    fn refresh_songs(&mut self) {
        let Some(Screen::Songs(songs)) = self.stack.last_mut() else {
            return;
        };
        if !songs.needs_query() {
            return;
        }
        let query = SearchQuery::all()
            .text(songs.search_text())
            .field(songs.field())
            .sort(songs.sort(), songs.descending());
        // No limit: the browser is the whole library, and a cap silently truncates it. Eight
        // thousand rows is a few megabytes, and the scroll is O(1) in the list length.
        match self.library.search(&query) {
            Ok(results) => songs.set_results(results),
            Err(error) => {
                tracing::warn!("search failed: {error}");
                songs.set_results(Vec::new());
            }
        }
    }

    /// Load covers for what is about to be drawn, a few per frame.
    ///
    /// Bounded per frame because decoding a JPEG takes milliseconds and a fast scroll would
    /// otherwise ask for a hundred of them at once and drop the frame rate through the floor.
    fn load_visible_covers(&mut self, renderer: &mut Renderer) {
        if self.settings.graphics.preview == rungstar_ui::settings::Preview::Off {
            return;
        }
        let Some(Screen::Songs(songs)) = self.stack.last() else {
            return;
        };
        let cursor = songs.browser.cursor();
        let all = songs.songs();
        if all.is_empty() {
            return;
        }
        // Nearest the cursor first, and a few ahead of it, so scrolling finds them already
        // decoded rather than always one step behind.
        let mut wanted: Vec<usize> = Vec::new();
        for offset in 0..12isize {
            for signed in [offset, -offset] {
                let index = cursor as isize + signed;
                if index >= 0 && (index as usize) < all.len() {
                    wanted.push(index as usize);
                }
            }
        }
        let mut budget = 3;
        for index in wanted {
            if budget == 0 {
                break;
            }
            let song = &all[index];
            if self.covers.knows(song.id) {
                continue;
            }
            let image = load_cover(song, renderer);
            self.covers.insert(song.id, image, renderer);
            budget -= 1;
        }
    }

    /// Label the on-screen hints for whatever the player is holding.
    fn set_control_hints(&mut self, gamepad: bool) {
        match self.stack.last_mut() {
            Some(Screen::Main(menu)) => menu.gamepad = gamepad,
            Some(Screen::Songs(songs)) => songs.gamepad = gamepad,
            Some(Screen::Options(options)) => options.gamepad = gamepad,
            Some(Screen::Sing(screen, _)) => screen.gamepad = gamepad,
            Some(Screen::Mics(screen, _)) => screen.gamepad = gamepad,
            _ => {}
        }
    }

    /// Whether a song is playing.
    ///
    /// A singing frame always has something moving in it, so it must never wait on an event.
    pub fn is_singing(&self) -> bool {
        matches!(self.stack.last(), Some(Screen::Sing(..)))
    }

    fn handle(&mut self, input: Input, area: Rect) {
        let transition = match self.stack.last_mut() {
            Some(Screen::Main(menu)) => menu.handle(input),
            Some(Screen::Songs(songs)) => songs.handle(input, area),
            Some(Screen::Options(options)) => {
                let outcome = options.handle(input, &mut self.settings);
                match outcome {
                    OptionsOutcome::Pop => Transition::Pop,
                    OptionsOutcome::Changed => {
                        self.restyle();
                        self.settings_dirty = true;
                        self.save_settings();
                        Transition::None
                    }
                    OptionsOutcome::Run(action) => {
                        self.run_action(action);
                        Transition::None
                    }
                    OptionsOutcome::None => Transition::None,
                }
            }
            Some(Screen::Sing(screen, session)) => {
                let (transition, choice) = screen.handle(input);
                let forced = match choice {
                    Some(PauseChoice::Continue) => {
                        session.resume();
                        Transition::None
                    }
                    Some(PauseChoice::Restart) | Some(PauseChoice::Quit) => {
                        // Restarting means going back and picking it again: keeping a session
                        // alive across a restart would mean owning the devices twice.
                        session.stop();
                        Transition::Pop
                    }
                    None => {
                        if screen.overlay == Overlay::Paused {
                            session.pause();
                        }
                        Transition::None
                    }
                };
                if forced == Transition::None {
                    transition
                } else {
                    forced
                }
            }
            Some(Screen::Mics(screen, monitor)) => {
                let (transition, outcome) = screen.handle(input);
                match outcome {
                    MicOutcome::Changed => monitor.reassign(&screen.devices),
                    MicOutcome::Refresh => {
                        monitor.rescan();
                        screen.devices = monitor.devices();
                    }
                    MicOutcome::None => {}
                }
                transition
            }
            Some(Screen::About) => match input {
                Input::Back | Input::Confirm => Transition::Pop,
                _ => Transition::None,
            },
            None => Transition::Quit,
        };
        self.apply(transition);
    }

    fn apply(&mut self, transition: Transition) {
        match transition {
            Transition::None => {}
            Transition::Pop => {
                if let Some(Screen::Mics(_, monitor)) = self.stack.last_mut() {
                    let assignment = monitor.saved();
                    monitor.stop();
                    self.settings.sound.microphones = assignment;
                    self.save_settings();
                }
                self.stack.pop();
                if self.stack.is_empty() {
                    self.running = false;
                }
            }
            Transition::Quit => self.running = false,
            Transition::Push(route) => match route {
                Route::SongSelect => {
                    self.stack.push(Screen::Songs(Box::new(SongSelect::new())));
                    // A first run has no index, and an empty song list with no explanation is
                    // where a player gives up. Scan on the way in.
                    if self.library.count().unwrap_or(0) == 0 {
                        self.start_scan(false);
                    }
                }
                Route::Options | Route::OptionsPage(_) => self
                    .stack
                    .push(Screen::Options(Box::new(OptionsScreen::new()))),
                Route::About => self.stack.push(Screen::About),
                Route::Main | Route::Search => {}
            },
            // Starting a song needs the audio subsystem, which the frame loop owns. It is
            // recorded here and acted on there.
            Transition::Sing(id) => self.pending_sing = Some(id),
        }
    }

    /// Start singing a song.
    ///
    /// Everything that touches a device lives in the session; the screen is pure and draws
    /// what it is handed. That is why this is a screen on the same stack as the browser
    /// rather than a second window.
    fn sing(&mut self, id: i64, audio: &sdl3::AudioSubsystem, capture: SdlCapture) {
        let Ok(Some(entry)) = self.library.song(id) else {
            self.status = "that song is no longer in the library".to_owned();
            return;
        };
        let Some(directory) = entry.directory().map(Path::to_path_buf) else {
            self.status = "that song has no folder".to_owned();
            return;
        };
        let Some(audio_name) = entry.audio_file.clone() else {
            self.status = format!("{} has no audio file", entry.display_name());
            return;
        };

        let bytes = match std::fs::read(&entry.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = format!("could not read the song: {error}");
                return;
            }
        };
        let parsed = match rungstar_song::SongTxt::parse_bytes(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.status = format!("{} is not a usable song: {error}", entry.display_name());
                return;
            }
        };
        let audio_path =
            resolve_beside(&directory, &audio_name).unwrap_or_else(|| directory.join(&audio_name));

        let session = session::Session::start(
            audio,
            &parsed.song,
            &audio_path,
            self.settings.game.players as usize,
            match self.settings.game.difficulty {
                rungstar_ui::settings::Difficulty::Easy => rungstar_score::Difficulty::Easy,
                rungstar_ui::settings::Difficulty::Medium => rungstar_score::Difficulty::Medium,
                rungstar_ui::settings::Difficulty::Hard => rungstar_score::Difficulty::Hard,
            },
            self.settings.threshold(),
            self.settings.sound.mic_delay_ms as f64,
            &self.settings.sound.microphones,
            capture,
        );
        let session = match session {
            Ok(session) => session,
            Err(error) => {
                self.status = format!("could not start the song: {error}");
                return;
            }
        };

        let mut screen = SingScreen::new(
            &entry.artist,
            &entry.title,
            self.settings.game.players as usize,
        );
        screen.show_input_panel = self.settings.advanced.input_panel == Switch::On;
        screen.duration = session.duration();
        let (low, high) = session.pitch_range();
        screen.pitch_low = low;
        screen.pitch_high = high;
        if self.settings.graphics.backgrounds == Switch::On {
            screen.background = self.covers.get(entry.id);
        }
        let _ = self.library.record_play(id);
        self.stack
            .push(Screen::Sing(Box::new(screen), Box::new(session)));
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::RescanLibrary => self.start_scan(false),
            Action::RebuildIndex => self.start_scan(true),
            Action::ResetToDefaults => {
                self.settings = Settings::default();
                self.restyle();
                self.save_settings();
                self.status = "settings reset".to_owned();
            }
            // These need screens that belong to later phases. Saying so is better than a
            // button that silently does nothing.
            Action::AddSongFolder => {
                self.status = format!(
                    "add song folders by editing {}",
                    self.data_dir.join("settings.toml").display()
                )
            }
            // Needs the audio subsystem, which the frame loop owns.
            Action::ManageMicrophones => self.pending_mics = true,
            Action::RebindControls => {
                self.status = "rebinding arrives with the input screen".to_owned()
            }
        }
    }

    fn draw(&mut self, list: &mut DrawList, area: Rect) {
        match self.stack.last_mut() {
            Some(Screen::Main(menu)) => {
                let subtitle = if self.status.is_empty() {
                    "An UltraStar Deluxe-class karaoke game"
                } else {
                    &self.status
                };
                menu.draw(list, area, &self.style, subtitle);
            }
            Some(Screen::Songs(songs)) => {
                let covers = &self.covers;
                songs.draw(list, area, &self.style, &|id| covers.get(id));
            }
            Some(Screen::Options(options)) => options.draw(list, area, &self.style, &self.settings),
            Some(Screen::Sing(screen, session)) => {
                let beat = session.visual_beat();
                let (syllables, next) = session.lyrics(beat);
                let line = session.current_line(beat);
                screen.draw(list, area, &self.style, &line, &syllables, &next, beat);
            }
            Some(Screen::Mics(screen, _)) => screen.draw(list, area, &self.style),
            Some(Screen::About) => draw_about(list, area, &self.style),
            None => {}
        }
    }
}

/// Find a file beside the song, tolerating a case mismatch in the header.
///
/// Real libraries are full of `#MP3:Song.MP3` next to `Song.mp3`, and on Linux that is a
/// missing file rather than a typo.
fn resolve_beside(directory: &Path, name: &str) -> Option<PathBuf> {
    let direct = directory.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    let wanted = name.to_lowercase();
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().to_lowercase() == wanted)
        .map(|entry| entry.path())
}

/// Load the theme named in the settings, falling back to the built-in one.
fn load_theme(data_dir: &std::path::Path, settings: &Settings) -> Theme {
    let path = data_dir
        .join("themes")
        .join(format!("{}.toml", settings.appearance.theme.to_lowercase()));
    let mut theme = match Theme::load(&path) {
        Ok(theme) if theme.validate().is_ok() => theme,
        Ok(_) | Err(_) => Theme::builtin(),
    };
    theme.metrics.text_scale = settings.appearance.text_scale;
    theme
}

fn draw_about(list: &mut DrawList, area: Rect, style: &Style) {
    let widgets = Widgets::new(style);
    let body = widgets.header(list, area, "About", "");
    let body = widgets.footer(list, body, &[("B", "Back")]);
    let inner = body.inset(style.gap(3.0));

    let lines = [
        ("RungStar", 1.6),
        (concat!("version ", env!("CARGO_PKG_VERSION")), 0.9),
        ("", 0.6),
        (
            "Licensed under the GNU General Public License, version 3 or later.",
            0.9,
        ),
        ("", 0.4),
        (
            "Behaviour is derived from UltraStar Deluxe and usdb_syncer, both copyleft, so \
             this is too. No upstream source is copied: the song format, the scoring and the \
             timing are reimplemented from a written specification, which is what makes them \
             testable.",
            0.9,
        ),
        ("", 0.4),
        ("UltraStar Deluxe - github.com/UltraStar-Deluxe/USDX", 0.85),
        ("usdb_syncer - github.com/bohning/usdb_syncer", 0.85),
    ];
    let mut y = inner.y;
    for (text, scale) in lines {
        let height = style.scaled_text(scale) * 1.8;
        list.text(
            Rect::new(inner.x, y, inner.w, height),
            text,
            TextStyle::new(
                style.scaled_text(scale),
                if scale > 1.0 { style.text } else { style.muted },
            ),
        );
        y += height;
    }
}

/// Whether a key will also arrive as typed text.
///
/// The rule is not a list of keys that are allowed through — that was the first attempt, and
/// it silently blocked F3, which is how you change the field being searched. The rule is that
/// a key producing a character must not *also* fire a shortcut, because the character is
/// already being delivered as `TextInput`. Everything else — function keys, arrows, Escape,
/// Tab — is unambiguous and stays live.
fn produces_text(keycode: Keycode) -> bool {
    let name = keycode.name();
    // Single-character names are exactly the printable keys: letters, digits and punctuation.
    // Space is spelled out but types a character all the same.
    name.chars().count() == 1 || name == "Space"
}

/// Map a key or button to the semantic input the screens understand./// Map a key or button to the semantic input the screens understand.
fn action_for(keycode: Keycode) -> Option<Input> {
    Some(match keycode {
        Keycode::Up => Input::Up,
        Keycode::Down => Input::Down,
        Keycode::Left => Input::Left,
        Keycode::Right => Input::Right,
        Keycode::Return | Keycode::KpEnter => Input::Confirm,
        Keycode::Escape => Input::Back,
        Keycode::Tab => Input::CycleLayout,
        // The keys the on-screen hints name. A hint that names a key which does nothing is
        // worse than no hint at all.
        Keycode::F => Input::Search,
        Keycode::Slash => Input::Search,
        Keycode::F3 => Input::Sort,
        Keycode::M => Input::ContextMenu,
        Keycode::R => Input::Random,
        Keycode::PageUp => Input::PageUp,
        Keycode::PageDown => Input::PageDown,
        Keycode::Backspace => Input::Backspace,
        _ => return None,
    })
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // `--check` starts everything a real launch does — settings, index, window, fonts,
    // renderer — draws one frame of every screen and exits. It is what makes "the game
    // starts" a thing that can be asserted rather than looked at, on a build machine with
    // nobody in front of it.
    let check_only = std::env::args().any(|a| a == "--check");

    let data_dir = paths::data_directory();
    let mut app = App::new(data_dir)?;

    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    let video = sdl.video().map_err(|e| anyhow::anyhow!("{e}"))?;
    let gamepads = sdl.gamepad().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut open_pads = Vec::new();

    let mut builder = video.window(
        "RungStar",
        app.settings.graphics.width,
        app.settings.graphics.height,
    );
    builder.position_centered().resizable();
    match app.settings.graphics.screen_mode {
        ScreenMode::Fullscreen => {
            builder.fullscreen();
        }
        ScreenMode::Borderless => {
            builder.borderless();
        }
        ScreenMode::Windowed => {}
    }
    let window = builder.build().context("creating the window")?;
    let canvas = window.into_canvas();

    let fonts = FontSet::load(None, None, None).context(
        "loading a font. No font is bundled yet, so the game borrows one from the system; \
         install a standard font package if this fails",
    )?;
    let mut renderer = Renderer::new(canvas, fonts).map_err(|e| anyhow::anyhow!("{e}"))?;

    let audio_subsystem = sdl.audio().map_err(|e| anyhow::anyhow!("no audio: {e}"))?;
    let mut events = sdl.event_pump().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut text_input_on = false;
    let mut list = DrawList::new();
    let mut last = Instant::now();

    if check_only {
        return self_check(&mut app, &mut renderer, &mut list);
    }

    while app.running {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.1);
        last = now;

        let area = renderer.projection().screen();
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } => app.running = false,
                Event::Window {
                    win_event: WindowEvent::Resized(..) | WindowEvent::PixelSizeChanged(..),
                    ..
                } => {
                    let _ = renderer.resize();
                }
                Event::KeyDown {
                    keycode: Some(key), ..
                } => {
                    app.set_control_hints(false);
                    // While a screen is editing text, a key that types a character must not
                    // also act as a shortcut. The character itself arrives as `TextInput`,
                    // which is also the only way to get an accented letter or a non-Latin
                    // script.
                    if !app.wants_text() || !produces_text(key) {
                        if let Some(input) = action_for(key) {
                            app.handle(input, area);
                        }
                    }
                }
                Event::TextInput { text, .. } => {
                    if app.wants_text() {
                        for c in text.chars() {
                            app.handle(Input::Type(c), area);
                        }
                    }
                }
                Event::MouseMotion { x, y, .. } => {
                    app.set_control_hints(false);
                    app.handle(Input::Hover(renderer.projection().unproject(x, y)), area);
                }
                Event::MouseButtonDown { x, y, .. } => {
                    app.set_control_hints(false);
                    app.handle(Input::Click(renderer.projection().unproject(x, y)), area);
                }
                Event::MouseWheel { y, .. } => {
                    // A wheel notch is a step, and a fast flick is several.
                    let steps = (y.abs().ceil() as i32).clamp(1, 8);
                    for _ in 0..steps {
                        app.handle(if y > 0.0 { Input::Up } else { Input::Down }, area);
                    }
                }
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Ok(pad) = gamepads.open(sdl3::joystick::JoystickId::new(which)) {
                        open_pads.push(pad);
                    }
                }
                Event::ControllerButtonDown { button, .. } => {
                    app.set_control_hints(true);
                    use sdl3::gamepad::Button;
                    let input = match button {
                        Button::DPadUp => Some(Input::Up),
                        Button::DPadDown => Some(Input::Down),
                        Button::DPadLeft => Some(Input::Left),
                        Button::DPadRight => Some(Input::Right),
                        Button::South => Some(Input::Confirm),
                        Button::East => Some(Input::Back),
                        Button::West => Some(Input::Search),
                        Button::North => Some(Input::Sort),
                        Button::Back => Some(Input::ContextMenu),
                        Button::LeftShoulder => Some(Input::CycleLayout),
                        Button::RightShoulder => Some(Input::CycleLayout),
                        _ => None,
                    };
                    if let Some(input) = input {
                        app.handle(input, area);
                    }
                }
                _ => {}
            }
        }

        // A song the browser asked for. Started here because it needs the audio subsystem,
        // and a fresh capture handle so the microphones belong to this session alone.
        if std::mem::take(&mut app.pending_mics) {
            app.stop_preview();
            let capture = SdlCapture::new(audio_subsystem.clone());
            let mut monitor = session::Monitor::start(
                capture,
                app.settings.game.players as usize,
                &app.settings.sound.microphones,
            );
            let mut screen = MicScreen::new(app.settings.game.players as usize);
            screen.gate = app.settings.threshold();
            screen.devices = monitor.devices();
            monitor.tick();
            app.stack
                .push(Screen::Mics(Box::new(screen), Box::new(monitor)));
        }

        if let Some(id) = app.pending_sing.take() {
            app.stop_preview();
            let capture = SdlCapture::new(audio_subsystem.clone());
            app.sing(id, &audio_subsystem, capture);
        }

        if std::mem::take(&mut app.settings_dirty) {
            app.apply_settings(renderer.canvas().window_mut());
            let _ = renderer.resize();
        }
        // SDL3 delivers no `TextInput` events until text input is started for the window, and
        // starts none by default. Without this the on-screen keyboard works and the physical
        // one does nothing, which is a strange way for a search box to behave.
        let editing = app.wants_text();
        if editing != text_input_on {
            let window = renderer.canvas().window();
            if editing {
                video.text_input().start(window);
            } else {
                video.text_input().stop(window);
            }
            text_input_on = editing;
        }

        app.poll_scan();
        let scanning = app.scanning().then(|| app.status.clone());
        if let Some(Screen::Songs(songs)) = app.stack.last_mut() {
            songs.scanning = scanning;
        }
        app.handle_song_menu();
        app.refresh_songs();
        app.update_preview(&audio_subsystem);
        app.load_visible_covers(&mut renderer);
        match app.stack.last_mut() {
            Some(Screen::Songs(songs)) => {
                songs.tick(dt);
            }
            Some(Screen::Mics(screen, monitor)) => {
                monitor.tick();
                screen.devices = monitor.devices();
            }
            Some(Screen::Sing(screen, session)) => {
                if let Err(error) = session.tick() {
                    tracing::warn!("playback stopped: {error}");
                    session.stop();
                    app.apply(Transition::Pop);
                } else {
                    session.update_singers(&mut screen.singers);
                    screen.position = session.position();
                    if session.is_finished() && screen.overlay != Overlay::Results {
                        // The scores go up rather than the screen closing: in a party the
                        // result is the point, and popping straight back to the browser
                        // throws it away before anybody has read it.
                        screen.overlay = Overlay::Results;
                        session.stop();
                    }
                }
            }
            _ => {}
        }

        list.clear();
        app.draw(&mut list, area);
        debug_assert!(list.is_balanced(), "a screen left a clip pushed");
        if app.settings.advanced.show_fps == Switch::On {
            let fps = format!("{:.0} fps", 1.0 / dt.max(0.0001));
            list.text(
                Rect::new(area.right() - 200.0, 4.0, 190.0, 40.0),
                fps,
                TextStyle::new(24.0, app.style.muted).align(rungstar_ui::Align::End),
            );
        }
        renderer
            .render(&list, app.style.background)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // A menu with nothing moving does not need to redraw as fast as the display can go,
        // and on a handheld that is battery for no picture. A song always has something
        // moving, so it never sleeps.
        if !app.is_singing() {
            let settled = match app.stack.last() {
                Some(Screen::Songs(songs)) => !songs.browser.animating(),
                // The meters move continuously; sleeping would make them stutter.
                Some(Screen::Mics(..)) => false,
                _ => true,
            };
            if settled {
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        }
    }

    app.save_settings();
    Ok(())
}

/// Draw one frame of every screen and report what happened.
///
/// Every screen is visited rather than just the first, because the failures worth catching
/// here — a font that will not load, a theme that resolves to nothing, a layout that divides
/// by a zero-width panel — are per screen and would otherwise wait for somebody to navigate
/// there.
fn self_check(app: &mut App, renderer: &mut Renderer, list: &mut DrawList) -> Result<()> {
    let area = renderer.projection().screen();
    println!("window      {:.0}x{:.0} design units", area.w, area.h);
    println!(
        "theme       {} / {}",
        app.theme.meta.name, app.settings.appearance.skin
    );
    for root in app.song_roots() {
        println!("songs from  {}", root.display());
    }
    // Scan here rather than reporting whatever happens to be indexed, so the count below is
    // a fact about the disk and not about a previous run.
    app.rescan(false);
    println!(
        "library     {} songs ({})",
        app.library.count().unwrap_or(0),
        app.status
    );

    let screens: Vec<(&str, Screen)> = vec![
        ("main", Screen::Main(MainMenu::new())),
        ("songs", Screen::Songs(Box::new(SongSelect::new()))),
        ("options", Screen::Options(Box::new(OptionsScreen::new()))),
        ("about", Screen::About),
    ];
    for (name, screen) in screens {
        app.stack.push(screen);
        app.refresh_songs();
        list.clear();
        app.draw(list, area);
        if !list.is_balanced() {
            anyhow::bail!("{name} left a clip pushed");
        }
        renderer
            .render(list, app.style.background)
            .map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
        println!("{name:<12}{} draw commands", list.len());
        app.stack.pop();
    }

    // The microphone screen, with a device that has never produced a sample -- the state a
    // player is in when they open it to find out why nothing is scoring.
    {
        let mut screen = rungstar_ui::micscreen::MicScreen::new(2);
        screen.devices = vec![rungstar_ui::micscreen::Device {
            name: "Example microphone".to_owned(),
            assignment: vec![1, 2],
            levels: vec![0.3, 0.0],
            heard: vec![true, false],
        }];
        list.clear();
        screen.draw(list, area, &app.style);
        if !list.is_balanced() {
            anyhow::bail!("the microphone screen left a clip pushed");
        }
        renderer
            .render(list, app.style.background)
            .map_err(|e| anyhow::anyhow!("microphones: {e}"))?;
        println!("microphones {} draw commands", list.len());
    }

    // The sing screen, drawn without a session: notes, lyrics and singers are all supplied
    // by the caller, so it can be exercised with nothing playing.
    {
        let mut screen = SingScreen::new("Artist", "Title", 6);
        screen.show_input_panel = true;
        let notes: Vec<rungstar_ui::singscreen::Note> = (0..9)
            .map(|i| rungstar_ui::singscreen::Note {
                start: 8.0 + i as f64 * 4.0,
                duration: 3.0,
                pitch: 60 + (i % 7),
                kind: if i % 5 == 0 {
                    rungstar_ui::singscreen::NoteKind::Golden
                } else {
                    rungstar_ui::singscreen::NoteKind::Normal
                },
            })
            .collect();
        screen.pitch_low = 60;
        screen.pitch_high = 66;
        let line = rungstar_ui::singscreen::NoteLine {
            start: notes.first().map(|n| n.start).unwrap_or(0.0),
            end: notes.last().map(|n| n.start + n.duration).unwrap_or(0.0),
            notes,
        };
        let syllables: Vec<rungstar_ui::singscreen::Syllable> = ["Hel", "lo ", "world"]
            .iter()
            .enumerate()
            .map(|(i, text)| rungstar_ui::singscreen::Syllable {
                text: (*text).to_owned(),
                start: i as f64 * 4.0,
                duration: 3.0,
                golden: false,
            })
            .collect();
        for overlay in [Overlay::None, Overlay::Paused, Overlay::Results] {
            screen.overlay = overlay;
            list.clear();
            screen.draw(list, area, &app.style, &line, &syllables, "next line", 20.0);
            if !list.is_balanced() {
                anyhow::bail!("the sing screen left a clip pushed");
            }
            renderer
                .render(list, app.style.background)
                .map_err(|e| anyhow::anyhow!("sing: {e}"))?;
        }
        println!("sing        {} draw commands, 6 singers", list.len());
    }

    // And the two browser overlays, which have their own layout maths.
    app.stack.push(Screen::Songs(Box::new(SongSelect::new())));
    for overlay in [Input::Search, Input::Search, Input::Sort] {
        app.handle(overlay, area);
        list.clear();
        app.draw(list, area);
        renderer
            .render(list, app.style.background)
            .map_err(|e| anyhow::anyhow!("overlay: {e}"))?;
    }
    println!("overlays    drawn");
    println!("ok");
    Ok(())
}

/// Keeps `Color` in scope for the draw helpers above.
#[allow(dead_code)]
const _: Option<Color> = None;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_keys_are_told_apart_from_command_keys() {
        // The keys that must not fire a shortcut while a search box has focus, because each
        // of them also arrives as text.
        for key in [
            Keycode::A,
            Keycode::F,
            Keycode::M,
            Keycode::R,
            Keycode::Z,
            Keycode::_0,
            Keycode::_9,
            Keycode::Space,
            Keycode::Minus,
            Keycode::Period,
        ] {
            assert!(produces_text(key), "{key:?} types a character");
        }

        // And the keys that must keep working: navigating the on-screen keyboard, finishing,
        // leaving, and F3 for the field being searched. An earlier whitelist blocked F3 and
        // made "search in" unreachable from a keyboard.
        for key in [
            Keycode::Up,
            Keycode::Down,
            Keycode::Left,
            Keycode::Right,
            Keycode::Return,
            Keycode::Escape,
            Keycode::Backspace,
            Keycode::Tab,
            Keycode::F3,
            Keycode::PageUp,
            Keycode::PageDown,
        ] {
            assert!(!produces_text(key), "{key:?} is a command, not a character");
        }
    }
}
