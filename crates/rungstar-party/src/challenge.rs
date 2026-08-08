//! What a challenge changes about one song.
//!
//! UltraStar Deluxe ships these as Lua plugins in `game/plugins/*.usdx`, one file each, every
//! one re-implementing the same "walk the lines, compare the scores" loop against a scripting
//! API. They are data, not programs: every one of them is some combination of *hide something*,
//! *stop early* and *knock somebody out*. So here they are a table of [`Effects`], and the loop
//! that reads them lives once in [`crate::watch`].
//!
//! Dropping the scripting engine is not a loss of features. It is the same fourteen modes with
//! the interpreter taken out, which is why they are testable.

use serde::{Deserialize, Serialize};

/// Whether the backing track plays, and when it does not.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Music {
    /// The song plays throughout.
    On,
    /// The song cuts out at random and you have to keep singing in time.
    ///
    /// The reference picks a mute of 3 to 6 seconds, then leaves at least 5 to 8 seconds of
    /// music before it may cut again — with a coin flip each time, so the gaps are uneven.
    /// The numbers are kept rather than tidied, because they are what makes it playable: a
    /// shorter silence is not disorienting and a longer one loses the beat entirely.
    Cutting {
        least_silence: f64,
        most_silence: f64,
        least_sound: f64,
        most_sound: f64,
    },
}

impl Music {
    pub const DEAF: Music = Music::Cutting {
        least_silence: 3.0,
        most_silence: 6.0,
        least_sound: 5.0,
        most_sound: 8.0,
    };
}

/// How much of the song is sung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Length {
    Whole,
    /// Stop at the first line boundary past halfway. The reference cuts at exactly half the
    /// last beat, which lands mid-word; a line boundary is the same idea done properly.
    Half,
}

/// What ends the song early, beyond running out of notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Finish {
    /// The song does.
    Song,
    /// The first singer to reach this many points, checked at every line.
    AtPoints(i32),
}

/// How somebody is put out of the round.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Knockout {
    /// Out after this many lines that scored nothing at all.
    ///
    /// Counted per singer and only against the others: everybody having a bad verse is a bad
    /// verse, not a round. See [`crate::watch`] for the comparison, which is the part the
    /// reference gets subtly right and is easy to get wrong.
    Silent { lines: usize },
    /// Out when the rating of the line just sung is below a bar that rises as the song goes on.
    ///
    /// The bar climbs from nothing to perfect over the first `full_at` of the song and stays
    /// there. Starting at zero is what makes it playable: the first line cannot put you out,
    /// and by the last chorus only a clean line keeps you in.
    Rising { full_at: f64 },
}

/// Everything a challenge changes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Effects {
    /// Whether the words are shown.
    pub lyrics: bool,
    /// Whether the note staff is shown.
    pub notes: bool,
    pub music: Music,
    pub length: Length,
    pub finish: Finish,
    pub knockout: Option<Knockout>,
}

impl Default for Effects {
    fn default() -> Self {
        Self::PLAIN
    }
}

impl Effects {
    /// An ordinary song: everything shown, nothing cut short.
    pub const PLAIN: Effects = Effects {
        lyrics: true,
        notes: true,
        music: Music::On,
        length: Length::Whole,
        finish: Finish::Song,
        knockout: None,
    };

    /// Whether this is the plain song, so a screen can skip saying so.
    pub fn is_plain(&self) -> bool {
        *self == Self::PLAIN
    }

    const fn blind_notes(self) -> Self {
        Self {
            notes: false,
            ..self
        }
    }
    const fn blind_lyrics(self) -> Self {
        Self {
            lyrics: false,
            ..self
        }
    }
    const fn to(self, points: i32) -> Self {
        Self {
            finish: Finish::AtPoints(points),
            ..self
        }
    }
}

/// One playable challenge, named for the screen that offers it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Challenge {
    /// Stable across releases: it is what a saved party writes down.
    pub id: &'static str,
    pub name: &'static str,
    /// One line saying what it does to you, shown under the cursor.
    pub blurb: &'static str,
    pub effects: Effects,
}

