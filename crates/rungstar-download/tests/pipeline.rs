//! The download pipeline, driven with no network and no yt-dlp.
//!
//! Everything that decides what happens — what to fetch, what to skip, what to do when a
//! resource is missing, when the song becomes singable — is exercised here against fake
//! fetchers. What is left needing a real account is the two requests that get the note file.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rungstar_download::meta::hash;
use rungstar_download::pipeline::{download, Fetcher, Progress, RunToEnd, Stop};
use rungstar_download::plan::{plan, safe_name, watchable, Source};
use rungstar_download::ytdlp::{
    arguments, arguments_with_runtime, is_permanent, written, ExtractError, Extraction, Extractor,
};
use rungstar_download::{Kind, Outcome, Resource, SyncMeta};
use rungstar_usdb::{SongDetails, SongId};

const SONG: &str =
    "#TITLE:Waterloo\n#ARTIST:Abba\n#MP3:audio.ogg\n#BPM:300\n#GAP:0\n: 0 4 60 Wa~\n- 8\nE\n";

fn parse(text: &str) -> rungstar_song::SongTxt {
    rungstar_song::SongTxt::parse_bytes(text.as_bytes())
        .expect("the fixture song should parse")
        .song
}

/// A song whose `#VIDEO` header is a meta-tag list.
fn tagged(tags: &str) -> rungstar_song::SongTxt {
    parse(&SONG.replace("#BPM:300", &format!("#VIDEO:{tags}\n#BPM:300")))
}

// ---------------------------------------------------------------- fake network

struct Canned {
    files: Vec<(String, Vec<u8>)>,
    asked: RefCell<Vec<String>>,
}

impl Canned {
    fn new(files: Vec<(&str, &[u8])>) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|(url, bytes)| (url.to_owned(), bytes.to_vec()))
                .collect(),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl Fetcher for Canned {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        self.asked.borrow_mut().push(url.to_owned());
        self.files
            .iter()
            .find(|(known, _)| known == url)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| format!("404 {url}"))
    }
}

/// An extractor that writes a file of the right shape, or refuses.
struct FakeYtDlp {
    fail: Option<ExtractError>,
    calls: RefCell<Vec<(String, bool, String)>>,
}

