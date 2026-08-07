//! Automatic repair of song text.
//!
//! Songs on USDB are community-authored and carry a predictable set of defects: timings that
//! do not start at zero, tempos transcribed at half speed, notes that overlap, lyrics typed
//! in caps lock, and quotation marks in whatever style the author's keyboard produced. This
//! module applies the same repairs as the reference downloader, in the same order, because
//! several of them depend on earlier ones having already run.
//!
//! Repairs split in two: those correcting outright errors always run, while the ones that
//! merely impose a house style are opt-in through [`FixOptions`].

use crate::error::Warnings;
use crate::headers::Headers;
use crate::meta_tags::{MedleyTag, MetaTags};
use crate::note::{Line, Note};
use crate::SongTxt;

/// A medley shorter than this is probably a mistake.
pub const MEDLEY_MIN_DURATION_SECONDS: f64 = 20.0;
/// A medley longer than this is probably a mistake.
pub const MEDLEY_MAX_DURATION_SECONDS: f64 = 80.0;

/// Which algorithm to use when recomputing when a line disappears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinebreakStyle {
    /// Leave line breaks alone.
    Disable,
    /// UltraStar Deluxe's editor rule: snap tight, based on the beat gap.
    Usdx,
    /// YASS Reloaded's rule: based on the gap in seconds, so it adapts to tempo.
    #[default]
    Yass,
}

/// Where the space between two syllables should live.
///
/// Syllables are concatenated to form the displayed line, so the spaces have to be
/// consistent or words run together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpaceStyle {
    Disable,
    /// Trailing: `"Hel"`, `"lo "`.
    #[default]
    After,
    /// Leading: `" Hel"`, `"lo"`.
    Before,
}

/// Which optional repairs to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixOptions {
    pub linebreaks: LinebreakStyle,
    pub first_word_capitalization: bool,
    pub spaces: SpaceStyle,
    pub quotation_marks: bool,
}

impl Default for FixOptions {
    fn default() -> Self {
        Self {
            linebreaks: LinebreakStyle::Yass,
            first_word_capitalization: true,
            spaces: SpaceStyle::After,
            quotation_marks: true,
        }
    }
}

impl FixOptions {
    /// The settings the reference downloader's own test suite pins.
    pub fn usdx_style() -> Self {
        Self {
            linebreaks: LinebreakStyle::Usdx,
            ..Self::default()
        }
    }

    /// Apply nothing optional.
    pub fn none() -> Self {
        Self {
            linebreaks: LinebreakStyle::Disable,
            first_word_capitalization: false,
            spaces: SpaceStyle::Disable,
            quotation_marks: false,
        }
    }
}

/// Round to the nearest 10 ms, ties to even.
pub(crate) fn round_to_ten(value: f64) -> i64 {
    let tenths = value / 10.0;
    let floor = tenths.floor();
    let diff = tenths - floor;
    let round_up = diff > 0.5 || (diff == 0.5 && (floor as i64) % 2 != 0);
    (if round_up { floor + 1.0 } else { floor }) as i64 * 10
}

impl SongTxt {
    /// Apply every automatic repair.
    ///
    /// Order matters. Relative timings become absolute before anything reads a beat; the duet
    /// split happens before headers are restored so `#P1`/`#P2` are known to be needed; and
    /// the tempo is raised only after the first note has moved to beat zero, so the `#GAP`
    /// adjustment is computed at the original scale.
    pub fn fix(&mut self, options: &FixOptions, warnings: &mut Warnings) {
        self.fix_relative_timings(warnings);
        self.tracks.split_duet_notes();
        self.restore_missing_headers(warnings);
        self.fix_first_timestamp();
        self.fix_low_bpm();
        self.tracks.fix_overlapping_and_touching_notes();
        self.tracks.fix_zero_length_notes();
        self.tracks.fix_pitch_values();
        self.tracks.fix_apostrophes();
        self.headers.fix_apostrophes();
        self.tracks.fix_all_caps();
        self.headers.fix_language();
        self.headers.fix_videogap(&self.meta_tags, warnings);

        match options.linebreaks {
            LinebreakStyle::Disable => {}
            LinebreakStyle::Usdx => self.tracks.fix_linebreaks_usdx(),
            LinebreakStyle::Yass => self.tracks.fix_linebreaks_yass(self.bpm()),
        }
        if options.first_word_capitalization {
            self.tracks.fix_first_word_capitalization();
        }
        if options.spaces != SpaceStyle::Disable {
            self.tracks.fix_spaces(options.spaces);
        }
        if options.quotation_marks {
            let language = self.headers.main_language().to_owned();
            self.tracks.fix_quotation_marks(&language);
        }
    }

