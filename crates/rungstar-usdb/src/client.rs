//! Talking to USDB.
//!
//! The network is behind [`Transport`] so the protocol above it can be driven by a test with
//! saved pages — which is the only way any of this is testable without an account, and the
//! reason the whole crate can be finished before anybody logs in.

use std::time::{Duration, Instant};

use crate::parse;
use crate::rate::{backoff, Limiter, Rate, RETRIES};
use crate::{CatalogSong, SongDetails, SongId, UsdbError};

/// Where USDB lives.
pub const BASE_URL: &str = "https://usdb.animux.de/";
/// USDB pages the catalog a hundred songs at a time and will not give more.
pub const PAGE_SIZE: usize = 100;
/// The site's own ceiling on song ids, which bounds a full crawl.
pub const MAX_SONG_ID: i64 = 100_000;

/// One request, named by the `link=` that dispatches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A page of the catalog.
    List {
        start: usize,
        order: Order,
    },
    /// One song's detail page.
    Detail(SongId),
    /// One song's note file.
    Txt(SongId),
    /// Whoever is logged in.
    Profile,
    Login {
        user: String,
        password: String,
    },
    Logout,
}

/// How the catalog is ordered, which is what makes an incremental sync possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// Most recently edited first. A sync walks this until it reaches songs it already has.
    #[default]
    LastChange,
    Artist,
    Title,
    Rating,
}

impl Order {
    fn key(self) -> &'static str {
        match self {
            Self::LastChange => "lastchange",
            Self::Artist => "interpret",
            Self::Title => "title",
            Self::Rating => "rating",
        }
    }

    fn descending(self) -> bool {
        matches!(self, Self::LastChange | Self::Rating)
    }
}

impl Endpoint {
    /// The query parameters, the form body, and whether it is a POST.
    ///
    /// Everything is `index.php`; the `link=` parameter is the whole of USDB's routing.
    pub fn request(&self) -> Request {
        match self {
            Self::List { start, order } => Request {
                post: true,
                params: vec![("link".into(), "list".into())],
                body: vec![
                    ("order".into(), order.key().into()),
                    (
                        "ud".into(),
                        if order.descending() { "desc" } else { "asc" }.into(),
                    ),
                    ("limit".into(), PAGE_SIZE.to_string()),
                    ("details".into(), "1".into()),
                    ("start".into(), start.to_string()),
                ],
            },
            Self::Detail(id) => Request {
                post: false,
                params: vec![
                    ("link".into(), "detail".into()),
                    ("id".into(), id.to_string()),
                ],
                body: Vec::new(),
            },
            // A POST with `wd=1`, which is what makes the page return the file rather than a
            // download prompt. Found by reading the reference; it is nowhere documented.
            Self::Txt(id) => Request {
                post: true,
                params: vec![
                    ("link".into(), "gettxt".into()),
                    ("id".into(), id.to_string()),
                ],
                body: vec![("wd".into(), "1".into())],
            },
            Self::Profile => Request {
                post: false,
                params: vec![("link".into(), "profil".into())],
                body: Vec::new(),
            },
            Self::Login { user, password } => Request {
                post: true,
                params: Vec::new(),
                body: vec![
                    ("user".into(), user.clone()),
                    ("pass".into(), password.clone()),
                    ("login".into(), "Login".into()),
                ],
            },
            Self::Logout => Request {
                post: true,
                params: vec![("link".into(), "logout".into())],
                body: Vec::new(),
            },
        }
    }
}

/// A request in the shape a transport needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub post: bool,
    pub params: Vec<(String, String)>,
    pub body: Vec<(String, String)>,
}

/// Whatever actually fetches pages.
///
/// One method, because that is all USDB needs, and it hands back the body as text. Cookies and
/// redirects are the transport's business.
pub trait Transport {
    fn fetch(&self, request: &Request) -> Result<String, UsdbError>;
}

/// A username and password, held only long enough to log in.
///
/// Not stored here and not written to the settings file: credentials belong in the OS keyring,
/// and a config file that quietly contains somebody's password is how they end up in a backup.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

/// A USDB session: a transport, a rate limit, and the protocol on top.
pub struct Usdb<T: Transport> {
    transport: T,
    limiter: Limiter,
    /// Who is logged in, once a page has said so.
    user: Option<String>,
    /// Advanced on every retry so the jitter differs between them.
    noise: u64,
}

/// What a session knows about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    /// Nobody is logged in. The catalog is still readable; note files are not.
    Anonymous,
    LoggedIn(String),
}

impl<T: Transport> Usdb<T> {
    pub fn new(transport: T) -> Self {
        Self::with_rate(transport, Rate::default())
    }

    pub fn with_rate(transport: T, rate: Rate) -> Self {
        Self {
            transport,
            limiter: Limiter::new(rate),
            user: None,
            noise: 0x5DEE_CE66,
        }
    }

