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
use rungstar_profile::stats::View;
use rungstar_profile::{Profiles, Score};
use rungstar_ui::draw::{DrawList, ImageId, TextStyle};
use rungstar_ui::geom::Rect;
use rungstar_ui::menus::{MainMenu, OptionsOutcome, OptionsScreen};
use rungstar_ui::micscreen::{MicOutcome, MicScreen};
use rungstar_ui::options::Action;
use rungstar_ui::partyscreen::{Kind, PartyOutcome, PartyScreen, Stage};
use rungstar_ui::playerscreen::{Entry, PlayerOutcome, PlayerScreen};
use rungstar_ui::screen::{Route, Transition, Widgets};
use rungstar_ui::settings::{OnSongClick, ScreenMode, Settings, Switch};
use rungstar_ui::singscreen::{Overlay, PauseChoice, SingScreen};
use rungstar_ui::songselect::{Facet, FacetValues, Input, SongAction, SongSelect};
use rungstar_ui::statsscreen::{Row as StatRow, StatsScreen};
use rungstar_ui::theme::{Style, Theme};
use rungstar_ui::usdbscreen::{Activity, Local, Row as UsdbRow, UsdbOutcome, UsdbScreen};
use rungstar_ui::Color;

mod session;
mod usdbjob;

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
    Players(Box<PlayerScreen>),
    Party(Box<PartyScreen>),
    Usdb(Box<UsdbScreen>),
    Stats(Box<StatsScreen>),
    About,
}

/// Everything the running game holds.
struct App {
    settings: Settings,
    theme: Theme,
    style: Style,
    library: Database,
    profiles: Profiles,
    /// Who is singing, by profile id, in singer order.
    singers: Vec<i64>,
    stack: Vec<Screen>,
    covers: CoverCache,
    data_dir: PathBuf,
    /// Set when a scan or a query has changed what the browser should be showing.
    status: String,
    /// A song the browser asked for, waiting for the frame loop to open the devices.
    pending_sing: Option<i64>,
    /// The song the singer picker is open for, if it is open.
    pending_pick: Option<i64>,
    /// How the next song is to be played: where it starts, and under what challenge.
    ///
    /// Kept beside the song rather than inside `pending_sing` so that restarting from the
    /// pause menu replays the same medley or challenge rather than quietly reverting to the
    /// plain song from the top.
    next_plan: session::Plan,
    /// The challenge the next song is sung under.
    challenge: &'static rungstar_party::Challenge,
    /// The song offered for the party round in progress.
    party_song: Option<i64>,
    /// Set while the browser is open to pick a song for a party round rather than to sing one.
    party_picking: bool,
    /// Set when a party song has just finished and its scores have to be reported.
    party_scores: Option<Vec<i32>>,
    /// Whether the jukebox is running: songs play back to back and nothing is scored.
    jukebox: bool,
    /// The USDB worker, started the first time the browser is opened.
    ///
    /// Lazily, because it logs in: somebody who never opens it should not have their password
    /// read out of the keyring on every launch.
    usdb: Option<usdbjob::UsdbJob>,
    /// Set when the microphone screen has been asked for.
    pending_mics: bool,
    /// The song being sung, so Restart knows what to start again.
    singing: Option<i64>,
    /// A video texture whose song has ended, waiting for the frame loop to release it.
    dropped_video: Option<rungstar_ui::draw::ImageId>,
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
        // Its own file, because the index is a cache that can be rebuilt and this is not.
        let profiles =
            Profiles::open(data_dir.join("profiles.db")).context("opening the profiles")?;