    /// Convert `#RELATIVE` timings to absolute beats and drop the header.
    ///
    /// In relative mode every line restarts near zero and the line break carries the offset
    /// to the next one, which nothing downstream understands.
    fn fix_relative_timings(&mut self, warnings: &mut Warnings) {
        if self.headers.relative.as_deref().is_none_or(str::is_empty) {
            return;
        }
        let mut offset = 0;
        for line in self.tracks.all_lines_mut() {
            for note in &mut line.notes {
                note.start += offset;
            }
            let Some(line_break) = line.line_break.as_mut() else {
                // Relative mode predates duets, so this must be the final line.
                break;
            };
            line_break.previous_line_out_time += offset;
            if let Some(t) = line_break.next_line_in_time.as_mut() {
                *t += offset;
            }
            offset = match line_break.next_line_in_time {
                Some(t) if t != 0 => t,
                _ => line_break.previous_line_out_time,
            };
        }
        self.headers.relative = None;
        warnings.warn("converted relative to absolute timings");
    }
}

impl SongTxt {
    /// Rebuild headers that can be derived from the meta tags or the notes.
    fn restore_missing_headers(&mut self, warnings: &mut Warnings) {
        if self.tracks.is_duet() {
            self.headers.p1 = Some(
                self.meta_tags
                    .player1
                    .clone()
                    .unwrap_or_else(|| "P1".to_owned()),
            );
            self.headers.p2 = Some(
                self.meta_tags
                    .player2
                    .clone()
                    .unwrap_or_else(|| "P2".to_owned()),
            );
        }
        if let Some(preview) = self.meta_tags.preview.filter(|p| *p != 0.0) {
            self.headers.previewstart = Some(preview);
        }
        if let Some(medley) = self.tracks.snap_medley(self.meta_tags.medley, warnings) {
            self.meta_tags.medley = Some(medley);
            self.headers.medleystartbeat = Some(medley.start);
            self.headers.medleyendbeat = Some(medley.end);
            let duration = self
                .bpm()
                .beats_to_secs(f64::from(medley.end - medley.start));
            if duration < MEDLEY_MIN_DURATION_SECONDS {
                warnings.warn(format!("medley is unusually short: {duration:.0} seconds"));
            } else if duration > MEDLEY_MAX_DURATION_SECONDS {
                warnings.warn(format!("medley is unusually long: {duration:.0} seconds"));
            }
        }
        if let Some(tags) = self.meta_tags.tags.clone() {
            self.headers.tags = Some(tags);
        }
    }

    /// Shift the song so the first note lands on beat zero, folding the difference into
    /// `#GAP`.
    ///
    /// Beat zero is what every other tool assumes, and `#GAP` is the honest place to record
    /// the lead-in.
    fn fix_first_timestamp(&mut self) {
        let offset = self.tracks.start();
        if offset == 0 {
            self.headers.gap = round_to_ten(self.headers.gap as f64);
            return;
        }
        let offset_ms = self.bpm().beats_to_ms(f64::from(offset));
        for line in self.tracks.all_lines_mut() {
            line.shift(-offset);
        }
        self.headers.shift_medley(|beats| beats - offset);
        self.headers.gap = round_to_ten(self.headers.gap as f64 + offset_ms);
    }