impl FakeYtDlp {
    fn working() -> Self {
        Self {
            fail: None,
            calls: RefCell::new(Vec::new()),
        }
    }
    fn broken(error: ExtractError) -> Self {
        Self {
            fail: Some(error),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl Extractor for FakeYtDlp {
    fn extract(
        &self,
        page: &str,
        audio_only: bool,
        into: &Path,
        stem: &str,
    ) -> Result<Extraction, ExtractError> {
        self.calls
            .borrow_mut()
            .push((page.to_owned(), audio_only, stem.to_owned()));
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        let extension = if audio_only { "opus" } else { "webm" };
        let path = into.join(format!("{stem}.{extension}"));
        std::fs::write(&path, format!("pretend {stem}").as_bytes()).unwrap();
        Ok(Extraction {
            path,
            note: String::new(),
        })
    }
}

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";

// ---------------------------------------------------------------- planning

#[test]
fn a_plan_fetches_the_song_then_the_audio_then_the_rest() {
    // The order is the feature. The reference downloads everything before a song is usable, so
    // a 60 MB video stands between you and a 4 MB song you wanted to sing now.
    let directory = tempfile::tempdir().unwrap();
    let song = tagged("v=dQw4w9WgXcQ,co=coverid");
    let plan = plan(SongId(1), SONG, &song, None, None, directory.path());
    let order: Vec<Kind> = plan.steps.iter().map(|s| s.kind).collect();
    assert_eq!(
        order,
        vec![Kind::Txt, Kind::Audio, Kind::Video, Kind::Cover],
        "the song and the audio must come before the big files"
    );
    // And the two that make it singable are named.
    let essential: Vec<Kind> = plan.essential().map(|s| s.kind).collect();
    assert_eq!(essential, vec![Kind::Txt, Kind::Audio]);
}

#[test]
fn one_link_serves_both_the_audio_and_the_video() {
    let directory = tempfile::tempdir().unwrap();
    let song = tagged("v=dQw4w9WgXcQ");
    let plan = plan(SongId(1), SONG, &song, None, None, directory.path());
    let audio = plan.steps.iter().find(|s| s.kind == Kind::Audio).unwrap();
    let video = plan.steps.iter().find(|s| s.kind == Kind::Video).unwrap();
    assert_eq!(
        audio.source,
        Source::Extract {
            page: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
            audio_only: true
        }
    );
    assert_eq!(
        video.source,
        Source::Extract {
            page: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
            audio_only: false
        }
    );
}

#[test]
fn a_video_posted_in_the_comments_is_used_when_the_tag_has_none() {
    let directory = tempfile::tempdir().unwrap();
    let song = tagged("a=dQw4w9WgXcQ");
    let details = SongDetails {
        comment_videos: vec!["https://www.youtube.com/watch?v=WIAvMiUcCgw".into()],
        ..SongDetails::default()
    };
    let plan = plan(
        SongId(1),
        SONG,
        &song,
        Some(&details),
        None,
        directory.path(),
    );
    let video = plan.steps.iter().find(|s| s.kind == Kind::Video).unwrap();
    assert_eq!(
        video.source,
        Source::Extract {
            page: "https://www.youtube.com/watch?v=WIAvMiUcCgw".into(),
            audio_only: false
        }
    );
}

#[test]
fn a_song_with_no_video_tag_and_no_comment_asks_for_no_video() {
    let directory = tempfile::tempdir().unwrap();
    let plan = plan(SongId(1), SONG, &parse(SONG), None, None, directory.path());
    assert!(!plan.steps.iter().any(|s| s.kind == Kind::Video));
    assert!(!plan.steps.iter().any(|s| s.kind == Kind::Audio));
    assert_eq!(plan.steps.len(), 1, "only the song file itself");
}

#[test]
fn a_file_already_on_disk_and_intact_is_not_fetched_again() {
    // What makes a repair cheap. Matching on the content rather than on a timestamp is what
    // makes it survive a folder that has been through cloud sync.
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path();
    let bytes = b"already here";
    std::fs::write(folder.join("Abba - Waterloo.jpg"), bytes).unwrap();
    let mut held = SyncMeta::new(SongId(1), 0, 0);
    held.put(Resource {
        kind: Kind::Cover,
        file: "Abba - Waterloo.jpg".into(),
        source: "https://assets.fanart.tv/fanart/coverid".into(),
        hash: hash(bytes),
        bytes: bytes.len() as u64,
    });

    let song = tagged("co=coverid");
    let intact = plan(SongId(1), SONG, &song, None, Some(&held), folder);
    assert_eq!(intact.skipped, vec![Kind::Cover]);
    assert!(!intact.steps.iter().any(|s| s.kind == Kind::Cover));

    // Damage it and it comes back.
    std::fs::write(folder.join("Abba - Waterloo.jpg"), b"truncated").unwrap();
    let damaged = plan(SongId(1), SONG, &song, None, Some(&held), folder);
    assert!(damaged.skipped.is_empty());
    assert!(damaged.steps.iter().any(|s| s.kind == Kind::Cover));
}

#[test]
fn a_bare_youtube_id_becomes_a_link() {
    assert_eq!(
        watchable("dQw4w9WgXcQ"),
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
    assert_eq!(
        watchable("v=dQw4w9WgXcQ"),
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
    assert_eq!(
        watchable("https://vimeo.com/12345"),
        "https://vimeo.com/12345"
    );
    assert_eq!(watchable("vimeo.com/12345"), "https://vimeo.com/12345");
}

#[test]
fn a_folder_name_is_safe_on_every_file_system() {
    assert_eq!(safe_name("AC/DC - T.N.T."), "AC_DC - T.N.T");
    assert_eq!(safe_name("What?"), "What_");
    assert_eq!(safe_name("Say: Yes"), "Say_ Yes");
    // Windows refuses a trailing dot or space, silently, by truncating.
    assert_eq!(safe_name("Trailing. "), "Trailing");
    // And the device names, which cannot be a folder at all.
    assert_eq!(safe_name("AUX"), "_AUX");
    assert_eq!(safe_name("con.txt"), "_con.txt");
    assert_eq!(safe_name("Auxiliary"), "Auxiliary", "only the exact names");
    assert_eq!(safe_name(""), "song");
    assert!(safe_name(&"x".repeat(400)).chars().count() <= 120);
}

// ---------------------------------------------------------------- running

fn run(
    song: &rungstar_song::SongTxt,
    details: Option<&SongDetails>,
    fetcher: &Canned,
    extractor: &FakeYtDlp,
    stop: &dyn Stop,
) -> (
    tempfile::TempDir,
    Result<rungstar_download::Report, rungstar_download::DownloadError>,
    Vec<Progress>,
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("songs");
    let scratch = directory.path().join("scratch");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();
    let plan = plan(SongId(7), SONG, song, details, None, &root.join("x"));
    let mut progress = Vec::new();
    let report = download(
        &plan,
        &root,
        &scratch,
        1000,
        2000,
        fetcher,
        extractor,
        stop,
        |step| progress.push(step),
    );
    (directory, report, progress)
}

#[test]
fn a_finished_download_lands_in_the_library_with_a_sidecar() {
    let fetcher = Canned::new(vec![("https://assets.fanart.tv/fanart/coverid", JPEG)]);
    let extractor = FakeYtDlp::working();
    let song = tagged("v=dQw4w9WgXcQ,co=coverid");
    let (_dir, report, progress) = run(&song, None, &fetcher, &extractor, &RunToEnd);
    let report = report.expect("it should have worked");

    assert_eq!(report.outcome, Outcome::Complete);
    assert!(report.folder.join("Abba - Waterloo.txt").is_file());
    assert!(report.folder.join("Abba - Waterloo.opus").is_file());
    assert!(report.folder.join("Abba - Waterloo [video].webm").is_file());
    assert!(report.folder.join("Abba - Waterloo.jpg").is_file());

    // The sidecar remembers all four, with hashes.
    let meta = SyncMeta::read(&report.folder).expect("a sidecar");
    assert_eq!(meta.usdb_id, SongId(7));
    assert_eq!(meta.usdb_mtime, 1000);
    assert!(meta.playable());
    for kind in [Kind::Txt, Kind::Audio, Kind::Video, Kind::Cover] {
        let resource = meta.get(kind).unwrap_or_else(|| panic!("{kind:?} missing"));
        assert_eq!(resource.hash.len(), 64, "not a blake3 hex digest");
        assert!(resource.bytes > 0);
    }
    // And nothing on disk disagrees with it.
    assert!(meta.broken(&report.folder).is_empty());

    // The song was announced as playable before the video finished.
    let playable_at = progress
        .iter()
        .position(|p| matches!(p, Progress::Playable(_)))
        .expect("never announced");
    let video_at = progress
        .iter()
        .position(|p| *p == Progress::Finished(Kind::Video))
        .expect("no video");
    assert!(
        playable_at < video_at,
        "the song waited for its video: {progress:?}"
    );
}

#[test]
fn a_missing_cover_does_not_stop_the_song_arriving() {
    // Refusing to deliver a singable song because its artwork 404ed is the reference's
    // behaviour and it is wrong.
    let fetcher = Canned::new(vec![]);
    let extractor = FakeYtDlp::working();
    let song = tagged("v=dQw4w9WgXcQ,co=goneid");
    let (_dir, report, _) = run(&song, None, &fetcher, &extractor, &RunToEnd);
    let report = report.expect("the song should still arrive");
    assert_eq!(report.outcome, Outcome::Partial);
    assert!(report.folder.join("Abba - Waterloo.txt").is_file());
    assert_eq!(report.missing.len(), 1);
    assert_eq!(report.missing[0].0, Kind::Cover);
}

#[test]
fn a_song_whose_audio_cannot_be_fetched_leaves_nothing_behind() {
    // The opposite case: half a song in the library is worse than none, because the scanner
    // indexes it and somebody tries to sing it.
    let fetcher = Canned::new(vec![]);
    let extractor = FakeYtDlp::broken(ExtractError::Unavailable("gone".into()));
    let song = tagged("v=dQw4w9WgXcQ");
    let (dir, report, _) = run(&song, None, &fetcher, &extractor, &RunToEnd);
    assert!(report.is_err(), "a song with no audio should not land");
    let root = dir.path().join("songs");
    let left: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert!(
        left.is_empty(),
        "something was left in the library: {left:?}"
    );
    // And the scratch folder is cleaned up too.
    let scratch: Vec<PathBuf> = std::fs::read_dir(dir.path().join("scratch"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert!(scratch.is_empty(), "scratch left behind: {scratch:?}");
}

#[test]
fn cancelling_stops_and_leaves_the_library_alone() {
    // The reference's abort is cooperative, polled every 500 ms, and cannot stop a running
    // subprocess at all.
    struct StopAtOnce;
    impl Stop for StopAtOnce {
        fn stopped(&self) -> bool {
            true
        }
    }
    let fetcher = Canned::new(vec![]);
    let extractor = FakeYtDlp::working();
    let song = tagged("v=dQw4w9WgXcQ");
    let (dir, report, _) = run(&song, None, &fetcher, &extractor, &StopAtOnce);
    let report = report.expect("cancelling is not a failure");
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert!(std::fs::read_dir(dir.path().join("songs"))
        .unwrap()
        .flatten()
        .next()
        .is_none());
}

#[test]
fn a_cancel_part_way_through_still_leaves_no_half_song() {
    let flag = AtomicBool::new(false);
    let fetcher = Canned::new(vec![]);
    let extractor = FakeYtDlp::working();
    let song = tagged("v=dQw4w9WgXcQ");
    // Stop after the first step by flipping the flag from the progress callback.
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("songs");
    let scratch = directory.path().join("scratch");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();
    let plan = plan(SongId(7), SONG, &song, None, None, &root.join("x"));
    let report = download(
        &plan,
        &root,
        &scratch,
        1,
        2,
        &fetcher,
        &extractor,
        &flag,
        |step| {
            if matches!(step, Progress::Finished(Kind::Txt)) {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        },
    )
    .unwrap();
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert!(std::fs::read_dir(&root).unwrap().flatten().next().is_none());
}

#[test]
fn an_image_is_named_for_what_it_is_rather_than_what_the_url_claims() {
    // Half the covers on fanart.tv are served from a path ending in .jpg and are PNGs. A
    // library full of PNGs called .jpg is a decoder's problem later.
    let fetcher = Canned::new(vec![("https://assets.fanart.tv/fanart/coverid", PNG)]);
    let extractor = FakeYtDlp::working();
    let song = tagged("v=dQw4w9WgXcQ,co=coverid");
    let (_dir, report, _) = run(&song, None, &fetcher, &extractor, &RunToEnd);
    let report = report.unwrap();
    assert!(
        report.folder.join("Abba - Waterloo.png").is_file(),
        "the file was named from the URL rather than its contents"
    );
}

#[test]
fn downloading_a_song_twice_fetches_nothing_the_second_time() {
    let fetcher = Canned::new(vec![("https://assets.fanart.tv/fanart/coverid", JPEG)]);
    let extractor = FakeYtDlp::working();
    let song = tagged("v=dQw4w9WgXcQ,co=coverid");
    let (dir, report, _) = run(&song, None, &fetcher, &extractor, &RunToEnd);
    let folder = report.unwrap().folder;

    let held = SyncMeta::read(&folder).unwrap();
    let again = plan(SongId(7), SONG, &song, None, Some(&held), &folder);
    assert!(
        again.steps.is_empty(),
        "it would fetch {:?} again",
        again.steps.iter().map(|s| s.kind).collect::<Vec<_>>()
    );
    assert_eq!(again.skipped.len(), 4);
    drop(dir);
}

// ---------------------------------------------------------------- repair

#[test]
fn repair_finds_songs_whose_files_have_gone_or_changed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();

    // One good song.
    let good = root.join("Good Song");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::write(good.join("song.txt"), b"hello").unwrap();
    let mut meta = SyncMeta::new(SongId(1), 0, 0);
    meta.put(Resource {
        kind: Kind::Txt,
        file: "song.txt".into(),
        source: "usdb".into(),
        hash: hash(b"hello"),
        bytes: 5,
    });
    meta.write(&good).unwrap();

    // One whose video was deleted.
    let broken = root.join("Broken Song");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("song.txt"), b"hello").unwrap();
    let mut meta = SyncMeta::new(SongId(2), 0, 0);
    meta.put(Resource {
        kind: Kind::Txt,
        file: "song.txt".into(),
        source: "usdb".into(),
        hash: hash(b"hello"),
        bytes: 5,
    });
    meta.put(Resource {
        kind: Kind::Video,
        file: "gone.webm".into(),
        source: "https://example.invalid".into(),
        hash: hash(b"whatever"),
        bytes: 8,
    });
    meta.write(&broken).unwrap();

    // And one nobody downloaded, which is left alone: repairing a song with no sidecar means
    // deciding what it should have been.
    let handmade = root.join("Somebody's Own Song");
    std::fs::create_dir_all(&handmade).unwrap();
    std::fs::write(handmade.join("song.txt"), b"mine").unwrap();

    let found = rungstar_download::pipeline::needs_repair(root);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].1, SongId(2));
    assert_eq!(found[0].2, vec![Kind::Video]);
}

#[test]
fn a_truncated_file_is_broken_even_though_it_exists() {
    // The failure a timestamp cannot see: a download interrupted halfway leaves a file that
    // exists, opens, and plays four seconds of a song.
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path();
    std::fs::write(folder.join("audio.ogg"), b"the whole thing").unwrap();
    let mut meta = SyncMeta::new(SongId(1), 0, 0);
    meta.put(Resource {
        kind: Kind::Audio,
        file: "audio.ogg".into(),
        source: "x".into(),
        hash: hash(b"the whole thing"),
        bytes: 15,
    });
    assert!(meta.broken(folder).is_empty());

    std::fs::write(folder.join("audio.ogg"), b"the whole").unwrap();
    assert_eq!(meta.broken(folder), vec![Kind::Audio]);
}

#[test]
fn a_sidecar_from_a_later_build_is_not_guessed_at() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path();
    std::fs::write(
        SyncMeta::path(folder),
        r#"{"version":99,"usdb_id":1,"usdb_mtime":0,"fetched_at":0,"resources":[]}"#,
    )
    .unwrap();
    assert!(
        SyncMeta::read(folder).is_none(),
        "a newer file was read as if this build understood it"
    );
}

#[test]
fn a_song_edited_on_usdb_is_seen_as_stale() {
    let meta = SyncMeta::new(SongId(1), 1000, 0);
    assert!(!meta.stale(1000));
    assert!(!meta.stale(999));
    assert!(meta.stale(1001));
}

// ---------------------------------------------------------------- yt-dlp

#[test]
fn the_extraction_command_asks_for_what_the_game_can_play() {
    let into = Path::new("/songs/x");
    let ffmpeg = Path::new("/opt/ffmpeg");
    let audio = arguments(
        "https://youtu.be/abc",
        true,
        into,
        "Abba - Waterloo",
        Some(ffmpeg),
    );
    assert!(audio.contains(&"--no-playlist".to_owned()), "{audio:?}");
    assert!(audio.contains(&"--ignore-config".to_owned()));
    assert_eq!(
        audio
            .iter()
            .position(|arg| arg == "--sleep-requests")
            .map(|at| audio[at + 1].as_str()),
        Some("0.75")
    );
    assert_eq!(
        audio
            .iter()
            .position(|arg| arg == "--concurrent-fragments")
            .map(|at| audio[at + 1].as_str()),
        Some("1")
    );
    assert!(audio
        .windows(2)
        .any(|args| { args == ["--remote-components".to_owned(), "ejs:github".to_owned()] }));
    assert!(audio.contains(&"-x".to_owned()), "audio only");
    assert!(audio.iter().any(|a| a.contains("bestaudio")));
    // Whatever route it takes, what lands has to be something the game can open. YouTube's
    // default is Opus in WebM, which Symphonia has no decoder for — so a download that
    // "worked" produced a song that would not play.
    assert_eq!(
        audio
            .iter()
            .position(|a| a == "--audio-format")
            .map(|at| audio[at + 1].as_str()),
        Some("m4a")
    );
    assert!(audio.last().is_some_and(|url| url.contains("youtu.be")));
    // Nothing but the media itself belongs in a song folder.
    assert!(audio.contains(&"--no-write-info-json".to_owned()));
    assert!(audio.contains(&"--no-write-thumbnail".to_owned()));

    let video = arguments(
        "https://youtu.be/abc",
        false,
        into,
        "Abba - Waterloo",
        Some(ffmpeg),
    );
    assert!(!video.contains(&"-x".to_owned()));
    assert!(
        video.iter().any(|a| a.contains("height<=1080")),
        "a 4K video is four times the bytes for a picture behind lyrics"
    );
}

#[test]
fn ffmpeg_is_pointed_at_rather_than_hoped_for() {
    // This is the whole of the "ffmpeg is not installed" bug. yt-dlp looks on the PATH and
    // nowhere else, so a copy shipped beside the game has to be named explicitly — otherwise
    // every download fails at the last step with the file sitting in the same folder as the
    // executable that just ran it.
    let args = arguments(
        "https://youtu.be/abc",
        true,
        Path::new("/songs/x"),
        "song",
        Some(Path::new("/games/rungstar/ffmpeg.exe")),
    );
    let at = args
        .iter()
        .position(|a| a == "--ffmpeg-location")
        .expect("no --ffmpeg-location");
    assert_eq!(args[at + 1], "/games/rungstar/ffmpeg.exe");
}

#[test]
fn the_javascript_runtime_is_given_to_ytdlp_explicitly() {
    let runtime = rungstar_download::runtime::JsRuntime {
        name: "deno",
        program: PathBuf::from("/games/rungstar/tools/deno"),
    };
    let args = arguments_with_runtime(
        "https://youtu.be/abc",
        true,
        Path::new("/songs/x"),
        "song",
        Some(Path::new("/games/rungstar/ffmpeg")),
        Some(&runtime),
    );
    let at = args
        .iter()
        .position(|argument| argument == "--js-runtimes")
        .expect("no JavaScript runtime argument");
    assert_eq!(args[at + 1], "deno:/games/rungstar/tools/deno");
}

#[test]
fn without_ffmpeg_it_asks_for_something_it_can_finish() {
    // Both of the good format choices need ffmpeg: `-x` is a post-processor, and anything
    // above 360p on YouTube arrives as separate video and audio that have to be merged. With
    // no ffmpeg, asking for either fails *after* the bytes have been downloaded. So ask for
    // formats that need no post-processing instead — worse video, but a song at the end of it.
    let audio = arguments("https://youtu.be/abc", true, Path::new("/x"), "song", None);
    assert!(!audio.contains(&"--ffmpeg-location".to_owned()));
    assert!(!audio.contains(&"-x".to_owned()), "-x always runs ffmpeg");
    assert!(
        audio.iter().any(|a| a.contains("bestaudio[ext=m4a]")),
        "m4a first, because the alternative is Opus in WebM and Symphonia cannot decode it"
    );

    let video = arguments("https://youtu.be/abc", false, Path::new("/x"), "song", None);
    assert!(
        video.iter().all(|a| !a.contains('+')),
        "a merged format cannot be merged without ffmpeg: {video:?}"
    );
}

#[test]
fn a_dead_link_is_told_apart_from_an_unlucky_one() {
    // Retrying an age-gated video four times with exponential backoff wastes a minute and
    // ends in the same place.
    for dead in [
        "ERROR: Sign in to confirm your age. This video may be inappropriate",
        "ERROR: Video unavailable",
        "ERROR: Private video. Sign in if you've been granted access",
        "The uploader has not made this video available in your country",
    ] {
        assert!(is_permanent(dead), "{dead}");
    }
    for temporary in [
        "ERROR: unable to download video data: HTTP Error 503",
        "WARNING: unable to extract player version; retrying",
        "ERROR: [Errno 11001] getaddrinfo failed",
    ] {
        assert!(!is_permanent(temporary), "{temporary}");
    }
}

#[test]
fn the_written_file_is_found_by_its_stem_and_partials_are_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let into = directory.path();
    std::fs::write(into.join("Song.part"), b"half").unwrap();
    std::fs::write(into.join("Song.ytdl"), b"bookkeeping").unwrap();
    assert_eq!(written(into, "Song"), None, "a partial is not the file");

    std::fs::write(into.join("Song.webm"), b"the whole thing").unwrap();
    assert_eq!(written(into, "Song"), Some(into.join("Song.webm")));
    assert_eq!(written(into, "Other"), None);
}

// ---------------------------------------------------------------- yt-dlp itself

fn runnable_tool_bytes() -> Vec<u8> {
    #[cfg(windows)]
    {
        let windows = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let curl = Path::new(&windows).join("System32").join("curl.exe");
        let bytes = std::fs::read(&curl).unwrap_or_else(|error| {
            panic!("{} is needed as a test fixture: {error}", curl.display())
        });
        assert!(bytes.len() >= 500_000, "the fixture is unexpectedly small");
        bytes
    }
    #[cfg(unix)]
    {
        let mut script = b"#!/bin/sh\nprintf 'test-tool 1.0\\n'\n".to_vec();
        script.resize(500_000, b'\n');
        script
    }
}

#[test]
fn a_copy_on_the_path_is_preferred_over_one_we_fetched() {
    // Somebody who installed it with their package manager is telling us which one to use,
    // and downloading a second is both rude and confusing.
    use rungstar_download::tool;
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path();

    // Nothing anywhere.
    let found = tool::find(data);
    if !tool::on_the_path() {
        assert_eq!(found, None, "it found a yt-dlp that is not there");

        // A fetched one is then used.
        let path = tool::install(data, &runnable_tool_bytes()).unwrap();
        assert!(path.is_file());
        assert_eq!(tool::find(data), Some(path));
    }
}

#[test]
fn a_stale_managed_copy_is_ignored_so_it_can_be_fetched_again() {
    use rungstar_download::tool;
    if tool::on_the_path() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = tool::managed_path(directory.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, vec![0u8; 600_000]).unwrap();
    assert!(path.is_file());
    assert_eq!(tool::find(directory.path()), None);
}

#[test]
fn an_error_page_is_not_installed_as_a_program() {
    // GitHub answers a bad asset name with a few hundred bytes of HTML. Writing that out and
    // marking it executable produces a failure nobody can read.
    use rungstar_download::tool;
    let directory = tempfile::tempdir().unwrap();
    let error = tool::install(directory.path(), b"<html>404</html>").unwrap_err();
    assert!(matches!(
        error,
        rungstar_download::ToolError::NotAProgram(16)
    ));
    assert!(!tool::managed_path(directory.path()).exists());
}

#[test]
fn a_large_but_broken_ytdlp_is_not_kept() {
    use rungstar_download::tool;
    let directory = tempfile::tempdir().unwrap();
    let error = tool::install(directory.path(), &vec![0u8; 600_000]).unwrap_err();
    assert!(matches!(error, rungstar_download::ToolError::NotRunnable));
    assert!(!tool::managed_path(directory.path()).exists());
}

#[test]
fn installing_leaves_nothing_half_written() {
    use rungstar_download::tool;
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path();
    tool::install(data, &runnable_tool_bytes()).unwrap();

    let left: Vec<String> = std::fs::read_dir(data.join("tools"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        left,
        vec![tool::file_name().to_owned()],
        "a part file survived"
    );

    // And installing again replaces it rather than failing.
    tool::install(data, &runnable_tool_bytes()).unwrap();
    assert!(tool::runs(&tool::managed_path(data)));
}

#[test]
fn the_download_url_names_this_platform() {
    use rungstar_download::tool;
    let url = tool::download_url();
    assert!(url.starts_with("https://github.com/yt-dlp/yt-dlp/releases/latest/"));
    if cfg!(windows) {
        assert!(url.ends_with("yt-dlp.exe"), "{url}");
    } else if cfg!(target_os = "macos") {
        assert!(url.ends_with("yt-dlp_macos"), "{url}");
    } else {
        // The standalone build, not the zipapp: a Deck in Game Mode may have no usable Python.
        assert!(url.ends_with("yt-dlp_linux"), "{url}");
    }
}

#[test]
fn deno_has_an_official_download_for_this_platform() {
    let url = rungstar_download::runtime::download_url().expect("a supported Deno platform");
    assert!(url.starts_with("https://github.com/denoland/deno/releases/latest/download/deno-"));
    assert!(url.ends_with(".zip"));
}

#[test]
fn a_deno_archive_is_installed_atomically_and_runs() {
    use std::io::{Cursor, Write};

    let mut bytes = runnable_tool_bytes();
    bytes.resize(1_000_001, 0);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        rungstar_download::runtime::file_name(),
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(&bytes).unwrap();
    let archive = zip.finish().unwrap().into_inner();

    let directory = tempfile::tempdir().unwrap();
    let runtime = rungstar_download::runtime::install(directory.path(), &archive).unwrap();
    assert_eq!(runtime.name, "deno");
    assert!(runtime.program.is_file());
    assert!(rungstar_download::runtime::runs(&runtime.program));
    assert!(!runtime.program.with_extension("part").exists());
}

#[test]
fn a_copy_beside_the_executable_is_found() {
    // A packaged release ships one so downloading works out of the box. The test runs from
    // the test binary's folder, which is where a bundled copy would sit next to the game.
    use rungstar_download::tool;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(folder) = exe.parent() else {
        return;
    };
    let bundled = folder.join(tool::file_name());
    if bundled.exists() {
        return; // something real is there; do not tread on it
    }
    std::fs::write(&bundled, runnable_tool_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(tool::beside_the_executable(), Some(bundled.clone()));
    let _ = std::fs::remove_file(&bundled);
    assert_eq!(tool::beside_the_executable(), None);
}

#[test]
fn a_fetched_copy_is_preferred_over_a_bundled_one() {
    // The bundled copy is as old as the release, and a yt-dlp a few months old is a yt-dlp
    // that has stopped working. Once a newer one has been fetched, that is the one to run.
    use rungstar_download::tool;
    if tool::on_the_path() {
        return; // the PATH wins over both, which its own test covers
    }
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path();
    let fetched = tool::install(data, &runnable_tool_bytes()).unwrap();
    assert_eq!(tool::find(data), Some(fetched));
}

#[test]
fn every_delivery_ships_yt_dlp() {
    // Three packaging paths, and one of them silently did not: the AppImage script got a
    // bundled copy and the Flatpak manifest did not, which is the delivery that matters on a
    // Steam Deck. A repository-level invariant is the only place that catches that, because
    // nothing about the code changes when a packaging file drops a line.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |relative: &str| {
        let path = root.join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };

    assert!(
        read("packaging/windows/portable.ps1").contains("yt-dlp.exe"),
        "the Windows zip does not bundle yt-dlp"
    );
    assert!(
        read("packaging/windows/portable.ps1").contains("deno.exe"),
        "the Windows zip does not bundle Deno"
    );
    assert!(
        read("packaging/linux/appimage.sh").contains("yt-dlp_linux"),
        "the AppImage does not bundle yt-dlp"
    );
    assert!(
        read("packaging/linux/appimage.sh").contains("deno-x86_64-unknown-linux-gnu.zip"),
        "the AppImage does not bundle Deno"
    );

    // The Flatpak needs both halves: the generated source and the install command. Either one
    // alone fails the build rather than shipping without it, but the manifest is where a
    // half-finished edit would sit unnoticed.
    let manifest = read("packaging/linux/de.rungstar.RungStar.yml");
    assert!(
        manifest.contains("ytdlp-source.json"),
        "the Flatpak declares no yt-dlp source"
    );
    assert!(
        manifest.contains("install -Dm755 yt-dlp /app/bin/yt-dlp"),
        "the Flatpak fetches yt-dlp and never installs it"
    );
    assert!(
        manifest.contains("deno-source.json")
            && manifest.contains("install -Dm755 deno-runtime/deno /app/bin/deno"),
        "the Flatpak does not declare and install Deno"
    );
    assert!(
        manifest.contains("org.freedesktop.Platform.ffmpeg-full:")
            && manifest.contains("no-autodownload: false"),
        "the Flatpak names ffmpeg-full but will not install it automatically"
    );
    assert!(
        read("packaging/linux/fetch-ytdlp.sh").contains("sha256"),
        "the Flatpak source is not pinned by checksum, which an offline build needs"
    );
    assert!(
        read("packaging/linux/fetch-deno.sh").contains("sha256"),
        "the Deno Flatpak source is not pinned by checksum"
    );
    let builder = read("packaging/linux/build-flatpak.sh");
    assert!(
        builder.contains("Cargo.lock") && builder.contains("-nt \"$here/cargo-sources.json\""),
        "the ignored Flatpak Cargo sources are not regenerated after Cargo.lock changes"
    );

    // And the file the generator writes is not committed, because it names one release.
    assert!(
        read(".gitignore").contains("ytdlp-source.json"),
        "a pinned yt-dlp release is committed and will go stale"
    );
    assert!(
        read(".gitignore").contains("deno-source.json"),
        "a pinned Deno release is committed and will go stale"
    );
}