        Ok(Self {
            settings,
            theme,
            style,
            library,
            profiles,
            singers: Vec::new(),
            stack: vec![Screen::Main(MainMenu::new())],
            covers: CoverCache::new(),
            data_dir,
            status: String::new(),
            pending_sing: None,
            pending_pick: None,
            next_plan: session::Plan::default(),
            challenge: rungstar_party::Challenge::normal(),
            party_song: None,
            party_picking: false,
            party_scores: None,
            jukebox: false,
            usdb: None,
            pending_mics: false,
            singing: None,
            dropped_video: None,
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
            SongAction::Sing => {
                self.next_plan = self.plain_plan();
                self.request_sing(id);
            }
            SongAction::SingFromChorus => {
                // The challenge is taken from the browser first, so a medley is sung under
                // whatever was chosen rather than always plainly.
                self.plain_plan();
                match self.medley(id) {
                    Some(plan) => {
                        self.next_plan = plan;
                        self.request_sing(id);
                    }
                    None => {
                        self.status = "that song has no chorus marked and is too short to guess one"
                            .to_owned()
                    }
                }
            }
            SongAction::PickChallenge => {
                if let Some(Screen::Songs(songs)) = self.stack.last_mut() {
                    songs.pick_challenge();
                }
            }
            SongAction::ToggleFavourite => {
                // A favourite belongs to somebody, so there has to be a somebody. Whoever is
                // singing first, or the only profile if there is one.
                let owner = self
                    .singers
                    .first()
                    .copied()
                    .or_else(|| self.profiles.players().ok()?.first().map(|p| p.id));
                let Some(owner) = owner else {
                    self.status =
                        "add a singer first, so the favourite belongs to somebody".to_owned();
                    return;
                };
                if let Ok(Some(song)) = self.library.song(id) {
                    match self
                        .profiles
                        .toggle_favourite(owner, &song.artist, &song.title)
                    {
                        Ok(true) => self.status = format!("favourited {}", song.display_name()),
                        Ok(false) => self.status = format!("unfavourited {}", song.display_name()),
                        Err(error) => self.status = error.to_string(),
                    }
                }
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
        match self.stack.last() {
            Some(Screen::Songs(songs)) => songs.wants_text(),
            Some(Screen::Players(screen)) => screen.wants_text(),
            Some(Screen::Usdb(screen)) => screen.wants_text(),
            _ => false,
        }
    }

    /// Reload the player list, with each profile's own history beside their name.
    fn refresh_players(&self, screen: &mut PlayerScreen) {
        let Ok(players) = self.profiles.players() else {
            return;
        };
        screen.players = players
            .into_iter()
            .map(|player| {
                // A profile is worth having because it remembers; showing what it remembers is
                // what makes that visible rather than a claim.
                let history = self.profiles.history(player.id, 1_000).unwrap_or_default();
                Entry {
                    id: player.id,
                    name: player.name,
                    colour: player.colour,
                    songs: history.len() as i64,
                    best: history.iter().map(|s| s.points).max().unwrap_or(0),
                }
            })
            .collect();
        // A profile that no longer exists cannot be singing.
        let present: Vec<i64> = screen.players.iter().map(|p| p.id).collect();
        screen.singers.retain(|id| present.contains(id));
    }

    /// Carry out what the player screen asked for.
    fn apply_player_outcome(&mut self, outcome: PlayerOutcome) {
        let now = unix_now();
        match outcome {
            PlayerOutcome::None => return,
            PlayerOutcome::Add(name) => {
                if let Err(error) = self.profiles.ensure_player(&name, now) {
                    self.status = error.to_string();
                }
            }
            PlayerOutcome::Rename(id, name) => {
                if let Err(error) = self.profiles.rename_player(id, &name) {
                    self.status = error.to_string();
                }
            }
            PlayerOutcome::Recolour(id, colour) => {
                let _ = self.profiles.set_colour(id, colour);
            }
            PlayerOutcome::Remove(id) => {
                let _ = self.profiles.remove_player(id);
                self.singers.retain(|s| *s != id);
            }
            PlayerOutcome::Singers(singers) => self.singers = singers,
            PlayerOutcome::Start => {
                if let Some(id) = self.pending_pick.take() {
                    self.stack.pop();
                    self.pending_sing = Some(id);
                }
                return;
            }
        }

        // Taken out and put back, because refreshing needs the store and the screen at once.
        if let Some(Screen::Players(screen)) = self.stack.last_mut() {
            let mut taken = std::mem::take(screen);
            self.refresh_players(&mut taken);
            self.singers.clone_from(&taken.singers);
            if let Some(Screen::Players(slot)) = self.stack.last_mut() {
                *slot = taken;
            }
        }
    }

    /// Fill the statistics screen for whichever view it is showing.
    fn refresh_stats(&mut self) {
        let Some(Screen::Stats(screen)) = self.stack.last() else {
            return;
        };
        if !screen.needs_rows() {
            return;
        }
        let (view, order) = (screen.view, screen.order);
        const LIMIT: usize = 200;
        let rows: Vec<StatRow> = match view {
            View::Scores => self
                .profiles
                .best_scores(order, LIMIT)
                .unwrap_or_default()
                .into_iter()
                .map(|e| StatRow {
                    label: format!("{} \u{2013} {}", e.artist, e.title),
                    detail: e.player,
                    value: e.points.to_string(),
                })
                .collect(),
            View::Singers => self
                .profiles
                .best_singers(order, LIMIT)
                .unwrap_or_default()
                .into_iter()
                .map(|e| StatRow {
                    label: e.name,
                    detail: format!("{} songs, best {}", e.songs, e.best),
                    value: e.average.to_string(),
                })
                .collect(),
            View::Songs => self
                .profiles
                .most_sung_songs(order, LIMIT)
                .unwrap_or_default()
                .into_iter()
                .map(|e| StatRow {
                    label: format!("{} \u{2013} {}", e.artist, e.title),
                    detail: format!("best {}", e.best),
                    value: e.times.to_string(),
                })
                .collect(),
            View::Artists => self
                .profiles
                .most_sung_artists(order, LIMIT)
                .unwrap_or_default()
                .into_iter()
                .map(|e| StatRow {
                    label: e.artist,
                    detail: String::new(),
                    value: e.times.to_string(),
                })
                .collect(),
        };
        if let Some(Screen::Stats(screen)) = self.stack.last_mut() {
            screen.set_rows(rows);
        }
    }

    /// Write down the song that has just ended, if there is one on top.
    fn record_finished_song(&mut self) {
        let Some(Screen::Sing(screen, _)) = self.stack.last() else {
            return;
        };
        // Cloned so the store can be borrowed mutably; a handful of singers, once a song.
        let artist = screen.artist.clone();
        let title = screen.title.clone();
        let scores: Vec<i32> = screen.singers.iter().map(|s| s.score).collect();
        self.record_scores(&artist, &title, &scores);
    }

    /// Write down what everybody scored, once a song is over.
    ///
    /// Only for singers who have a profile: an unnamed player's score has nowhere to go and
    /// recording it under a placeholder is how UltraStar's tables end up full of "Player 1".
    fn record_scores(&mut self, artist: &str, title: &str, scores: &[i32]) {
        let now = unix_now();
        for (index, points) in scores.iter().enumerate() {
            let Some(player_id) = self.singers.get(index).copied() else {
                continue;
            };
            let score = Score {
                player_id,
                artist: artist.to_owned(),
                title: title.to_owned(),
                difficulty: match self.settings.game.difficulty {
                    rungstar_ui::settings::Difficulty::Easy => 0,
                    rungstar_ui::settings::Difficulty::Medium => 1,
                    rungstar_ui::settings::Difficulty::Hard => 2,
                },
                points: *points,
                notes: 0,
                golden: 0,
                line_bonus: 0,
                sung_at: now,
            };
            if let Err(error) = self.profiles.record(&score) {
                tracing::warn!("score not saved: {error}");
            }
        }
    }

    /// Read an existing UltraStar database, from wherever it usually lives.
    fn import_ultrastar(&mut self) {
        let found = rungstar_profile::import::likely_ultrastar_paths()
            .into_iter()
            .find(|path| path.is_file());
        let Some(path) = found else {
            self.status = "no UltraStar database found in the usual places".to_owned();
            return;
        };
        match rungstar_profile::import_ultrastar(&mut self.profiles, &path, unix_now()) {
            Ok(report) => self.status = report.summary(),
            Err(error) => self.status = format!("import failed: {error}"),
        }
    }

    /// Keep the browser's highscore panel in step with the cursor.
    fn refresh_highscores(&mut self) {
        let wanted = match self.stack.last() {
            Some(Screen::Songs(songs)) => songs
                .selected()
                .map(|song| (song.artist.clone(), song.title.clone())),
            _ => None,
        };
        let Some((artist, title)) = wanted else {
            return;
        };
        let scores = self.highscores(&artist, &title);
        if let Some(Screen::Songs(songs)) = self.stack.last_mut() {
            songs.highscores = scores;
        }
    }

    /// The song's table, for the browser to show beside it.
    fn highscores(&self, artist: &str, title: &str) -> Vec<(String, i32)> {
        self.profiles
            .best_for(artist, title, None, 5)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| (entry.player, entry.points))
            .collect()
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

    /// Fill the browser's filter lists from the index.
    ///
    /// Once, and again after a scan. Seven `GROUP BY` queries over eight thousand rows is a
    /// few milliseconds, and doing it when the panel opens instead would put that on the
    /// keypress that opens it.
    fn refresh_facets(&mut self) {
        let Some(Screen::Songs(songs)) = self.stack.last() else {
            return;
        };
        if !songs.needs_facets() {
            return;
        }
        let mut values = FacetValues::new();
        for facet in Facet::ALL {
            let Some(column) = facet.column() else {
                continue;
            };
            match self.library.facet(column) {
                Ok(found) => values.set(facet, found),
                Err(error) => tracing::warn!("could not list {column}: {error}"),
            }
        }
        if let Some(Screen::Songs(songs)) = self.stack.last_mut() {
            songs.set_facets(values);
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
            .sort(songs.sort(), songs.descending())
            .filters(songs.filters());
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
            Some(Screen::Players(screen)) => screen.gamepad = gamepad,
            Some(Screen::Party(screen)) => screen.gamepad = gamepad,
            Some(Screen::Usdb(screen)) => screen.gamepad = gamepad,
            Some(Screen::Stats(screen)) => screen.gamepad = gamepad,
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
                let mut restart = false;
                let mut record = false;
                let forced = match choice {
                    Some(PauseChoice::Continue) => {
                        session.resume();
                        Transition::None
                    }
                    // Giving up still earns what was already sung. Dropping straight back to
                    // the browser throws away a score somebody worked for, and a half-sung
                    // song is exactly when you most want to see the number.
                    //
                    // Skipping the outro *records* it, because the singing is finished and
                    // only the instrumental is left — the score is the same one the song
                    // would have ended with. Giving up does not: that is an abandoned song,
                    // and a partial score in the highscore table is a lie about a whole one.
                    Some(PauseChoice::SkipOutro) => {
                        session.stop();
                        screen.overlay = Overlay::Results;
                        record = true;
                        Transition::None
                    }
                    Some(PauseChoice::Quit) => {
                        session.stop();
                        screen.overlay = Overlay::Results;
                        Transition::None
                    }
                    Some(PauseChoice::Restart) => {
                        // The devices cannot be owned twice, so the old session is dropped
                        // before the new one opens them.
                        session.stop();
                        restart = true;
                        Transition::Pop
                    }
                    None => {
                        if screen.overlay == Overlay::Paused {
                            session.pause();
                        }
                        Transition::None
                    }
                };
                if record {
                    self.record_finished_song();
                }
                if restart {
                    self.pending_sing = self.singing;
                }
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
            Some(Screen::Players(screen)) => {
                let (transition, outcome) = screen.handle(input);
                self.apply_player_outcome(outcome);
                transition
            }
            Some(Screen::Party(screen)) => {
                let (transition, outcome) = screen.handle(input);
                self.apply_party_outcome(outcome);
                transition
            }
            Some(Screen::Usdb(screen)) => {
                let (transition, outcome) = screen.handle(input);
                self.apply_usdb_outcome(outcome);
                transition
            }
            Some(Screen::Stats(screen)) => screen.handle(input),
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
                if let Some(Screen::Sing(screen, _)) = self.stack.last() {
                    // The video texture is this song's; leaving it behind would leak one per
                    // song sung.
                    self.dropped_video = screen.video;
                    // A party song's result is read off the same panels the singers watched,
                    // so there is one score per microphone and it is already in slot order.
                    let under = self.stack.len().saturating_sub(2);
                    if matches!(self.stack.get(under), Some(Screen::Party(_))) {
                        self.party_scores = Some(screen.singers.iter().map(|s| s.score).collect());
                    }
                }
                if let Some(Screen::Mics(screen, monitor)) = self.stack.last_mut() {
                    let assignment = monitor.saved();
                    // The number of singers follows the assignment rather than the other way
                    // round: you decided how many are playing by giving them channels.
                    let singers = screen.singer_count();
                    monitor.stop();
                    self.settings.sound.microphones = assignment;
                    if singers > 0 {
                        self.settings.game.players = singers as u8;
                    }
                    self.settings.clamp();
                    self.save_settings();
                }
                if matches!(self.stack.last(), Some(Screen::Players(_))) {
                    // Backing out of the picker is a change of mind about the song, not just
                    // about the singers.
                    self.pending_pick = None;
                }
                self.stack.pop();
                if self.stack.is_empty() {
                    self.running = false;
                }
                // Reported after the score screen has closed rather than when the song ended,
                // so the standings do not change under the result being read.
                self.report_party_round();
                // The jukebox rolls straight into the next song, and Back from the sing screen
                // is what stops it: an endless mode needs one obvious way out.
                if self.jukebox {
                    self.jukebox = false;
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
                Route::Players => {
                    let mut screen = PlayerScreen::new();
                    screen.microphones = self.settings.game.players as usize;
                    screen.singers = self.singers.clone();
                    self.refresh_players(&mut screen);
                    self.stack.push(Screen::Players(Box::new(screen)));
                }
                Route::Party => {
                    let mut screen = PartyScreen::new();
                    screen.pool = self
                        .profiles
                        .players()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|player| player.name)
                        .collect();
                    self.stack.push(Screen::Party(Box::new(screen)));
                }
                Route::Jukebox => {
                    if self.library.count().unwrap_or(0) == 0 {
                        self.start_scan(false);
                        self.status = "finding your songs first".to_owned();
                    }
                    match self.random_song() {
                        Some(id) => {
                            self.jukebox = true;
                            self.next_plan = session::Plan::default();
                            self.pending_sing = Some(id);
                        }
                        None => self.status = "there are no songs to play".to_owned(),
                    }
                }
                Route::Usdb => {
                    self.start_usdb();
                    let mut screen = UsdbScreen::new();
                    if let Some(job) = &self.usdb {
                        screen.catalog_size = job.catalog().len();
                    }
                    self.stack.push(Screen::Usdb(Box::new(screen)));
                }
                Route::Stats => self.stack.push(Screen::Stats(Box::new(StatsScreen::new()))),
                Route::About => self.stack.push(Screen::About),
                Route::Main | Route::Search => {}
            },
            // Starting a song needs the audio subsystem, which the frame loop owns. It is
            // recorded here and acted on there.
            Transition::Sing(id) => {
                if self.party_picking {
                    // The browser was open to choose this round's song, not to sing one.
                    self.party_picking = false;
                    self.stack.pop();
                    self.offer_to_party(Some(id));
                    return;
                }
                self.next_plan = self.plain_plan();
                self.request_sing(id);
            }
        }
    }

    /// Start the USDB worker if it is not already running.
    fn start_usdb(&mut self) {
        if self.usdb.is_some() {
            return;
        }
        let songs = self
            .settings
            .game
            .song_roots
            .first()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.data_dir.join("songs"));
        let _ = std::fs::create_dir_all(&songs);
        self.usdb = Some(usdbjob::UsdbJob::start(
            self.data_dir.clone(),
            songs,
            self.settings
                .network
                .usdb_user
                .clone()
                .filter(|u| !u.is_empty()),
        ));
    }