    /// Double the tempo until it is plausible, scaling every timestamp to match.
    ///
    /// Transcribing at half tempo is a very common authoring mistake. The song plays
    /// correctly either way, but a sane BPM makes the beat grid usable in an editor.
    fn fix_low_bpm(&mut self) {
        let Some(mut bpm) = self.headers.bpm else {
            return;
        };
        if !bpm.is_too_low() {
            return;
        }
        let factor = bpm.make_large_enough();
        if factor == 1 {
            return;
        }
        self.headers.bpm = Some(bpm);
        self.headers.shift_medley(|beats| beats * factor);
        for line in self.tracks.all_lines_mut() {
            line.multiply(factor);
        }
    }
}

impl crate::note::Tracks {
    /// Detect a second singer's part hiding inside a single track and split it out.
    ///
    /// Some duets are uploaded without the `P2` marker. The giveaway is a line break whose
    /// beat runs *backwards* — time cannot go back, so the file must have restarted for a
    /// second voice at that point.
    pub fn split_duet_notes(&mut self) {
        if self.track_2.is_some() {
            return;
        }
        let Some(first_break) = self.track_1.first().and_then(|l| l.line_break) else {
            return;
        };
        let mut last_out = first_break.previous_line_out_time;
        for idx in 0..self.track_1.len() {
            let Some(line_break) = self.track_1[idx].line_break else {
                return;
            };
            let cutoff = line_break.previous_line_out_time;
            if cutoff < last_out {
                if let Some(mid) = duet_split_index(&self.track_1[idx], cutoff) {
                    let mut rest = self.track_1.split_off(idx);
                    let mut head = rest.remove(0);
                    let tail_notes = head.notes.split_off(mid);
                    let tail = Line {
                        notes: tail_notes,
                        line_break: head.line_break.take(),
                    };
                    self.track_1.push(head);
                    let mut second = vec![tail];
                    second.append(&mut rest);
                    self.track_2 = Some(second);
                    return;
                }
            }
            last_out = cutoff;
        }
    }

    /// Move a medley's bounds onto real line boundaries.
    ///
    /// A medley that starts mid-line would cut a word in half, so the start snaps to the
    /// beginning of the line it falls in and the end to the end of its line.
    pub fn snap_medley(
        &self,
        medley: Option<MedleyTag>,
        warnings: &mut Warnings,
    ) -> Option<MedleyTag> {
        let medley = medley?;
        // A medley of a duet has no single timeline to cut, so it is dropped.
        if self.track_2.is_some() {
            return None;
        }
        let start = self
            .track_1
            .iter()
            .find(|l| medley.start <= l.end())
            .map(Line::start);
        let end = self
            .track_1
            .iter()
            .rev()
            .find(|l| medley.end >= l.start())
            .map(Line::end);
        let (Some(start), Some(end)) = (start, end) else {
            warnings.warn(format!(
                "medley beats ({}, {}) do not align with any line; ignoring medley",
                medley.start, medley.end
            ));
            return None;
        };
        if start != medley.start {
            warnings.warn(format!("medley start {} snapped to {start}", medley.start));
        }
        if end != medley.end {
            warnings.warn(format!("medley end {} snapped to {end}", medley.end));
        }
        Some(MedleyTag { start, end })
    }

