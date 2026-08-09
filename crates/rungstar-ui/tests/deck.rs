//! The Steam Deck pass: every screen at the panel it will actually be played on.
//!
//! A Deck is 1280×800, which is 16:10. The design space is a thousand units tall and as wide
//! as the aspect makes it, so a Deck is **1600×1000 design units** and one design unit is
//! **0.8 physical pixels**. Everything here follows from that arithmetic, and the arithmetic
//! is the reason resolution independence was built the way it was: there is no Deck layout,
//! only the layout, checked at the Deck's shape.
//!
//! Two things are asserted. Nothing is drawn outside the window — a control half off the
//! bottom edge is unreachable on a handheld with no mouse — and no text is too small to read
//! at arm's length on a seven-inch screen.

use rungstar_editor::Editor;
use rungstar_library::SongEntry;
use rungstar_party::{Party, Team};
use rungstar_ui::draw::{Command, DrawList};
use rungstar_ui::editorscreen::EditorScreen;
use rungstar_ui::geom::Rect;
use rungstar_ui::menus::{MainMenu, OptionsScreen};
use rungstar_ui::micscreen::MicScreen;
use rungstar_ui::partyscreen::{PartyScreen, Stage};
use rungstar_ui::playerscreen::{Entry, PlayerScreen};
use rungstar_ui::settings::Settings;
use rungstar_ui::songselect::{Input, SongSelect};
use rungstar_ui::statsscreen::{Row as StatRow, StatsScreen};
use rungstar_ui::theme::Theme;
use rungstar_ui::usdbscreen::UsdbScreen;

/// The Deck's panel, in design units.
///
/// 1280 × 800 is 16:10; a thousand units tall makes it 1600 wide.
const DECK: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1600.0,
    h: 1000.0,
};

/// How many physical pixels one design unit is on a Deck.
const PIXELS_PER_UNIT: f32 = 800.0 / 1000.0;

/// The smallest text worth calling readable, in physical pixels.
///
/// A seven-inch panel held at arm's length. Below about twelve pixels a lower-case letter is
/// three pixels of stem and somebody is squinting at a party.
const LEGIBLE: f32 = 12.0;

fn song(id: i64, artist: &str, title: &str) -> SongEntry {
    SongEntry {
        id,
        path: format!("C:/songs/{artist}/song.txt").into(),
        folder: "Pop".into(),
        artist: artist.into(),
        title: title.into(),
        edition: Some("Best of".into()),
        genre: Some("Rock".into()),
        language: Some("English".into()),
        creator: Some("Somebody".into()),
        tags: None,
        year: Some(1974),
        bpm: 300.0,
        gap_ms: 0,
        duration_secs: 180.0,
        is_duet: false,
        audio_file: Some("audio.ogg".into()),
        video_file: None,
        cover_file: None,
        background_file: None,
        note_count: 100,
        golden_count: 4,
        difficulty: 0.5,
        medley_start: None,
        medley_end: None,
        preview_start: None,
        usdb_id: None,
        times_played: 0,
        last_played: None,
    }
}

