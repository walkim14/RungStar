//! The USDB worker: one thread that owns the network, the catalog and the download queue.
//!
//! Nothing here runs on the frame loop. A catalog sync is three hundred requests and a
//! download is a subprocess pulling sixty megabytes; either one on the main thread is a frozen
//! window, and a frozen window is indistinguishable from a crash. The screen sends orders down
//! a channel and reads events back, exactly as the library scan already does.
//!
//! One worker, not a pool. USDB is a single volunteer-run PHP box and the point of the rate
//! limiter is to be a good guest; four threads racing each other past it would defeat it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use rungstar_download::pipeline::{download, Fetcher, Progress};
use rungstar_download::{plan, Kind, Outcome, SyncMeta};
use rungstar_usdb::client::{Http, Order};
use rungstar_usdb::{Catalog, Credentials, Session, SongId, Usdb, UsdbError};

/// What the screen asks for.
#[derive(Debug, Clone)]
pub enum Order_ {
    Sync,
    Download(SongId),
    LogIn { user: String, password: String },
    LogOut,
    Repair,
}

/// How a sign-in is being kept between launches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keeping {
    /// The OS password store. Windows and macOS always have one.
    Keyring,
    /// No password store on this machine, so the session cookie is kept instead. It lasts
    /// until USDB expires it, and then the password has to be typed once more.
    SessionOnly,
}

/// What the worker reports.
#[derive(Debug, Clone)]
pub enum Event {
    /// Something is happening, said in a few words, with a fraction where one is known.
    Doing(String, Option<f32>),
    /// The catalog changed and the screen should re-read it.
    CatalogChanged(usize),
    /// A song finished, complete or not.
    Downloaded(SongId, PathBuf, Outcome),
    /// A song is being fetched now.
    Fetching(SongId),
    /// Who is logged in, and how that is being remembered.
    Signed(Option<String>, Keeping),
    /// Something went wrong, in words a person can act on.
    Problem(String),
    /// Nothing is happening any more.
    Idle,
}

/// The handle the application holds.
pub struct UsdbJob {
    orders: Sender<Order_>,
    events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    /// The catalog, shared so the screen can search it without asking the worker.
    catalog: Arc<std::sync::Mutex<Catalog>>,
    queued: usize,
    fetching: Option<SongId>,
}

impl UsdbJob {
    /// Start the worker.
    ///
    /// `data` is where the catalog is kept; `songs` is the root new songs are written under;
    /// `scratch` is where they are built first — under the song root on purpose, so the move
    /// into place is a rename on the same volume rather than a copy.
    pub fn start(data: PathBuf, songs: PathBuf, user: Option<String>) -> Self {
        let (order_tx, order_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let catalog = Arc::new(std::sync::Mutex::new(
            Catalog::load(&data.join("usdb-catalog.json")).unwrap_or_default(),
        ));

        let worker_cancel = Arc::clone(&cancel);
        let worker_catalog = Arc::clone(&catalog);
        std::thread::Builder::new()
            .name("usdb".to_owned())
            .spawn(move || {
                run(
                    data,
                    songs,
                    user,
                    order_rx,
                    event_tx,
                    worker_cancel,
                    worker_catalog,
                )
            })
            .expect("the USDB worker thread could not be started");

        Self {
            orders: order_tx,
            events: event_rx,
            cancel,
            catalog,
            queued: 0,
            fetching: None,
        }
    }

    pub fn catalog(&self) -> std::sync::MutexGuard<'_, Catalog> {
        self.catalog.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn send(&mut self, order: Order_) {
        if matches!(order, Order_::Download(_)) {
            self.queued += 1;
        }
        // A new order clears a cancel left over from the last one.
        self.cancel.store(false, Ordering::Relaxed);
        let _ = self.orders.send(order);
    }

    /// Stop whatever is running. The flag is read between steps and by the extractor.
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.queued = 0;
    }

    pub fn queued(&self) -> usize {
        self.queued
    }

    pub fn fetching(&self) -> Option<SongId> {
        self.fetching
    }

    /// Everything the worker has said since last time.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            match &event {
                Event::Fetching(id) => self.fetching = Some(*id),
                Event::Downloaded(..) => {
                    self.fetching = None;
                    self.queued = self.queued.saturating_sub(1);
                }
                Event::Idle => self.fetching = None,
                _ => {}
            }
            events.push(event);
        }
        events
    }
}

/// Plain HTTP, for covers and artwork.
struct Files {
    agent: ureq::Agent,
}

impl Fetcher for Files {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut response = self.agent.get(url).call().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut response.body_mut().as_reader(), &mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }
}

struct Flag(Arc<AtomicBool>);