    /// Guarantee notes ascend and are separated by at least one beat.
    ///
    /// Overlapping notes make the scoring engine credit the same beat to two syllables, and
    /// out-of-order ones break every downstream assumption about time moving forwards.
    ///
    /// Ordering is repaired by redistributing the timings, not by moving the syllables: the
    /// lyrics are the one thing an out-of-order file gets right, since they are what the
    /// author typed in reading order. The k-th earliest timing therefore goes to the k-th
    /// syllable. The reference implementation does this with a single pass of adjacent
    /// swaps, which leaves anything worse than one inversion still out of order.
    pub fn fix_overlapping_and_touching_notes(&mut self) {
        for track in self.all_tracks_mut() {
            with_flat_notes(track, |notes| {
                // Stable, so syllables sharing a start beat keep their written order.
                let mut timings: Vec<(i32, i32)> =
                    notes.iter().map(|n| (n.start, n.duration)).collect();
                timings.sort_by_key(|(start, _)| *start);
                for (note, (start, duration)) in notes.iter_mut().zip(timings) {
                    note.start = start;
                    note.duration = duration;
                }

                // Now open up a gap wherever two notes touch or overlap. Notes only ever
                // shrink or move later, so the ordering established above survives.
                for i in 0..notes.len().saturating_sub(1) {
                    let (left, right) = notes.split_at_mut(i + 1);
                    let (current, next) = (&mut left[i], &mut right[0]);
                    let gap = current.gap(next);
                    if gap <= 0 {
                        current.shorten(1 - gap);
                    }
                    let gap = current.gap(next);
                    if gap <= 0 {
                        // The note could not shrink far enough, so the next one gives way.
                        next.shift_start(1 - gap);
                    }
                }
            });
        }
    }

    /// Give a zero-length note a beat, when there is room for it.
    pub fn fix_zero_length_notes(&mut self) {
        for track in self.all_tracks_mut() {
            with_flat_notes(track, |notes| {
                for i in 0..notes.len().saturating_sub(1) {
                    if notes[i].duration == 0 && notes[i + 1].start >= notes[i].end() + 2 {
                        notes[i].duration = 1;
                    }
                }
            });
        }
    }

    /// Re-centre pitches that were authored octaves away from the usual baseline.
    ///
    /// Only whole octaves are shifted, and only when the song is at least two octaves off,
    /// so a genuinely low-voiced transcription is left alone.
    pub fn fix_pitch_values(&mut self) {
        let Some(min_pitch) = self.all_notes().map(|n| n.pitch).min() else {
            return;
        };
        let octave_shift = min_pitch.div_euclid(12);
        if octave_shift.abs() < 2 {
            return;
        }
        for note in self.all_notes_mut() {
            note.pitch -= octave_shift * 12;
        }
    }
}

/// Index at which a line should be cut into two singers' parts, if any.
fn duet_split_index(line: &Line, cutoff: i32) -> Option<usize> {
    let mid = line
        .notes
        .iter()
        .position(|n| n.start < cutoff)
        .unwrap_or(0);
    (mid != 0).then_some(mid)
}

/// Run `f` over a track's notes as one flat sequence, then put them back.
///
/// Several repairs compare each note with the next one, and that pairing has to cross line
/// boundaries — a note at the end of a line can still overlap the first note of the next.
fn with_flat_notes(track: &mut [Line], f: impl FnOnce(&mut [Note])) {
    let lengths: Vec<usize> = track.iter().map(|l| l.notes.len()).collect();
    let mut flat: Vec<Note> = Vec::with_capacity(lengths.iter().sum());
    for line in track.iter_mut() {
        flat.append(&mut line.notes);
    }
    f(&mut flat);
    let mut it = flat.into_iter();
    for (line, len) in track.iter_mut().zip(lengths) {
        line.notes = it.by_ref().take(len).collect();
    }
}

impl crate::note::Tracks {
    /// Normalise the several characters people type instead of a typographic apostrophe.
    pub fn fix_apostrophes(&mut self) {
        for note in self.all_notes_mut() {
            note.text = replace_false_apostrophes(&note.text);
        }
    }

    /// Undo lyrics typed entirely in capitals.
    pub fn fix_all_caps(&mut self) {
        if !self.is_all_caps() {
            return;
        }
        for note in self.all_notes_mut() {
            note.text = note.text.to_lowercase();
        }
        self.fix_first_word_capitalization();
    }