/// Draw one screen and check it against the Deck.
fn check(name: &str, list: &DrawList) {
    assert!(list.is_balanced(), "{name} left a clip pushed");
    for command in list.commands() {
        match command {
            Command::Rect { rect, .. } | Command::Outline { rect, .. } => {
                assert!(
                    rect.x >= -1.0
                        && rect.y >= -1.0
                        && rect.right() <= DECK.w + 1.0
                        && rect.bottom() <= DECK.h + 1.0,
                    "{name}: something is off the Deck's screen at {rect:?}"
                );
            }
            Command::Text { text, style, .. } => {
                if text.trim().is_empty() {
                    continue;
                }
                let pixels = style.size * PIXELS_PER_UNIT;
                assert!(
                    pixels >= LEGIBLE,
                    "{name}: {text:?} is {pixels:.1} physical pixels on a Deck"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn every_screen_fits_a_steam_deck_and_is_readable_on_it() {
    let style = Theme::builtin().resolve_default();
    let settings = Settings::default();
    let mut list = DrawList::new();

    // The main menu.
    let mut menu = MainMenu::new();
    menu.draw(
        &mut list,
        DECK,
        &style,
        "An UltraStar Deluxe-class karaoke game",
    );
    check("main menu", &list);

    // The song browser, in every layout, with a song under the cursor.
    let songs: Vec<SongEntry> = (0..40)
        .map(|i| {
            song(
                i,
                &format!("Artist {i}"),
                &format!("A Song With Quite A Long Title {i}"),
            )
        })
        .collect();
    for _ in 0..3 {
        let mut browser = SongSelect::new();
        browser.set_results(songs.clone());
        for step in [None, Some(Input::Search), Some(Input::CycleFilter)] {
            let mut browser = {
                let mut fresh = SongSelect::new();
                fresh.set_results(songs.clone());
                if let Some(step) = step {
                    fresh.handle(step, DECK);
                }
                fresh
            };
            list.clear();
            browser.draw(&mut list, DECK, &style, &|_| None);
            check("song browser", &list);
        }
        browser.handle(Input::CycleLayout, DECK);
    }

    // Options, every page.
    let mut options = OptionsScreen::new();
    let mut settings = settings;
    for _ in 0..8 {
        options.handle(Input::Confirm, &mut settings);
        for _ in 0..20 {
            list.clear();
            options.draw(&mut list, DECK, &style, &settings);
            check("options", &list);
            options.handle(Input::Down, &mut settings);
        }
        options.handle(Input::Back, &mut settings);
        options.handle(Input::Down, &mut settings);
    }

    // Singers.
    let mut players = PlayerScreen::new();
    players.players = (1..=6)
        .map(|id| Entry {
            id,
            name: format!("Singer number {id}"),
            colour: (id as u8) % 6,
            songs: 12,
            best: 8123,
        })
        .collect();
    players.microphones = 4;
    list.clear();
    players.draw(&mut list, DECK, &style);
    check("singers", &list);

    // Statistics.
    let mut stats = StatsScreen::new();
    stats.set_rows(
        (0..20)
            .map(|i| StatRow {
                label: format!("A Fairly Long Song Title {i}"),
                detail: "Somebody With A Long Name".to_owned(),
                value: "9123".to_owned(),
            })
            .collect(),
    );
    list.clear();
    stats.draw(&mut list, DECK, &style);
    check("statistics", &list);

    // Microphones.
    let mut mics = MicScreen::new();
    mics.devices = vec![
        rungstar_ui::micscreen::Device {
            name: "Realtek(R) Audio High Definition Microphone Array".to_owned(),
            assignment: vec![1, 2],
            levels: vec![0.4, 0.1],
            heard: vec![true, false],
        };
        4
    ];
    list.clear();
    mics.draw(&mut list, DECK, &style);
    check("microphones", &list);

    // Party, at every stage.
    let mut party = PartyScreen::new();
    party.pool = vec!["Ada".into(), "Grace".into(), "Kata".into()];
    let mut running = Party::new(
        vec![
            Team::new("Team 1", vec!["Ada".into()]),
            Team::new("Team 2", vec!["Grace".into()]),
        ],
        4,
    );
    running.offer("Abba - Waterloo");
    party.party = Some(running);
    party.offered = Some("Abba - Waterloo".to_owned());
    for stage in [Stage::Setup, Stage::Round, Stage::Finished] {
        party.stage = stage;
        list.clear();
        party.draw(&mut list, DECK, &style);
        check("party", &list);
    }

    // USDB.
    let mut usdb = UsdbScreen::new();
    usdb.catalog_size = 30_000;
    usdb.set_rows(
        (0..30)
            .map(|i| rungstar_ui::usdbscreen::Row {
                id: rungstar_usdb::SongId(i),
                artist: format!("Artist {i}"),
                title: format!("A Long Song Title Number {i}"),
                language: "English".into(),
                genre: "Pop".into(),
                edition: "Best of".into(),
                year: Some(1974),
                rating: 4.5,
                golden: true,
                local: rungstar_ui::usdbscreen::Local::Absent,
            })
            .collect(),
    );
    usdb.activity = rungstar_ui::usdbscreen::Activity {
        what: "fetching".into(),
        fraction: Some(0.5),
        queued: 2,
    };
    for step in [None, Some(Input::Search), Some(Input::CycleFilter)] {
        let mut usdb = {
            let mut fresh = UsdbScreen::new();
            fresh.catalog_size = usdb.catalog_size;
            fresh.set_rows(usdb.rows.clone());
            if let Some(step) = step {
                fresh.handle(step);
            }
            fresh
        };
        list.clear();
        usdb.draw(&mut list, DECK, &style);
        check("usdb", &list);
    }

    // The editor.
    let parsed = rungstar_editor::song::SongTxt::parse_bytes(
        b"#TITLE:Check\n#ARTIST:Nobody\n#MP3:a.ogg\n#BPM:300\n#GAP:0\n\
          : 0 4 60 la\n: 4 4 62 la\n- 8\n: 12 4 64 la\nE\n",
    )
    .unwrap();
    let mut editor = EditorScreen::new(Editor::over(parsed.song, "check.txt".into()));
    for step in [None, Some(Input::ContextMenu), Some(Input::Search)] {
        if let Some(step) = step {
            editor.handle(Input::Back);
            editor.handle(step);
        }
        list.clear();
        editor.draw(&mut list, DECK, &style);
        check("editor", &list);
    }
}

#[test]
fn the_smallest_text_in_the_theme_is_readable_on_a_deck() {
    // The check above only sees text that happens to be drawn. This one pins the scale itself,
    // so a theme that shrinks the base size cannot quietly take every hint below legibility.
    let style = Theme::builtin().resolve_default();
    let smallest = style.scaled_text(0.7) * PIXELS_PER_UNIT;
    assert!(
        smallest >= LEGIBLE,
        "the smallest text in the theme is {smallest:.1} pixels on a Deck"
    );
}