impl Challenge {
    /// Every challenge, in the order the picker shows them: the plain song, then the ones that
    /// take something away, then the ones that end early, then the ones that put you out.
    pub const ALL: &'static [Challenge] = &[
        Challenge {
            id: "normal",
            name: "Normal",
            blurb: "The song as written. Everybody sings to the end.",
            effects: Effects::PLAIN,
        },
        Challenge {
            id: "blind-lyrics",
            name: "Blind lyrics",
            blurb: "No words. The notes are still there, so you have to know how it goes.",
            effects: Effects::PLAIN.blind_lyrics(),
        },
        Challenge {
            id: "blind-notes",
            name: "Blind notes",
            blurb: "No staff. The words are there and finding the tune is your problem.",
            effects: Effects::PLAIN.blind_notes(),
        },
        Challenge {
            id: "blind",
            name: "Blind",
            blurb: "No words and no notes. Nothing on screen but the music.",
            effects: Effects::PLAIN.blind_lyrics().blind_notes(),
        },
        Challenge {
            id: "deaf",
            name: "Deaf",
            blurb: "The music cuts out for a few seconds at a time. Keep going.",
            effects: Effects {
                music: Music::DEAF,
                ..Effects::PLAIN
            },
        },
        Challenge {
            id: "short",
            name: "Short song",
            blurb: "Stops halfway, at the end of a line. Good for a long queue.",
            effects: Effects {
                length: Length::Half,
                ..Effects::PLAIN
            },
        },
        Challenge {
            id: "to-2000",
            name: "First to 2000",
            blurb: "Ends the moment somebody reaches 2000 points.",
            effects: Effects::PLAIN.to(2000),
        },
        Challenge {
            id: "to-5000",
            name: "First to 5000",
            blurb: "Ends the moment somebody reaches 5000 points.",
            effects: Effects::PLAIN.to(5000),
        },
        Challenge {
            id: "to-7000",
            name: "First to 7000",
            blurb: "Ends the moment somebody reaches 7000 points. Most of a song.",
            effects: Effects::PLAIN.to(7000),
        },
        Challenge {
            id: "blind-to-500",
            name: "Blind to 500",
            blurb: "Nothing on screen, first to 500 points. Over in a verse.",
            effects: Effects::PLAIN.blind_lyrics().blind_notes().to(500),
        },
        Challenge {
            id: "blind-to-1000",
            name: "Blind to 1000",
            blurb: "Nothing on screen, first to 1000 points.",
            effects: Effects::PLAIN.blind_lyrics().blind_notes().to(1000),
        },
        Challenge {
            id: "blind-to-5000",
            name: "Blind to 5000",
            blurb: "Nothing on screen, first to 5000 points. This one is hard.",
            effects: Effects::PLAIN.blind_lyrics().blind_notes().to(5000),
        },
        Challenge {
            id: "hardcore",
            name: "Hardcore",
            blurb:
                "Three lines that score nothing and you are out, unless everybody is missing them.",
            effects: Effects {
                knockout: Some(Knockout::Silent { lines: 3 }),
                ..Effects::PLAIN
            },
        },
        Challenge {
            id: "hold-the-line",
            name: "Hold the line",
            blurb: "A rising bar. It starts at nothing and reaches perfect three quarters in.",
            effects: Effects {
                knockout: Some(Knockout::Rising { full_at: 0.75 }),
                ..Effects::PLAIN
            },
        },
        Challenge {
            id: "hold-the-line-blind",
            name: "Hold the line, blind",
            blurb: "The rising bar, with nothing on screen to help you clear it.",
            effects: Effects {
                knockout: Some(Knockout::Rising { full_at: 0.75 }),
                ..Effects::PLAIN.blind_lyrics().blind_notes()
            },
        },
    ];

    /// The plain song, which is what everything defaults to.
    pub fn normal() -> &'static Challenge {
        &Self::ALL[0]
    }

    /// Look one up by the id a saved game wrote down.
    ///
    /// An unknown id is the plain song rather than an error: a party saved by a later build
    /// that had one more mode should still open.
    pub fn by_id(id: &str) -> &'static Challenge {
        Self::ALL
            .iter()
            .find(|challenge| challenge.id == id)
            .unwrap_or_else(|| Self::normal())
    }

    /// Whether this one can put somebody out before the song ends.
    pub fn is_knockout(&self) -> bool {
        self.effects.knockout.is_some()
    }
}
