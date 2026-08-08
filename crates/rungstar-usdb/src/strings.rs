//! The labels USDB writes, in the three languages it serves them in.
//!
//! The site answers in German, English or French depending on the account, and everything on
//! a detail page is found by its label. Parsing by label without knowing the language is how
//! a scraper quietly returns nothing for half its users — so the language is detected from the
//! welcome banner on every response, not configured once.

/// Every label the parser looks for, in one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Labels {
    pub language: Language,
    pub welcome: &'static str,
    pub song_language: &'static str,
    pub year: &'static str,
    pub genre: &'static str,
    pub edition: &'static str,
    pub bpm: &'static str,
    pub gap: &'static str,
    pub golden_notes: &'static str,
    pub songcheck: &'static str,
    pub date: &'static str,
    pub uploaded_by: &'static str,
    pub views: &'static str,
    pub rating: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
}

/// Which of the three USDB serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    English,
    German,
    French,
}

/// Strings that mean the same thing in every language, because USDB does not translate them.
pub mod fixed {
    /// The body of a page asking for a login. USDB returns this with HTTP 200.
    pub const NOT_LOGGED_IN: &str = "You are not logged in. Login to use this function.";
    /// The body of a page for a song id that does not exist. Also HTTP 200.
    pub const NOT_FOUND: &str = "Datensatz nicht gefunden";
    pub const PLEASE_LOGIN: &str = "Welcome, Please login";
    pub const LOGIN_INVALID: &str = "Login or Password invalid, please try again.";
}

pub const ENGLISH: Labels = Labels {
    language: Language::English,
    welcome: "Welcome",
    song_language: "Language",
    year: "Year",
    genre: "Genre",
    edition: "Edition",
    bpm: "BPM",
    gap: "GAP",
    golden_notes: "Golden Notes",
    songcheck: "Songcheck",
    date: "Date",
    uploaded_by: "Uploaded by",
    views: "Views",
    rating: "Rating",
    yes: "Yes",
    no: "No",
};

pub const GERMAN: Labels = Labels {
    language: Language::German,
    welcome: "Willkommen",
    song_language: "Sprache",
    year: "Jahr",
    genre: "Genre",
    edition: "Edition",
    bpm: "BPM",
    gap: "GAP",
    golden_notes: "Goldene Noten",
    songcheck: "Songcheck",
    date: "Datum",
    uploaded_by: "Hochgeladen von",
    views: "Aufrufe",
    rating: "Bewertung",
    yes: "Ja",
    no: "Nein",
};

pub const FRENCH: Labels = Labels {
    language: Language::French,
    welcome: "Bienvenue",
    song_language: "Langue",
    year: "An",
    genre: "Genre",
    edition: "Edition",
    bpm: "BPM",
    gap: "GAP",
    golden_notes: "Notes en or",
    songcheck: "Songcheck",
    date: "Date",
    uploaded_by: "Téléchargé par",
    views: "Affichages",
    rating: "Classement",
    yes: "Oui",
    no: "Non",
};

pub const ALL: [Labels; 3] = [ENGLISH, GERMAN, FRENCH];

impl Labels {
    /// Which language a page is in, from its welcome banner.
    ///
    /// English is the fallback rather than an error: a page whose banner has moved is still
    /// worth reading, and the numeric fields do not depend on the language at all.
    pub fn detect(page: &str) -> Labels {
        // The banner is the only reliable marker — it is on every page, logged in or not.
        // Looked for near the top so a comment containing "Willkommen" cannot decide it.
        let head = &page[..page.len().min(8000)];
        for labels in ALL {
            if head.contains(labels.welcome) {
                return labels;
            }
        }
        // A logged-out page says "Welcome, Please login" in English whatever the account
        // language is, so it lands here too.
        ENGLISH
    }
}