    /// Carry out what the USDB browser asked for.
    fn apply_usdb_outcome(&mut self, outcome: UsdbOutcome) {
        use usdbjob::Order_;
        match outcome {
            UsdbOutcome::None => {}
            UsdbOutcome::Search(_) => {}
            UsdbOutcome::Sync => {
                self.start_usdb();
                if let Some(job) = &mut self.usdb {
                    job.send(Order_::Sync);
                }
            }
            UsdbOutcome::Download(id) => {
                self.start_usdb();
                if let Some(job) = &mut self.usdb {
                    job.send(Order_::Download(id));
                }
            }
            UsdbOutcome::Repair => {
                self.start_usdb();
                if let Some(job) = &mut self.usdb {
                    job.send(Order_::Repair);
                }
            }
            UsdbOutcome::Cancel => {
                if let Some(job) = &mut self.usdb {
                    job.cancel();
                }
            }
            UsdbOutcome::LogIn { user, password } => {
                self.start_usdb();
                // The username is a setting; the password never is. It goes to the OS keyring
                // and nowhere else, because a config file that quietly holds somebody's
                // password is how it ends up in a backup and a bug report.
                self.settings.network.usdb_user = Some(user.clone());
                self.save_settings();
                if let Some(job) = &mut self.usdb {
                    job.send(Order_::LogIn { user, password });
                }
            }
            UsdbOutcome::LogOut => {
                if let Some(job) = &mut self.usdb {
                    job.send(Order_::LogOut);
                }
            }
        }
    }

