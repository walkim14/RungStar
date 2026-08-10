//! Every screen, drawn once, for the tests that check something about all of them.
//!
//! Written once because the two things worth checking across the whole interface — that it
//! fits a Steam Deck, and that no two lines of text land on top of each other — are the same
//! walk with a different assertion at the end. Keeping two copies of it means the second one
//! silently stops covering a screen the first one gained.

#![allow(dead_code)]

use rungstar_editor::Editor;
use rungstar_library::SongEntry;
use rungstar_party::{Party, Team};
use rungstar_ui::draw::DrawList;
use rungstar_ui::editorscreen::EditorScreen;
use rungstar_ui::geom::Rect;
use rungstar_ui::menus::{MainMenu, OptionsScreen};
use rungstar_ui::micscreen::MicScreen;
use rungstar_ui::partyscreen::{PartyScreen, Stage};
use rungstar_ui::playerscreen::{Entry, PlayerScreen};
use rungstar_ui::settings::Settings;
use rungstar_ui::songselect::{Input, SongSelect};
use rungstar_ui::statsscreen::{Row as StatRow, StatsScreen};
use rungstar_ui::theme::Style;
use rungstar_ui::usdbscreen::UsdbScreen;

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
        loudness: None,
        peak: None,
    }
}

/// Keep a copy of what was just drawn.
fn record(name: &str, list: &DrawList, out: &mut Vec<(String, DrawList)>) {
    out.push((name.to_owned(), list.clone()));
}

/// Draw every screen in the game at `area`, and hand back what each one produced.
pub fn every_screen(area: Rect, style: &Style) -> Vec<(String, DrawList)> {
    let settings = Settings::default();
    let mut list = DrawList::new();
    let mut out: Vec<(String, DrawList)> = Vec::new();

    // The main menu.
    let mut menu = MainMenu::new();
    menu.draw(
        &mut list,
        area,
        style,
        "An UltraStar Deluxe-class karaoke game",
    );
    record("main menu", &list, &mut out);

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
                    fresh.handle(step, area);
                }
                fresh
            };
            list.clear();
            browser.draw(&mut list, area, style, &|_| None);
            record("song browser", &list, &mut out);
        }
        browser.handle(Input::CycleLayout, area);
    }

    // Options, every page.
    let mut options = OptionsScreen::new();
    let mut settings = settings;
    for _ in 0..8 {
        options.handle(Input::Confirm, &mut settings);
        for _ in 0..20 {
            list.clear();
            options.draw(&mut list, area, style, &settings);
            record("options", &list, &mut out);
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
    players.draw(&mut list, area, style);
    record("singers", &list, &mut out);

    // The microphone delay measurement, while it runs and once it has finished.
    {
        use rungstar_ui::calibratescreen::{CalibrateScreen, Doing, Pass, Report};
        let heard = |millis: f32| Pass {
            millis,
            confidence: 0.9,
            heard: true,
            level: 0.4,
        };
        let mut screen = CalibrateScreen::new();
        screen.doing = Some(Doing {
            device: "Realtek(R) Audio High Definition Microphone Array".to_owned(),
            device_index: 2,
            devices: 3,
            pass: 3,
            passes: 5,
            playing: true,
            level: 0.42,
        });
        screen.reports = vec![Report {
            name: "USB Microphone".to_owned(),
            delay: Ok(96.0),
            passes: vec![heard(95.0), heard(96.0), heard(97.0)],
        }];
        list.clear();
        screen.draw(&mut list, area, style);
        record("measuring the delay", &list, &mut out);

        // Finished, with one that worked and two that did not — the case where the screen has
        // the most to say and the most room to get it wrong.
        screen.doing = None;
        screen.applied = Some(96);
        screen.reports.push(Report {
            name: "Headset Microphone (a very long device name indeed)".to_owned(),
            delay: Err("the microphone did not hear the sound".to_owned()),
            passes: vec![Pass {
                millis: 210.0,
                confidence: 0.04,
                heard: false,
                level: 0.03,
            }],
        });
        screen.reports.push(Report {
            name: "Silent Device".to_owned(),
            delay: Err("the microphone recorded silence".to_owned()),
            passes: vec![Pass {
                millis: 0.0,
                confidence: 0.0,
                heard: false,
                level: 0.0,
            }],
        });
        list.clear();
        screen.draw(&mut list, area, style);
        record("delay measured", &list, &mut out);

        // And nothing to measure at all.
        let mut refused = CalibrateScreen::new();
        refused.trouble = Some("no speakers to play through".to_owned());
        list.clear();
        refused.draw(&mut list, area, style);
        record("delay refused", &list, &mut out);
    }

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
    stats.draw(&mut list, area, style);
    record("statistics", &list, &mut out);

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
    mics.draw(&mut list, area, style);
    record("microphones", &list, &mut out);

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
        party.draw(&mut list, area, style);
        record("party", &list, &mut out);
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
        usdb.draw(&mut list, area, style);
        record("usdb", &list, &mut out);
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
        editor.draw(&mut list, area, style);
        record("editor", &list, &mut out);
    }

    out
}