    /// Capitalise the first letter of every line.
    pub fn fix_first_word_capitalization(&mut self) {
        for line in self.all_lines_mut() {
            let Some(note) = line.notes.first_mut() else {
                continue;
            };
            note.text = capitalize_first_letter(&note.text);
        }
    }

    /// Put inter-syllable spaces consistently before or after each syllable.
    pub fn fix_spaces(&mut self, style: SpaceStyle) {
        for line in self.all_lines_mut() {
            if line.notes.is_empty() {
                continue;
            }
            match style {
                SpaceStyle::Disable => {}
                SpaceStyle::After => fix_spaces_after(&mut line.notes),
                SpaceStyle::Before => fix_spaces_before(&mut line.notes),
            }
        }
    }

    /// Replace generic quotation marks with the pair the song's language uses.
    ///
    /// Opening and closing marks alternate across the whole song, so the state has to be
    /// carried from note to note. Nested quotes are not supported — nor are they by the
    /// reference implementation.
    pub fn fix_quotation_marks(&mut self, language: &str) {
        let (open, close) = crate::lang::quotation_marks(language);
        let spaced = crate::lang::has_spaced_quotes(language);
        let mut opening = true;
        for note in self.all_notes_mut() {
            note.text = replace_quotation_marks(&note.text, open, close, spaced, &mut opening);
        }
    }
}

/// Normalise the many near-apostrophes to the typographic one.
pub fn replace_false_apostrophes(value: &str) -> String {
    // Two straight single quotes were meant to be one double quote; handle that before the
    // single-character replacements would consume them.
    let value = value.replace("''", "\"");
    // Grave accent, acute accent, prime, left single quote and the straight apostrophe all
    // stand in for the same character on one keyboard layout or another.
    value
        .chars()
        .map(|c| match c {
            '`' | '\u{00b4}' | '\u{2032}' | '\u{2018}' | '\'' => '\u{2019}',
            other => other,
        })
        .collect()
}

/// Upper-case the first letter, leaving any leading punctuation in place.
///
/// Characters whose upper case is more than one letter are left alone. German sharp s is the
/// one that matters: capitalising it yields "SS", which both changes the word and adds an
/// upper-case letter that the all-caps detector would later read as evidence of caps lock.
fn capitalize_first_letter(text: &str) -> String {
    let Some((idx, ch)) = text.char_indices().find(|(_, c)| c.is_alphabetic()) else {
        return text.to_owned();
    };
    if !ch.is_lowercase() {
        return text.to_owned();
    }
    let mut upper = ch.to_uppercase();
    if upper.len() != 1 {
        return text.to_owned();
    }
    let Some(upper) = upper.next() else {
        return text.to_owned();
    };
    let mut out = String::with_capacity(text.len() + 1);
    out.push_str(&text[..idx]);
    out.push(upper);
    out.push_str(&text[idx + ch.len_utf8()..]);
    out
}

/// Shift the space between syllables to the end of the earlier one.
///
/// Syllables are concatenated to render a line, so exactly one of the two neighbours must
/// carry the separator. This is written as a fixed point — running it twice changes nothing —
/// which the reference implementation is not: it unconditionally hands the final syllable a
/// trailing space, so a syllable with no lyric would acquire one and the next pass would
/// migrate it backwards, drifting by a space each time.
fn fix_spaces_after(notes: &mut [Note]) {
    for i in 0..notes.len() {
        let text = std::mem::take(&mut notes[i].text);
        let leading = text.starts_with(' ');
        let trailing = text.trim_start().ends_with(' ');
        let body = text.trim();

        // A leading space belonged to the join with the previous syllable, so it moves back.
        // A syllable with no lyric has no join to express, and is left alone.
        if leading && i > 0 && !body.is_empty() {
            append_space(&mut notes[i - 1].text);
        }
        notes[i].text = match (body.is_empty(), trailing) {
            (true, _) => String::new(),
            (false, true) => format!("{body} "),
            (false, false) => body.to_owned(),
        };
    }
    // The last syllable of a line ends with a space too, so lines concatenate cleanly and
    // the sung-syllable highlight covers the whole final word.
    if let Some(last) = notes.last_mut() {
        append_space(&mut last.text);
    }
}