    /// Read what the USDB worker has said and fill the browser with it.
    fn refresh_usdb(&mut self) {
        let Some(job) = &mut self.usdb else {
            return;
        };
        let events = job.poll();
        let mut catalog_changed = false;
        let mut rescan = false;
        let mut doing: Option<(String, Option<f32>)> = None;
        let mut idle = false;
        let mut problem: Option<String> = None;
        let mut signed: Option<Option<String>> = None;
        let mut catalog_size: Option<usize> = None;
        let mut keeping: Option<usdbjob::Keeping> = None;
        for event in events {
            match event {
                usdbjob::Event::Doing(what, fraction) => doing = Some((what, fraction)),
                usdbjob::Event::CatalogChanged(size) => {
                    catalog_changed = true;
                    catalog_size = Some(size);
                }
                usdbjob::Event::Downloaded(id, folder, outcome) => {
                    tracing::info!("song {id} finished: {outcome:?}");
                    if folder.as_os_str().is_empty() {
                        continue;
                    }
                    // The song is on disk; the library has to be told, or it is invisible
                    // until the next launch.
                    rescan = true;
                    doing = Some((
                        match outcome {
                            rungstar_download::Outcome::Complete => "downloaded".to_owned(),
                            rungstar_download::Outcome::Partial => {
                                "downloaded, without everything".to_owned()
                            }
                            rungstar_download::Outcome::Cancelled => "stopped".to_owned(),
                        },
                        None,
                    ));
                }
                usdbjob::Event::Fetching(_) => catalog_changed = true,
                usdbjob::Event::Signed(who, how) => {
                    signed = Some(who);
                    keeping = Some(how);
                }
                usdbjob::Event::Problem(why) => problem = Some(why),
                usdbjob::Event::Idle => idle = true,
            }
        }

        let queued = job.queued();
        let fetching = job.fetching();
        let known: Vec<UsdbRow> = if catalog_changed
            || matches!(self.stack.last(), Some(Screen::Usdb(s)) if s.needs_rows())
        {
            let catalog = job.catalog();
            let text = match self.stack.last() {
                Some(Screen::Usdb(screen)) => screen.search_text().to_owned(),
                _ => String::new(),
            };
            catalog
                .search(&text)
                .into_iter()
                .take(500)
                .map(|song| {
                    let local = if Some(song.id) == fetching {
                        Local::Fetching
                    } else {
                        Local::Absent
                    };
                    UsdbRow::from_catalog(song, local)
                })
                .collect()
        } else {
            Vec::new()
        };
        let size = catalog_size.unwrap_or_else(|| job.catalog().len());

        if rescan {
            self.start_scan(false);
        }
        if let Some(who) = signed.clone() {
            self.settings.network.usdb_user = who.clone();
        }

        let Some(Screen::Usdb(screen)) = self.stack.last_mut() else {
            return;
        };
        screen.catalog_size = size;
        if catalog_changed || screen.needs_rows() {
            screen.set_rows(known);
        }
        if let Some(who) = signed {
            let named = who.is_some();
            screen.user = who;
            screen.problem.clear();
            // Said once, on signing in, rather than as a standing warning: a machine with no
            // password store will ask again when the session runs out, and somebody should
            // know that before it happens rather than be surprised by it.
            if named && keeping == Some(usdbjob::Keeping::SessionOnly) {
                screen.problem = "signed in \u{2014} this device has no password store, so you                                   will be asked again when the session expires"
                    .to_owned();
            }
        }
        if let Some((what, fraction)) = doing {
            screen.activity = Activity {
                what,
                fraction,
                queued,
            };
            screen.problem.clear();
        } else {
            screen.activity.queued = queued;
        }
        if idle {
            screen.activity = Activity {
                queued,
                ..Activity::default()
            };
        }
        if let Some(why) = problem {
            screen.problem = why;
        }
    }

    /// Carry out what the party screen asked for.
    fn apply_party_outcome(&mut self, outcome: PartyOutcome) {
        match outcome {
            PartyOutcome::None => {}
            PartyOutcome::Begin => self.begin_party(),
            PartyOutcome::Sing => self.sing_for_party(),
            PartyOutcome::Reroll => {
                if let Some(Screen::Party(screen)) = self.stack.last_mut() {
                    let spent = screen.party.as_mut().is_some_and(|party| party.reject());
                    if spent {
                        screen.offered = None;
                    }
                }
                let drawn = self.random_song();
                self.offer_to_party(drawn);
            }
            PartyOutcome::Choose => {
                // The browser, with everything it already does — search, filters, previews.
                // A party song picker that is a worse song list is not worth having.
                self.party_picking = true;
                self.stack.push(Screen::Songs(Box::new(SongSelect::new())));
                if self.library.count().unwrap_or(0) == 0 {
                    self.start_scan(false);
                }
            }
            PartyOutcome::Leave => {
                self.party_song = None;
                self.party_picking = false;
                self.party_scores = None;
            }
        }
    }