    pub fn session(&self) -> Session {
        match &self.user {
            Some(name) => Session::LoggedIn(name.clone()),
            None => Session::Anonymous,
        }
    }

    /// Fetch a page, pacing and retrying.
    ///
    /// A login failure is not retried: repeating a request that was refused for a reason that
    /// will not change is what gets an account locked.
    pub fn page(&mut self, endpoint: &Endpoint) -> Result<String, UsdbError> {
        let request = endpoint.request();
        let mut attempt = 0;
        loop {
            let wait = self.limiter.take(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            match self.transport.fetch(&request) {
                Ok(page) => {
                    parse::check_page(&page)?;
                    if let Some(name) = parse::logged_in_as(&page) {
                        self.user = Some(name);
                    }
                    return Ok(page);
                }
                Err(error @ (UsdbError::NotLoggedIn | UsdbError::NotFound)) => return Err(error),
                Err(error @ UsdbError::BadCredentials) => return Err(error),
                Err(error) if attempt >= RETRIES => return Err(error),
                Err(error) => {
                    self.noise = self
                        .noise
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    let wait = backoff(
                        attempt,
                        Duration::from_millis(500),
                        Duration::from_secs(30),
                        self.noise >> 33,
                    );
                    tracing::warn!("USDB request failed ({error}); retrying in {wait:?}");
                    std::thread::sleep(wait);
                    attempt += 1;
                }
            }
        }
    }

    /// Log in with a username and password.
    pub fn log_in(&mut self, credentials: &Credentials) -> Result<Session, UsdbError> {
        let page = self.page(&Endpoint::Login {
            user: credentials.user.clone(),
            password: credentials.password.clone(),
        })?;
        if page.contains(crate::strings::fixed::LOGIN_INVALID) {
            return Err(UsdbError::BadCredentials);
        }
        self.user = parse::logged_in_as(&page);
        Ok(self.session())
    }

    /// Who the transport's cookies already say we are, if anybody.
    pub fn who_am_i(&mut self) -> Result<Session, UsdbError> {
        let page = self.page(&Endpoint::Profile)?;
        self.user = parse::logged_in_as(&page);
        Ok(self.session())
    }

    /// One page of the catalog.
    pub fn catalog_page(
        &mut self,
        start: usize,
        order: Order,
    ) -> Result<Vec<CatalogSong>, UsdbError> {
        let page = self.page(&Endpoint::List { start, order })?;
        Ok(parse::catalog_page(&page))
    }

    /// Every song on USDB, oldest edit first.
    ///
    /// `keep_going` is called with each page as it arrives and stops the crawl when it returns
    /// false — which is how an incremental sync stops at the high-water mark, and how a
    /// cancelled one stops at all. Three hundred requests is a long time to be uninterruptible.
    pub fn catalog(
        &mut self,
        order: Order,
        mut keep_going: impl FnMut(&[CatalogSong]) -> bool,
    ) -> Result<usize, UsdbError> {
        let mut total = 0;
        let mut start = 0;
        while (start as i64) < MAX_SONG_ID {
            let page = self.catalog_page(start, order)?;
            let found = page.len();
            total += found;
            if !keep_going(&page) {
                break;
            }
            // A short page is the last page. USDB gives no total.
            if found < PAGE_SIZE {
                break;
            }
            start += PAGE_SIZE;
        }
        Ok(total)
    }

    pub fn details(&mut self, id: SongId) -> Result<SongDetails, UsdbError> {
        let page = self.page(&Endpoint::Detail(id))?;
        parse::details(&page, id)
    }

    /// The note file. This is the one thing that needs an account.
    pub fn song_txt(&mut self, id: SongId) -> Result<String, UsdbError> {
        let page = self.page(&Endpoint::Txt(id))?;
        parse::song_txt(&page)
    }

    /// Where a song's cover lives. Served straight off USDB rather than through `index.php`.
    pub fn cover_url(id: SongId) -> String {
        format!("{BASE_URL}data/cover/{id}.jpg")
    }
}

/// The real transport, over HTTP.
pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Http {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            // A cookie jar is the whole of USDB's session handling: logging in sets one and
            // every later request carries it.
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Transport for Http {
    fn fetch(&self, request: &Request) -> Result<String, UsdbError> {
        let url = format!("{BASE_URL}index.php");
        let query: Vec<(&str, &str)> = request
            .params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let mut response = if request.post {
            let form: Vec<(&str, &str)> = request
                .body
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            self.agent
                .post(&url)
                .query_pairs(query)
                .send_form(form)
                .map_err(|e| UsdbError::Transport(e.to_string()))?
        } else {
            self.agent
                .get(&url)
                .query_pairs(query)
                .call()
                .map_err(|e| UsdbError::Transport(e.to_string()))?
        };
        response
            .body_mut()
            .read_to_string()
            .map_err(|e| UsdbError::Transport(e.to_string()))
    }
}