/// Shift the space between syllables to the start of the later one.
fn fix_spaces_before(notes: &mut [Note]) {
    for i in 0..notes.len() {
        let text = std::mem::take(&mut notes[i].text);
        let trailing = text.ends_with(' ');
        let leading = text.trim_end().starts_with(' ');
        let body = text.trim();

        if trailing && i + 1 < notes.len() && !body.is_empty() {
            prepend_space(&mut notes[i + 1].text);
        }
        notes[i].text = match (body.is_empty(), leading) {
            (true, _) => String::new(),
            (false, true) => format!(" {body}"),
            (false, false) => body.to_owned(),
        };
    }
    if let Some(first) = notes.first_mut() {
        prepend_space(&mut first.text);
    }
}

/// Ensure the text ends with exactly one space, unless it has no content at all.
fn append_space(text: &mut String) {
    let body = text.trim_end().to_owned();
    *text = if body.is_empty() {
        String::new()
    } else {
        format!("{body} ")
    };
}

/// Ensure the text starts with exactly one space, unless it has no content at all.
fn prepend_space(text: &mut String) {
    let body = text.trim_start().to_owned();
    *text = if body.is_empty() {
        String::new()
    } else {
        format!(" {body}")
    };
}

/// Rewrite the quotation marks in one fragment, carrying the open/close state across calls.
fn replace_quotation_marks(
    text: &str,
    open: char,
    close: char,
    spaced: bool,
    opening: &mut bool,
) -> String {
    if !text
        .chars()
        .any(|c| crate::lang::QUOTATION_MARKS_TO_REPLACE.contains(&c))
    {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len() + 2);
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if crate::lang::QUOTATION_MARKS_TO_REPLACE.contains(&ch) {
            if *opening {
                out.push(open);
                if spaced {
                    out.push(crate::lang::NARROW_NO_BREAK_SPACE);
                    // The narrow space replaces the ordinary one the author typed.
                    if chars.get(i + 1).is_some_and(|c| c.is_whitespace()) {
                        i += 1;
                    }
                }
            } else {
                if spaced {
                    if i > 0 && chars[i - 1].is_whitespace() {
                        out.pop();
                    }
                    out.push(crate::lang::NARROW_NO_BREAK_SPACE);
                }
                out.push(close);
            }
            *opening = !*opening;
        } else {
            out.push(ch);
        }
        i += 1;
    }
    out.into_iter().collect()
}

impl crate::note::Tracks {
    /// Recompute line-break beats the way UltraStar Deluxe's editor does.
    pub fn fix_linebreaks_usdx(&mut self) {
        for track in self.all_tracks_mut() {
            fix_linebreaks(track, |last_end, line_start, gap| {
                Some(if gap < 2 {
                    line_start
                } else if gap == 2 {
                    last_end + 1
                } else {
                    last_end + 2
                })
            });
        }
    }

    /// Recompute line-break beats the way YASS Reloaded does.
    ///
    /// Unlike the UltraStar rule this reasons in seconds, so a long instrumental gap clears
    /// the lyrics at the same wall-clock moment regardless of tempo.
    pub fn fix_linebreaks_yass(&mut self, bpm: crate::Bpm) {
        for track in self.all_tracks_mut() {
            fix_linebreaks(track, |last_end, line_start, gap| {
                let gap_secs = bpm.beats_to_secs(f64::from(gap));
                if gap_secs >= 4.0 {
                    Some(last_end + bpm.secs_to_beats_trunc(2.0))
                } else if gap_secs >= 2.0 {
                    Some(last_end + bpm.secs_to_beats_trunc(1.0))
                } else if (0..=1).contains(&gap) {
                    Some(last_end)
                } else if (2..=8).contains(&gap) {
                    Some(line_start - 2)
                } else if (9..=12).contains(&gap) {
                    Some(line_start - 3)
                } else if (13..=16).contains(&gap) {
                    Some(line_start - 4)
                } else if gap > 16 {
                    Some(last_end + 10)
                } else {
                    // A negative gap means the notes overlap; leave it to the geometry fix.
                    None
                }
            });
        }
    }
}