    /// Build the party or the bracket the setup stage describes, and start it.
    ///
    /// Teams are filled round-robin from the saved profiles rather than through a team-editing
    /// screen: with four people and two teams, first and third against second and fourth. It
    /// is the split anybody would make, and it costs no screen to make it.
    fn begin_party(&mut self) {
        let Some(Screen::Party(screen)) = self.stack.last_mut() else {
            return;
        };
        let pool = screen.pool.clone();
        if pool.len() < screen.size {
            return;
        }
        if screen.kind.is_tournament() {
            let players: Vec<String> = pool.into_iter().take(screen.size).collect();
            match rungstar_party::Bracket::new(players) {
                Ok(bracket) => {
                    screen.bracket = Some(bracket);
                    screen.party = None;
                }
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            }
        } else {
            let mut teams: Vec<rungstar_party::Team> = (0..screen.size)
                .map(|index| rungstar_party::Team::new(format!("Team {}", index + 1), Vec::new()))
                .collect();
            for (index, name) in pool.iter().enumerate() {
                teams[index % screen.size].players.push(name.clone());
            }
            let mut party = rungstar_party::Party::new(teams, screen.rounds);
            party.offer(String::new());
            screen.party = Some(party);
            screen.bracket = None;
        }
        self.challenge = screen.challenge();
        let classic = screen.kind == Kind::Classic;
        screen.to_round();
        // Classic draws a song and offers it; the other two ask for one.
        let drawn = classic.then(|| self.random_song()).flatten();
        self.offer_to_party(drawn);
    }

    /// Put a song in front of the team whose turn it is.
    fn offer_to_party(&mut self, song: Option<i64>) {
        let name = song
            .and_then(|id| self.library.song(id).ok().flatten())
            .map(|entry| entry.display_name());
        self.party_song = song.filter(|_| name.is_some());
        if let Some(Screen::Party(screen)) = self.stack.last_mut() {
            if let (Some(party), Some(name)) = (screen.party.as_mut(), name.clone()) {
                party.offer(name);
            }
            screen.offered = name;
            screen.to_round();
        }
    }

    /// Start the song the party is holding.
    fn sing_for_party(&mut self) {
        let Some(id) = self.party_song else {
            return;
        };
        let Some(Screen::Party(screen)) = self.stack.last_mut() else {
            return;
        };
        // Who is at the microphones this round, in slot order: one singer per team, or the two
        // players of the match. Everything downstream — panels, scoring, highscores — already
        // follows this list, so a party needs no separate idea of who is playing.
        let names: Vec<String> = match (&screen.bracket, &screen.party) {
            (Some(bracket), _) => bracket
                .next_match()
                .map(|(round, index)| {
                    let fixture = &bracket.rounds[round][index];
                    vec![
                        bracket.name(fixture.left).to_owned(),
                        bracket.name(fixture.right).to_owned(),
                    ]
                })
                .unwrap_or_default(),
            (_, Some(party)) => party
                .teams
                .iter()
                .map(|team| team.singer().unwrap_or_default().to_owned())
                .collect(),
            _ => Vec::new(),
        };
        if let Some(party) = screen.party.as_mut() {
            party.accept();
        }
        let effects = screen.challenge().effects;

        let known = self.profiles.players().unwrap_or_default();
        self.singers = names
            .iter()
            .filter_map(|name| {
                known
                    .iter()
                    .find(|player| player.name.eq_ignore_ascii_case(name))
                    .map(|player| player.id)
            })
            .collect();
        self.next_plan = session::Plan {
            effects,
            seed: unix_now() as u64,
            ..session::Plan::default()
        };
        self.pending_sing = Some(id);
    }

    /// Report a finished party song and move the party on.
    ///
    /// Called after the score screen closes rather than the moment the song ends, so the
    /// result is read before the standings change under it.
    fn report_party_round(&mut self) {
        let Some(scores) = self.party_scores.take() else {
            return;
        };
        let Some(Screen::Party(screen)) = self.stack.last_mut() else {
            return;
        };
        if let Some(bracket) = screen.bracket.as_mut() {
            if let Some((round, index)) = bracket.next_match() {
                let left = scores.first().copied().unwrap_or(0);
                let right = scores.get(1).copied().unwrap_or(0);
                bracket.report(round, index, (left, right));
            }
            if bracket.is_finished() {
                screen.to_finished();
                return;
            }
        } else if let Some(party) = screen.party.as_mut() {
            party.finish_round(&scores);
            if party.phase() == rungstar_party::Phase::Finished {
                screen.to_finished();
                return;
            }
        }
        screen.offered = None;
        self.party_song = None;
        let classic = screen.kind == Kind::Classic;
        screen.to_round();
        let drawn = classic.then(|| self.random_song()).flatten();
        self.offer_to_party(drawn);
    }