impl rungstar_download::Stop for Flag {
    fn stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    data: PathBuf,
    songs: PathBuf,
    user: Option<String>,
    orders: Receiver<Order_>,
    events: Sender<Event>,
    cancel: Arc<AtomicBool>,
    catalog: Arc<std::sync::Mutex<Catalog>>,
) {
    let mut usdb = Usdb::new(Http::new());
    let files = Files {
        agent: ureq::Agent::new_with_defaults(),
    };
    let extractor = rungstar_download::YtDlp::default();
    let stop = Flag(Arc::clone(&cancel));
    let catalog_path = data.join("usdb-catalog.json");
    let scratch = songs.join(".rungstar-downloads");

    let session_path = data.join(rungstar_usdb::session_file::FILE);
    // Windows and macOS always have a password store. Linux needs a D-Bus Secret Service,
    // which is a desktop session service — a Steam Deck in Game Mode has none, and neither
    // does a kiosk or a container. Probed once, because the answer decides what is kept and
    // what the screen has to tell somebody.
    let keeping = if rungstar_usdb::secret::available() {
        Keeping::Keyring
    } else {
        Keeping::SessionOnly
    };

    // The saved cookie first, on every platform. It signs in with no password at all, and on
    // a machine with a keyring it means the password is only read when the session has
    // actually expired rather than on every launch.
    let mut signed_in = false;
    if let Err(error) = rungstar_usdb::session_file::load(usdb.transport().agent(), &session_path) {
        tracing::warn!("the saved USDB session could not be read: {error}");
    } else if let Ok(Session::LoggedIn(who)) = usdb.who_am_i() {
        signed_in = true;
        let _ = events.send(Event::Signed(Some(who), keeping));
    }

    // Only then the password, and only where there is a store to have kept one.
    if !signed_in && keeping == Keeping::Keyring {
        if let Some(name) = &user {
            match rungstar_usdb::secret::password(name) {
                Ok(Some(password)) => {
                    let credentials = Credentials {
                        user: name.clone(),
                        password,
                    };
                    match usdb.log_in(&credentials) {
                        Ok(Session::LoggedIn(who)) => {
                            let _ = rungstar_usdb::session_file::save(
                                usdb.transport().agent(),
                                &session_path,
                            );
                            let _ = events.send(Event::Signed(Some(who), keeping));
                        }
                        Ok(Session::Anonymous) | Err(UsdbError::BadCredentials) => {
                            // A stored password that no longer works is worse than none: it
                            // fails silently on every launch and nothing says why.
                            let _ = rungstar_usdb::secret::forget(name);
                            let _ = events.send(Event::Problem(
                                "the saved USDB password no longer works; sign in again".to_owned(),
                            ));
                        }
                        Err(error) => {
                            let _ = events.send(Event::Problem(error.to_string()));
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = events.send(Event::Problem(error.to_string()));
                }
            }
        }
    }

    while let Ok(order) = orders.recv() {
        match order {
            Order_::LogIn { user, password } => {
                let _ = events.send(Event::Doing("signing in".to_owned(), None));
                let credentials = Credentials {
                    user: user.clone(),
                    password: password.clone(),
                };
                match usdb.log_in(&credentials) {
                    Ok(Session::LoggedIn(who)) => {
                        // The cookie is kept on every machine: it is what makes the next
                        // launch already signed in without the password being read at all.
                        if let Err(error) = rungstar_usdb::session_file::save(
                            usdb.transport().agent(),
                            &session_path,
                        ) {
                            tracing::warn!("the USDB session could not be kept: {error}");
                        }
                        // The password only where there is somewhere safe to put it. On a
                        // machine with no store it is simply not written down — an obfuscated
                        // copy in the data directory would be the same password with a step in
                        // front of it, and the key would be in the binary.
                        if keeping == Keeping::Keyring {
                            if let Err(error) = rungstar_usdb::secret::remember(&user, &password) {
                                let _ = events.send(Event::Problem(error.to_string()));
                            }
                        }
                        let _ = events.send(Event::Signed(Some(who), keeping));
                    }
                    Ok(Session::Anonymous) => {
                        let _ = events.send(Event::Problem(
                            "USDB did not accept that sign-in".to_owned(),
                        ));
                    }
                    Err(error) => {
                        let _ = events.send(Event::Problem(error.to_string()));
                    }
                }
                let _ = events.send(Event::Idle);
            }
            Order_::LogOut => {
                if let Some(name) = &user {
                    let _ = rungstar_usdb::secret::forget(name);
                }
                let _ = rungstar_usdb::session_file::forget(&session_path);
                let _ = usdb.page(&rungstar_usdb::Endpoint::Logout);
                let _ = events.send(Event::Signed(None, keeping));
            }
            Order_::Sync => {
                sync(&mut usdb, &catalog, &catalog_path, &cancel, &events);
                let _ = events.send(Event::Idle);
            }
            Order_::Download(id) => {
                let _ = events.send(Event::Fetching(id));
                match fetch_one(
                    &mut usdb, id, &songs, &scratch, &files, &extractor, &stop, &events, &catalog,
                ) {
                    Ok((folder, outcome)) => {
                        let _ = events.send(Event::Downloaded(id, folder, outcome));
                    }
                    Err(error) => {
                        let _ = events.send(Event::Problem(error));
                        let _ =
                            events.send(Event::Downloaded(id, PathBuf::new(), Outcome::Cancelled));
                    }
                }
                let _ = events.send(Event::Idle);
            }
            Order_::Repair => {
                let broken = rungstar_download::needs_repair(&songs);
                if broken.is_empty() {
                    let _ = events.send(Event::Doing(
                        "nothing in the library is missing a file".to_owned(),
                        None,
                    ));
                }
                for (index, (_, id, missing)) in broken.iter().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let names: Vec<&str> = missing.iter().map(|k| k.label()).collect();
                    let _ = events.send(Event::Doing(
                        format!("repairing {id}: {}", names.join(", ")),
                        Some((index as f32 + 1.0) / broken.len() as f32),
                    ));
                    let _ = fetch_one(
                        &mut usdb, *id, &songs, &scratch, &files, &extractor, &stop, &events,
                        &catalog,
                    );
                }
                let _ = events.send(Event::Idle);
            }
        }
    }
}

fn sync(
    usdb: &mut Usdb<Http>,
    catalog: &Arc<std::sync::Mutex<Catalog>>,
    path: &std::path::Path,
    cancel: &Arc<AtomicBool>,
    events: &Sender<Event>,
) {
    // The high-water mark from *before* the sync. Newest-first ordering means the crawl can
    // stop as soon as a whole page is older than this, so a daily sync is one or two requests
    // rather than three hundred.
    let before = catalog.lock().map(|c| c.high_water()).unwrap_or(0);
    let mut report = rungstar_usdb::SyncReport::default();
    let result = usdb.catalog(Order::LastChange, |page| {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let mut held = match catalog.lock() {
            Ok(held) => held,
            Err(poisoned) => poisoned.into_inner(),
        };
        held.absorb(page, &mut report);
        let caught_up = before > 0 && held.caught_up(page, before);
        let total = held.len();
        drop(held);
        let _ = events.send(Event::Doing(
            format!("{total} songs, {} new", report.added),
            None,
        ));
        !caught_up
    });

    match result {
        Ok(_) => {
            let held = catalog.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(error) = held.save(path) {
                let _ = events.send(Event::Problem(error.to_string()));
            }
            let _ = events.send(Event::CatalogChanged(held.len()));
        }
        Err(error) => {
            let _ = events.send(Event::Problem(error.to_string()));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fetch_one(
    usdb: &mut Usdb<Http>,
    id: SongId,
    songs: &std::path::Path,
    scratch: &std::path::Path,
    files: &Files,
    extractor: &dyn rungstar_download::Extractor,
    stop: &dyn rungstar_download::Stop,
    events: &Sender<Event>,
    catalog: &Arc<std::sync::Mutex<Catalog>>,
) -> Result<(PathBuf, Outcome), String> {
    let _ = events.send(Event::Doing(format!("fetching song {id}"), None));
    let txt = usdb.song_txt(id).map_err(|e| match e {
        UsdbError::NotLoggedIn => {
            "USDB only gives out song files to signed-in users. Sign in and try again.".to_owned()
        }
        other => other.to_string(),
    })?;
    let parsed = rungstar_song::SongTxt::parse_bytes(txt.as_bytes())
        .map_err(|e| format!("USDB sent a song this build cannot read: {e}"))?;

    // The detail page is only worth a request when the song file leaves something out — a
    // cover or a video that has to come from the comments. Most songs name both.
    let wants_details =
        parsed.song.meta_tags.video.is_none() || parsed.song.meta_tags.cover.is_none();
    let details = wants_details.then(|| usdb.details(id).ok()).flatten();

    let folder_guess = songs.join(plan::safe_name(&format!(
        "{} - {}",
        parsed.song.headers.artist, parsed.song.headers.title
    )));
    let held = SyncMeta::read(&folder_guess);
    let plan = plan::plan(
        id,
        &txt,
        &parsed.song,
        details.as_ref(),
        held.as_ref(),
        &folder_guess,
    );

    let usdb_mtime = catalog
        .lock()
        .ok()
        .and_then(|held| held.get(id).map(|song| song.last_change))
        .unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);

    let steps = plan.steps.len().max(1) as f32;
    let mut done = 0.0;
    let report = download(
        &plan,
        songs,
        scratch,
        usdb_mtime,
        now,
        files,
        extractor,
        stop,
        |progress| match progress {
            Progress::Started(kind) => {
                let _ = events.send(Event::Doing(
                    format!("{} \u{2014} {}", plan.title, kind.label()),
                    Some(done / steps),
                ));
            }
            Progress::Finished(_) => done += 1.0,
            Progress::Missed(kind, why) => {
                done += 1.0;
                // Not a failure: the song is still coming. Worth saying once.
                tracing::warn!("no {} for {}: {why}", kind.label(), plan.title);
            }
            Progress::Playable(_) => {
                let _ = events.send(Event::Doing(
                    format!("{} is ready to sing", plan.title),
                    Some(done / steps),
                ));
            }
        },
    )
    .map_err(|e| e.to_string())?;

    // Something optional missing is worth knowing about, quietly.
    for (kind, why) in &report.missing {
        tracing::info!("{} has no {}: {why}", plan.title, kind.label());
    }
    let _ = Kind::ALL;
    Ok((report.folder, report.outcome))
}