/// Apply a line-break rule across a track.
///
/// The two-value break form is always collapsed to one: the second value is a display hint
/// that the rules below make redundant, and keeping a stale one desynchronises the lyrics.
fn fix_linebreaks(track: &mut [Line], rule: impl Fn(i32, i32, i32) -> Option<i32>) {
    for i in 1..track.len() {
        if track[i - 1].line_break.is_none() || track[i].notes.is_empty() {
            continue;
        }
        let last_end = track[i - 1].end();
        let line_start = track[i].start();
        let new_out = rule(last_end, line_start, line_start - last_end);
        let Some(line_break) = track[i - 1].line_break.as_mut() else {
            continue;
        };
        line_break.next_line_in_time = None;
        if let Some(out) = new_out {
            line_break.previous_line_out_time = out;
        }
    }
}

impl Headers {
    /// Normalise apostrophes in the human-readable header fields.
    pub fn fix_apostrophes(&mut self) {
        for slot in [&mut self.artist, &mut self.title] {
            *slot = replace_false_apostrophes(slot);
        }
        for slot in [
            &mut self.language,
            &mut self.genre,
            &mut self.p1,
            &mut self.p2,
            &mut self.album,
        ] {
            if let Some(value) = slot.as_mut() {
                *value = replace_false_apostrophes(value);
            }
        }
    }

    /// Canonicalise `#LANGUAGE` to comma-separated English names.
    ///
    /// Files name languages in their own language ("Deutsch", "allemand", "alemán"), which
    /// makes filtering by language useless until they are folded together.
    pub fn fix_language(&mut self) {
        let Some(language) = self.language.as_ref() else {
            return;
        };
        let canonical: Vec<String> = language
            .replace([';', '/', '|'], ",")
            .split(',')
            .map(|part| crate::lang::canonical_language(part.trim()))
            .collect();
        // A value made only of separators canonicalises to nothing; drop it rather than keep
        // an empty value, which would not survive a write/read cycle.
        let joined = canonical.join(", ");
        self.language = (!joined.trim().is_empty()).then_some(joined);
    }

    /// Drop a `#VIDEOGAP` that cannot mean anything.
    ///
    /// The offset only makes sense when audio and video come from different sources. When
    /// they share one, applying it actively pushes them out of sync.
    pub fn fix_videogap(&mut self, meta_tags: &MetaTags, warnings: &mut Warnings) {
        if self.videogap.is_none() {
            return;
        }
        let has_audio = meta_tags.audio.is_some();
        let has_video = meta_tags.video.is_some();
        if has_audio && !has_video {
            self.videogap = None;
            warnings.warn("song is audio only; dropping #VIDEOGAP");
        } else if !has_audio && has_video {
            self.videogap = None;
            warnings.warn("audio and video share a source; dropping #VIDEOGAP");
        }
        // Both set and equal means the author deliberately corrected a source that is itself
        // out of sync, so the offset stays.
    }

    /// Apply `f` to whichever medley bounds are set.
    pub fn shift_medley(&mut self, f: impl Fn(i32) -> i32) {
        for slot in [&mut self.medleystartbeat, &mut self.medleyendbeat] {
            if let Some(beat) = slot.as_mut() {
                // Zero is treated as unset, matching the reference implementation.
                if *beat != 0 {
                    *beat = f(*beat);
                }
            }
        }
    }
}