    /// A song at random from the whole library.
    ///
    /// Not filtered to what anybody knows: being handed something nobody has heard of is what
    /// the jokers are for, and a "random" that only ever offers the same fifty songs is not
    /// a party.
    fn random_song(&mut self) -> Option<i64> {
        let count = self.library.count().unwrap_or(0);
        if count == 0 {
            return None;
        }
        // The clock is the only entropy to hand, and it moves between rounds. Mixed rather
        // than used raw, because consecutive seconds otherwise give neighbouring songs.
        let mut seed = unix_now() as u64 ^ (self.party_song.unwrap_or(0) as u64).wrapping_mul(31);
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        seed = (seed ^ (seed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        seed = (seed ^ (seed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        let offset = ((seed ^ (seed >> 31)) % count.max(1) as u64) as usize;

        let mut query = SearchQuery::all().limit(1);
        query.offset = offset;
        self.library
            .search(&query)
            .ok()
            .and_then(|found| found.first().map(|entry| entry.id))
    }

    /// A plan for the whole song under whatever challenge the browser has chosen.
    ///
    /// The challenge lives on the browser rather than in the settings: it is a choice about
    /// the next song, not a preference, and a party that turned on Blind three weeks ago
    /// should not still be blind tonight.
    fn plain_plan(&mut self) -> session::Plan {
        self.challenge = match self.stack.last() {
            Some(Screen::Songs(songs)) => songs.challenge(),
            _ => self.challenge,
        };
        session::Plan {
            effects: self.challenge.effects,
            seed: unix_now() as u64,
            ..session::Plan::default()
        }
    }

    /// Where the chorus is, as a plan that starts there.
    ///
    /// `#MEDLEYSTARTBEAT` when the file has one, `#PREVIEWSTART` when it does not — the
    /// preview point is somebody's answer to "the bit worth hearing", which is the same
    /// question. Failing that, a third of the way in, which is roughly where a first chorus
    /// lands and is better than refusing.
    ///
    /// A song with no medley end runs to its own end rather than for a fixed length: cutting a
    /// chorus off after thirty seconds is worse than singing one verse too many.
    fn medley(&self, id: i64) -> Option<session::Plan> {
        let entry = self.library.song(id).ok().flatten()?;
        let bpm = rungstar_song::Bpm::new(entry.bpm);
        let gap = entry.gap_ms as f64;
        let at_beat = |beat: i32| bpm.beat_to_time(f64::from(beat), gap);

        let start = entry
            .medley_start
            .map(at_beat)
            .or(entry.preview_start)
            .or_else(|| (entry.duration_secs > 60.0).then(|| entry.duration_secs / 3.0))?;
        // A few beats of run-up, so the first word is not already going when the audio starts.
        let start = (start - 2.0).max(0.0);
        let end = entry.medley_end.map(at_beat);
        if end.is_some_and(|end| end <= start) {
            return None;
        }
        Some(session::Plan {
            start_secs: Some(start),
            end_secs: end,
            effects: self.challenge.effects,
            seed: unix_now() as u64,
        })
    }

    /// Ask who is singing, when there is anything to ask.
    ///
    /// One microphone and one profile leaves nothing to choose, and a screen offering no
    /// choice is just a keypress in the way. Two microphones is the case that matters: the
    /// scores have to land on the right people, and afterwards is too late to say who sang.
    fn request_sing(&mut self, id: i64) {
        let microphones = usize::from(self.settings.game.players.max(1));
        let profiles = self.profiles.players().map(|p| p.len()).unwrap_or(0);
        self.assume_the_only_singer();
        let duet = self
            .library
            .song(id)
            .ok()
            .flatten()
            .is_some_and(|song| song.is_duet);
        let always = self.settings.game.on_song_click == OnSongClick::SelectPlayers;
        if profiles == 0 || !(always || microphones > 1 || duet) {
            self.pending_sing = Some(id);
            return;
        }

        let Ok(Some(entry)) = self.library.song(id) else {
            self.pending_sing = Some(id);
            return;
        };
        let mut screen = PlayerScreen::new();
        screen.microphones = microphones;
        screen.singers = self.singers.clone();
        screen.for_song = Some(entry.display_name());
        // A duet names its parts, so the two rows above the Start button say who is singing
        // what rather than just how many are singing.
        screen.duet = entry.is_duet.then(|| duet_parts(&entry.path));
        self.refresh_players(&mut screen);
        self.pending_pick = Some(id);
        self.stack.push(Screen::Players(Box::new(screen)));
    }

    /// With one profile and nobody chosen, the one profile is who is singing.
    ///
    /// Otherwise somebody who made a profile and never visited the singer screen sings as
    /// "Player 1" and their score is thrown away at the end, which looks exactly like the
    /// highscore table being broken. Two profiles is a real question and is left to be asked.
    fn assume_the_only_singer(&mut self) {
        if !self.singers.is_empty() {
            return;
        }
        if let Ok(players) = self.profiles.players() {
            if let [only] = players.as_slice() {
                self.singers = vec![only.id];
            }
        }
    }

    /// Start singing a song.
    ///
    /// Everything that touches a device lives in the session; the screen is pure and draws
    /// what it is handed. That is why this is a screen on the same stack as the browser
    /// rather than a second window.
    fn sing(&mut self, id: i64, audio: &sdl3::AudioSubsystem, capture: SdlCapture) {
        // Also here, not only in the picker: a song can be started without passing through it.
        self.assume_the_only_singer();
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
        // The video is optional in every sense: the song may not name one, the file may not be
        // beside it, and the player may have turned videos off.
        let video = entry
            .video_file
            .as_ref()
            .filter(|_| self.settings.graphics.video_enabled == Switch::On)
            .and_then(|name| resolve_beside(&directory, name));

        let players = singing_players(
            self.singers.len(),
            usize::from(self.settings.game.players.max(1)),
        );

        let session = session::Session::start(
            audio,
            &parsed.song,
            &audio_path,
            players,
            match self.settings.game.difficulty {
                rungstar_ui::settings::Difficulty::Easy => rungstar_score::Difficulty::Easy,
                rungstar_ui::settings::Difficulty::Medium => rungstar_score::Difficulty::Medium,
                rungstar_ui::settings::Difficulty::Hard => rungstar_score::Difficulty::Hard,
            },
            self.settings.threshold(),
            self.settings.sound.mic_delay_ms as f64,
            &self.settings.sound.microphones,
            video.as_deref(),
            capture,
            &self.next_plan,
        );
        let session = match session {
            Ok(session) => session,
            Err(error) => {
                self.status = format!("could not start the song: {error}");
                return;
            }
        };

        let mut screen = SingScreen::new(&entry.artist, &entry.title, session.players());
        // Real names and their chosen colours, so the panels say who is who rather than
        // "Player 1". This is the whole reason profiles exist.
        for (index, singer) in screen.singers.iter_mut().enumerate() {
            if let Some(profile) = self
                .singers
                .get(index)
                .and_then(|id| self.profiles.player(*id).ok().flatten())
            {
                singer.name = profile.name;
            }
        }
        screen.show_input_panel = self.settings.advanced.input_panel == Switch::On;
        // Nobody is singing a jukebox song, so nobody has a panel and nothing is recorded.
        // The scorer still runs underneath — stopping it would be a second code path through
        // the session for no gain — but its result never leaves this screen.
        screen.show_panels = !self.jukebox;
        // What the challenge takes away is decided once, at the start: a mode that hid the
        // words halfway through would be a different mode.
        let effects = self.next_plan.effects;
        screen.show_lyrics = effects.lyrics;
        screen.show_notes = effects.notes;
        screen.challenge = (!effects.is_plain()).then(|| self.challenge.name.to_owned());
        screen.effect = self.settings.lyrics.effect;
        screen.duration = session.duration();
        let (low, high) = session.pitch_range();
        screen.pitch_low = low;
        screen.pitch_high = high;
        if self.settings.graphics.backgrounds == Switch::On {
            screen.background = self.covers.get(entry.id);
        }
        screen.video_size = self.settings.graphics.video_size;
        // A duet names its parts and splits the singers between them, so each gets a staff.
        screen.parts = session.part_names().to_vec();
        screen.singer_part = session.singer_parts().to_vec();
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
            Action::ImportUltrastar => self.import_ultrastar(),
            Action::WipeStatistics => match self.profiles.clear_scores() {
                Ok(0) => self.status = "there were no scores to delete".to_owned(),
                Ok(1) => self.status = "deleted 1 score".to_owned(),
                Ok(count) => self.status = format!("deleted {count} scores"),
                Err(error) => self.status = error.to_string(),
            },
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
                // One part for an ordinary song, two for a duet. Gathered first because the
                // screen borrows them all at once.
                let gathered: Vec<(rungstar_ui::singscreen::NoteLine, Vec<_>, String)> = (0
                    ..session.part_count())
                    .map(|part| {
                        let (syllables, next) = session.lyrics(part, beat);
                        (session.current_line(part, beat), syllables, next)
                    })
                    .collect();
                let parts: Vec<rungstar_ui::singscreen::PartView<'_>> = gathered
                    .iter()
                    .map(
                        |(line, syllables, next)| rungstar_ui::singscreen::PartView {
                            line,
                            syllables,
                            next_line: next,
                        },
                    )
                    .collect();
                screen.draw(list, area, &self.style, &parts, beat);
            }
            Some(Screen::Mics(screen, _)) => screen.draw(list, area, &self.style),
            Some(Screen::Players(screen)) => screen.draw(list, area, &self.style),
            Some(Screen::Party(screen)) => screen.draw(list, area, &self.style),
            Some(Screen::Usdb(screen)) => screen.draw(list, area, &self.style),
            Some(Screen::Stats(screen)) => screen.draw(list, area, &self.style),
            Some(Screen::About) => draw_about(list, area, &self.style),
            None => {}
        }
    }
}

/// Seconds since the epoch, for anything that records when it happened.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// How many people are being scored this song.
///
/// Who is singing, when anybody has been chosen. Two microphones assigned and one person who
/// turned up is an ordinary evening, and a second panel scoring zero all the way through is
/// not a useful thing to show them. Nobody chosen falls back to the microphone count, because
/// singing without a profile has to keep working.
fn singing_players(chosen: usize, microphones: usize) -> usize {
    match chosen {
        0 => microphones.max(1),
        chosen => chosen.min(microphones.max(1)),
    }
}

/// The two part names of a duet, for the singer picker.
///
/// Read from the file rather than the index: the index records that a song *is* a duet but not
/// what its parts are called, and the real names are worth a file read that happens once, at
/// the moment somebody is deciding who sings which.
fn duet_parts(path: &Path) -> (String, String) {
    let parsed = std::fs::read(path)
        .ok()
        .and_then(|bytes| rungstar_song::SongTxt::parse_bytes(&bytes).ok());
    let Some(parsed) = parsed else {
        return ("Part 1".to_owned(), "Part 2".to_owned());
    };
    let headers = &parsed.song.headers;
    (
        headers.p1.clone().unwrap_or_else(|| "Part 1".to_owned()),
        headers.p2.clone().unwrap_or_else(|| "Part 2".to_owned()),
    )
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
        Keycode::D => Input::CycleFilter,
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
                        if let Some(mut input) = action_for(key) {
                            // Enter on a real keyboard finishes the search. Somebody typing
                            // is not looking at the on-screen keyboard's cursor, so pressing
                            // whatever key happens to be under it is never what they meant.
                            if app.wants_text() && matches!(key, Keycode::Return | Keycode::KpEnter)
                            {
                                input = Input::Submit;
                            }
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
                        Button::LeftStick => Some(Input::CycleFilter),
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
            // Always the full ceiling: the player count is set *by* assigning channels here,
            // so capping the cycle at the current count would make it impossible to grow.
            let mut screen = MicScreen::new();
            screen.split_channels = app.settings.sound.split_channels == Switch::On;
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

        if let Some(id) = app.dropped_video.take() {
            renderer.drop_image(id);
        }
        app.poll_scan();
        let scanning = app.scanning().then(|| app.status.clone());
        if let Some(Screen::Songs(songs)) = app.stack.last_mut() {
            songs.scanning = scanning;
        }
        app.handle_song_menu();
        app.refresh_stats();
        app.refresh_facets();
        app.refresh_songs();
        app.refresh_usdb();
        app.refresh_highscores();
        app.update_preview(&audio_subsystem);
        app.load_visible_covers(&mut renderer);
        let mut finished_song = false;
        let mut next_jukebox_song = false;
        // Read before the sing screen is borrowed, since that borrow takes the whole app.
        let jukebox = app.jukebox;
        match app.stack.last_mut() {
            Some(Screen::Songs(songs)) => {
                songs.tick(dt);
            }
            Some(Screen::Mics(screen, monitor)) => {
                monitor.tick();
                screen.devices = monitor.devices();
            }
            Some(Screen::Sing(screen, session)) => {
                // The video frame for this moment, uploaded into one texture that is reused
                // for the whole song rather than a new one thirty times a second.
                if let Some(frame) = session.video_frame() {
                    let uploaded = match screen.video {
                        Some(id) => renderer
                            .update_image(id, frame.width, frame.height, &frame.rgba)
                            .map(|()| Some(id)),
                        None => renderer
                            .add_image(frame.width, frame.height, &frame.rgba)
                            .map(Some),
                    };
                    match uploaded {
                        Ok(id) => screen.video = id,
                        Err(error) => {
                            tracing::warn!("video frame not shown: {error}");
                            screen.video = None;
                        }
                    }
                }
                if let Some(aspect) = session.video_aspect() {
                    screen.video_aspect = aspect;
                }
                if let Err(error) = session.tick() {
                    tracing::warn!("playback stopped: {error}");
                    session.stop();
                    app.apply(Transition::Pop);
                } else {
                    session.update_singers(&mut screen.singers);
                    screen.position = session.position();
                    // The challenge state, refreshed per frame: the bar rises with the beat
                    // and the music cuts in and out under the Deaf mode.
                    let watch = session.watch();
                    screen.bar = watch.bar_at(session.visual_beat());
                    screen.knocked_out = watch.standings().iter().map(|s| s.is_out()).collect();
                    screen.audible = session.audible();
                    // Once the last note has gone by there is nothing left to sing, so the
                    // screen offers to skip the rest of the instrumental.
                    screen.outro = session.past_last_note();
                    if session.is_finished() && screen.overlay != Overlay::Results {
                        session.stop();
                        if jukebox {
                            // No score screen and nothing recorded: the next song starts.
                            // A jukebox that stops to congratulate somebody every four minutes
                            // is not background music.
                            next_jukebox_song = true;
                        } else {
                            // Recorded outside the borrow, since writing needs the whole app.
                            finished_song = true;
                            // The scores go up rather than the screen closing: in a party the
                            // result is the point, and popping straight back to the browser
                            // throws it away before anybody has read it.
                            screen.overlay = Overlay::Results;
                        }
                    }
                }
            }
            _ => {}
        }

        if finished_song {
            app.record_finished_song();
        }
        if next_jukebox_song {
            // Popped and re-pushed rather than restarted in place: a new song is a new
            // session, a new video and a new set of notes, and `sing` already does all of it.
            app.apply(Transition::Pop);
            app.jukebox = true;
            if let Some(id) = app.random_song() {
                app.pending_sing = Some(id);
            } else {
                app.jukebox = false;
            }
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
        app.refresh_stats();
        app.refresh_facets();
        app.refresh_songs();
        app.refresh_highscores();
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

    // The two profile screens, with a profile and a table so the layouts are exercised rather
    // than only their empty states.
    {
        let mut screen = PlayerScreen::new();
        screen.microphones = 2;
        screen.players = vec![
            Entry {
                id: 1,
                name: "Walki".into(),
                colour: 0,
                songs: 12,
                best: 8800,
            },
            Entry {
                id: 2,
                name: "Anna".into(),
                colour: 1,
                songs: 3,
                best: 9100,
            },
        ];
        screen.singers = vec![1];
        list.clear();
        screen.draw(list, area, &app.style);
        renderer
            .render(list, app.style.background)
            .map_err(|e| anyhow::anyhow!("players: {e}"))?;
        println!("players     {} draw commands", list.len());
    }
    {
        let mut screen = StatsScreen::new();
        screen.set_rows(
            (0..6)
                .map(|i| StatRow {
                    label: format!("Artist {i} \u{2013} Song {i}"),
                    detail: "Walki".into(),
                    value: (9000 - i * 100).to_string(),
                })
                .collect(),
        );
        list.clear();
        screen.draw(list, area, &app.style);
        renderer
            .render(list, app.style.background)
            .map_err(|e| anyhow::anyhow!("stats: {e}"))?;
        println!("statistics  {} draw commands", list.len());
    }

    // The microphone screen, with a device that has never produced a sample -- the state a
    // player is in when they open it to find out why nothing is scoring.
    {
        let mut screen = rungstar_ui::micscreen::MicScreen::new();
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
                part: 0,
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
        // A synthetic video frame, so the letterboxing and the scrim are exercised even on a
        // machine with no songs.
        let checker: Vec<u8> = (0..64 * 36 * 4)
            .map(|i| {
                if (i / 4 / 8 + i / 4 / 64 / 8) % 2 == 0 {
                    200
                } else {
                    40
                }
            })
            .collect();
        if let Ok(id) = renderer.add_image(64, 36, &checker) {
            screen.video = Some(id);
            screen.video_aspect = 16.0 / 9.0;
        }

        for overlay in [Overlay::None, Overlay::Paused, Overlay::Results] {
            screen.overlay = overlay;
            list.clear();
            let parts = [rungstar_ui::singscreen::PartView {
                line: &line,
                syllables: &syllables,
                next_line: "next line",
            }];
            screen.draw(list, area, &app.style, &parts, 20.0);
            if !list.is_balanced() {
                anyhow::bail!("the sing screen left a clip pushed");
            }
            renderer
                .render(list, app.style.background)
                .map_err(|e| anyhow::anyhow!("sing: {e}"))?;
        }
        println!("sing        {} draw commands, 6 singers", list.len());
    }

    // The party screen, at every stage it has.
    {
        let mut screen = PartyScreen::new();
        screen.pool = vec!["Ada".into(), "Grace".into()];
        screen.party = Some(rungstar_party::Party::new(
            vec![
                rungstar_party::Team::new("Team 1", vec!["Ada".into()]),
                rungstar_party::Team::new("Team 2", vec!["Grace".into()]),
            ],
            3,
        ));
        screen.offered = Some("Abba - Waterloo".to_owned());
        app.stack.push(Screen::Party(Box::new(screen)));
        for stage in [Stage::Setup, Stage::Round, Stage::Finished] {
            if let Some(Screen::Party(screen)) = app.stack.last_mut() {
                screen.stage = stage;
            }
            list.clear();
            app.draw(list, area);
            renderer
                .render(list, app.style.background)
                .map_err(|e| anyhow::anyhow!("party: {e}"))?;
        }
        println!("party       {} draw commands", list.len());
        app.stack.pop();
    }

    // The USDB browser, empty and full, and its sign-in.
    {
        let mut screen = UsdbScreen::new();
        screen.catalog_size = 2;
        screen.set_rows(vec![UsdbRow {
            id: rungstar_usdb::SongId(1),
            artist: "Abba".into(),
            title: "Waterloo".into(),
            language: "English".into(),
            year: Some(1974),
            rating: 4.5,
            golden: true,
            local: Local::Absent,
        }]);
        screen.activity = Activity {
            what: "fetching".into(),
            fraction: Some(0.5),
            queued: 1,
        };
        app.stack.push(Screen::Usdb(Box::new(screen)));
        for step in [Input::Back, Input::Search, Input::Back, Input::CycleFilter] {
            list.clear();
            app.draw(list, area);
            renderer
                .render(list, app.style.background)
                .map_err(|e| anyhow::anyhow!("usdb: {e}"))?;
            if let Some(Screen::Usdb(screen)) = app.stack.last_mut() {
                screen.handle(step);
            }
        }
        println!("usdb        {} draw commands", list.len());
        app.stack.pop();
    }

    // And the browser overlays, which have their own layout maths.
    app.stack.push(Screen::Songs(Box::new(SongSelect::new())));
    app.refresh_facets();
    for overlay in [
        Input::Search,
        Input::Search,
        Input::Sort,
        Input::Sort,
        Input::CycleFilter,
        Input::CycleFilter,
        Input::ContextMenu,
        // Into the value column, so the panel is drawn with a real list of genres behind it
        // rather than only its categories.
        Input::Right,
        Input::Down,
    ] {
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
    fn the_number_singing_follows_who_was_chosen() {
        // Nobody chosen: everybody with a microphone sings, which is how it worked before
        // profiles existed and has to keep working.
        assert_eq!(singing_players(0, 2), 2);
        assert_eq!(singing_players(0, 0), 1);

        // One person chosen with two microphones assigned is one panel, not two. This is the
        // case that showed up as a second player scoring zero for the whole song.
        assert_eq!(singing_players(1, 2), 1);
        assert_eq!(singing_players(2, 2), 2);

        // And more chosen than there are microphones cannot conjure one: the extra singer
        // would have nothing to sing into.
        assert_eq!(singing_players(4, 2), 2);
    }

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
