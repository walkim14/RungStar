//! RungStar: the game.
//!
//! Owns the window, the screen stack and the library, and does the three things a screen
//! cannot do for itself — run a query, load a cover, save the settings. Screens are pure state
//! that produce a display list; this file is where that meets a device.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;

use rungstar_library::{scan, Database, ScanOptions, SearchQuery, SongEntry};
use rungstar_platform::font::FontSet;
use rungstar_platform::render::Renderer;
use rungstar_ui::draw::{DrawList, ImageId, TextStyle};
use rungstar_ui::geom::Rect;
use rungstar_ui::menus::{MainMenu, OptionsOutcome, OptionsScreen};
use rungstar_ui::options::Action;
use rungstar_ui::screen::{Route, Transition, Widgets};
use rungstar_ui::settings::{ScreenMode, Settings, Switch};
use rungstar_ui::songselect::{Input, SongSelect};
use rungstar_ui::theme::{Style, Theme};
use rungstar_ui::Color;

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
    running: bool,
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

    /// Bring the index in line with the disk.
    fn rescan(&mut self, verify: bool) {
        let roots = self.song_roots();
        for root in &roots {
            let _ = std::fs::create_dir_all(root);
        }
        let mut options = ScanOptions::new(roots);
        options.verify = verify;
        let started = Instant::now();
        match scan(&mut self.library, &options) {
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
            .limit(5000);
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
                        self.rescan(false);
                    }
                }
                Route::Options | Route::OptionsPage(_) => self
                    .stack
                    .push(Screen::Options(Box::new(OptionsScreen::new()))),
                Route::About => self.stack.push(Screen::About),
                Route::Main | Route::Search => {}
            },
            Transition::Sing(id) => {
                // Playing a song is the sing screen's job, and it is still a separate binary.
                // Recording the play here keeps the count honest in the meantime.
                let _ = self.library.record_play(id);
                if let Ok(Some(song)) = self.library.song(id) {
                    self.status = format!("would sing: {}", song.display_name());
                }
            }
        }
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::RescanLibrary => self.rescan(false),
            Action::RebuildIndex => self.rescan(true),
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
            Action::ManageMicrophones => {
                self.status = "microphone setup arrives with multi-singer support".to_owned()
            }
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
            Some(Screen::About) => draw_about(list, area, &self.style),
            None => {}
        }
    }
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

/// Map a key or button to the semantic input the screens understand.
fn action_for(keycode: Keycode) -> Option<Input> {
    Some(match keycode {
        Keycode::Up => Input::Up,
        Keycode::Down => Input::Down,
        Keycode::Left => Input::Left,
        Keycode::Right => Input::Right,
        Keycode::Return | Keycode::KpEnter => Input::Confirm,
        Keycode::Escape => Input::Back,
        Keycode::Tab => Input::CycleLayout,
        Keycode::F3 => Input::Sort,
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

    let mut events = sdl.event_pump().map_err(|e| anyhow::anyhow!("{e}"))?;
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
                    if let Some(input) = action_for(key) {
                        app.handle(input, area);
                    }
                }
                Event::TextInput { text, .. } => {
                    // Typed characters only reach the search field; screens that are not
                    // editing ignore them.
                    for c in text.chars() {
                        app.handle(Input::Type(c), area);
                    }
                }
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Ok(pad) = gamepads.open(sdl3::joystick::JoystickId::new(which)) {
                        open_pads.push(pad);
                    }
                }
                Event::ControllerButtonDown { button, .. } => {
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

        app.refresh_songs();
        app.load_visible_covers(&mut renderer);
        if let Some(Screen::Songs(songs)) = app.stack.last_mut() {
            songs.tick(dt);
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

    // And the two overlays, which are the parts with their own layout maths.
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
