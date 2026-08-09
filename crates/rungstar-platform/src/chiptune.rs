//! The menu music, synthesised rather than shipped.
//!
//! The interface sounds are committed WAVs because they are a few kilobytes each. A
//! thirty-second loop is 2.6 MB, which is a large binary nobody reviews, in a repository, for
//! something that is a hundred lines of arithmetic. So this renders it at startup instead —
//! about twenty milliseconds — and the loop can be as long as the music wants to be rather than
//! as long as the file size tolerates.
//!
//! **It is written as a chiptune because the constraints of one are what make it bearable
//! underneath an interface.** Three tone channels and a noise channel is the NES's arrangement,
//! and it forces the music to stay thin: there is no pad, no reverb tail, nothing occupying the
//! middle of the mix where a menu blip or somebody talking has to be heard. A modern-sounding
//! loop with any width to it competes with the game.
//!
//! The channels are the ones a 2A03 has, for the same reason:
//!
//! - **Two pulse channels.** Duty cycle is the only timbre control they have, and it is enough:
//!   an eighth-width pulse is thin and reedy, a half-width one is hollow and fat.
//! - **A triangle** for the bass. Quantised to sixteen steps, which is what gives it the buzz
//!   that a smooth triangle does not have.
//! - **Noise** for the drums, shaped by an envelope into a kick, a snare and a hat.
//!
//! In A minor, matching the pentatonic the interface sounds are built on, so a blip landing on
//! top of the music is in key with it.

/// Output rate. The same as everything else, so nothing is resampled.
const RATE: f32 = 44_100.0;

/// Beats per minute. Brisk enough not to drag under a menu, slow enough not to nag.
const BPM: f32 = 132.0;

/// Semitone offsets from A, so a score can be written in note names.
const A: i32 = 0;
const B: i32 = 2;
const C: i32 = 3;
const D: i32 = 5;
const E: i32 = 7;
const F: i32 = 8;
const G: i32 = 10;

/// A note's frequency, `semitones` above A3 (220 Hz).
fn hz(semitones: i32) -> f32 {
    220.0 * 2f32.powf(semitones as f32 / 12.0)
}

/// Seconds per sixteenth note, which is the grid everything is written on.
fn step_secs() -> f32 {
    60.0 / BPM / 4.0
}

/// One channel's part: `(step, length in steps, semitones above A3)`, and a rest is simply a
/// gap. Written as data so the composition is readable as a list of notes rather than as
/// control flow.
type Part = &'static [(usize, usize, i32)];

/// How long the loop is, in sixteenths. Sixteen bars.
const STEPS: usize = 16 * 16;

/// The chord under each bar, as three notes, repeating every four bars: Am - F - C - G.
///
/// Used by the arpeggio, and by the test that checks the melody does not rest on a note the
/// chord underneath it disagrees with.
const CHORDS: [[i32; 3]; 4] = [
    [A, C + 12, E + 12],
    [F, A, C + 12],
    [C, E, G],
    [G, B, D + 12],
];

/// The bass. Root and fifth of each chord, on the beat.
///
/// The progression is Am - F - C - G, four bars each time round and twice through: the most
/// common four chords in popular music, which is the point. Menu music has to be immediately
/// familiar or it draws attention to itself.
const BASS: Part = &[
    // Am
    (0, 4, A - 12),
    (4, 2, A - 12),
    (8, 4, E - 12),
    (12, 2, A - 12),
    // F
    (16, 4, F - 24),
    (20, 2, F - 24),
    (24, 4, C - 12),
    (28, 2, F - 24),
    // C
    (32, 4, C - 12),
    (36, 2, C - 12),
    (40, 4, G - 24),
    (44, 2, C - 12),
    // G
    (48, 4, G - 24),
    (52, 2, G - 24),
    (56, 4, D - 12),
    (60, 2, G - 24),
    // The same four bars again, so the second half of the loop is not a new idea.
    (64, 4, A - 12),
    (68, 2, A - 12),
    (72, 4, E - 12),
    (76, 2, A - 12),
    (80, 4, F - 24),
    (84, 2, F - 24),
    (88, 4, C - 12),
    (92, 2, F - 24),
    (96, 4, C - 12),
    (100, 2, C - 12),
    (104, 4, G - 24),
    (108, 2, C - 12),
    (112, 4, G - 24),
    (116, 2, G - 24),
    (120, 4, D - 12),
    (124, 2, G - 24),
    // Bars 9-16 repeat the progression under a different melody.
    (128, 4, A - 12),
    (132, 2, A - 12),
    (136, 4, E - 12),
    (140, 2, A - 12),
    (144, 4, F - 24),
    (148, 2, F - 24),
    (152, 4, C - 12),
    (156, 2, F - 24),
    (160, 4, C - 12),
    (164, 2, C - 12),
    (168, 4, G - 24),
    (172, 2, C - 12),
    (176, 4, G - 24),
    (180, 2, G - 24),
    (184, 4, D - 12),
    (188, 2, G - 24),
    (192, 4, A - 12),
    (196, 2, A - 12),
    (200, 4, E - 12),
    (204, 2, A - 12),
    (208, 4, F - 24),
    (212, 2, F - 24),
    (216, 4, C - 12),
    (220, 2, F - 24),
    (224, 4, C - 12),
    (228, 2, C - 12),
    (232, 4, G - 24),
    (236, 2, G - 24),
    // The last bar walks back to the top, so the loop point is a resolution rather than a cut.
    (240, 4, G - 24),
    (244, 2, B - 12),
    (248, 4, C - 12),
    (252, 4, E - 12),
];

/// The melody, on the wide pulse.
///
/// A natural minor, and mostly its pentatonic core — the B and the F are passing notes rather
/// than places the melody rests, which is what keeps it from clashing with an interface blip
/// landing on top of it.
const LEAD: Part = &[
    (0, 6, A),
    (6, 2, C + 12),
    (8, 4, E),
    (12, 4, A),
    (16, 6, C + 12),
    (22, 2, D + 12),
    (24, 6, C + 12),
    (30, 2, A),
    (32, 6, E + 12),
    (38, 2, D + 12),
    (40, 4, C + 12),
    (44, 4, G),
    (48, 8, D + 12),
    (56, 4, B),
    (60, 4, A),
    // Second time, up an octave in places, which is all the variation a loop needs.
    (64, 6, A),
    (70, 2, C + 12),
    (72, 4, E + 12),
    (76, 4, A + 12),
    (80, 6, G + 12),
    (86, 2, E + 12),
    (88, 6, F + 12),
    (94, 2, E + 12),
    (96, 6, E + 12),
    (102, 2, G + 12),
    (104, 4, A + 12),
    (108, 4, G + 12),
    (112, 8, D + 12),
    (120, 4, E + 12),
    (124, 4, A),
    // Bars 9-12 drop the lead almost entirely, which is what stops sixteen bars of melody
    // becoming wallpaper. The arpeggio carries it.
    (128, 8, E + 12),
    (140, 4, C + 12),
    (152, 8, F + 12),
    (168, 8, G + 12),
    // And it comes back for the last four.
    (192, 6, A + 12),
    (198, 2, G + 12),
    (200, 4, E + 12),
    (204, 4, D + 12),
    (208, 6, C + 12),
    (214, 2, D + 12),
    (216, 6, E + 12),
    (222, 2, C + 12),
    (224, 6, G + 12),
    (230, 2, E + 12),
    (232, 8, D + 12),
    (240, 4, C + 12),
    (244, 4, B),
    (248, 8, A),
];

/// Sixteenth-note arpeggios of the chord, on the thin pulse. What makes it sound like a
/// chiptune rather than like a tune played on a beeper.
fn arpeggio() -> Vec<(usize, usize, i32)> {
    let mut notes = Vec::with_capacity(STEPS);
    for step in 0..STEPS {
        let bar = (step / 16) % 4;
        // Bars 10 and 12 drop out, under the section where the melody thins to long notes.
        // Alternating rather than resting throughout: something has to keep the pulse, and
        // four bars of bass and drums alone reads as the music having stopped.
        if (128..192).contains(&step) && (step / 16) % 2 == 1 {
            continue;
        }
        let chord = CHORDS[bar];
        notes.push((step, 1, chord[step % 3] + 12));
    }
    notes
}

/// A drum, as what it is made of rather than as a name.
#[derive(Clone, Copy)]
enum Drum {
    /// Low, pitched, short: a sine sweeping down.
    Kick,
    /// Noise with a mid body.
    Snare,
    /// Brief high noise.
    Hat,
}

/// Which drum lands on which sixteenth.
fn drums() -> Vec<(usize, Drum)> {
    let mut hits = Vec::new();
    for bar in 0..16 {
        let at = bar * 16;
        hits.push((at, Drum::Kick));
        hits.push((at + 4, Drum::Snare));
        hits.push((at + 10, Drum::Kick));
        hits.push((at + 12, Drum::Snare));
        // Eighth-note hats, with the offbeats quieter — handled by the synthesiser.
        for eighth in 0..8 {
            hits.push((at + eighth * 2, Drum::Hat));
        }
        // A fill at the end of every fourth bar, which is what stops the loop being a metronome.
        if bar % 4 == 3 {
            hits.push((at + 13, Drum::Snare));
            hits.push((at + 14, Drum::Snare));
            hits.push((at + 15, Drum::Snare));
        }
    }
    hits
}

/// A pulse wave, which is a square with a duty cycle.
///
/// `duty` is the fraction of each cycle spent high: 0.125 is thin and reedy, 0.5 is hollow.
/// Raw rather than band-limited, because the aliasing is the sound — a clean pulse is a synth
/// pad and this is meant to be a chip.
fn pulse(out: &mut [f32], at: usize, freq: f32, secs: f32, duty: f32, gain: f32) {
    let total = (secs * RATE) as usize;
    // A little shorter than the note, so repeated notes at the same pitch are heard as separate
    // notes rather than as one long one.
    let sounding = (total as f32 * 0.85) as usize;
    let period = RATE / freq.max(1.0);
    for n in 0..sounding.min(out.len().saturating_sub(at)) {
        let phase = (n as f32 % period) / period;
        let square = if phase < duty { 1.0 } else { -1.0 };
        // A short attack and a long decay: a chip channel has a volume envelope and a flat one
        // sounds like a test tone.
        let progress = n as f32 / sounding.max(1) as f32;
        let attack = (n as f32 / (RATE * 0.004)).min(1.0);
        let decay = 1.0 - progress * 0.55;
        out[at + n] += square * gain * attack * decay;
    }
}

/// A triangle quantised to sixteen steps, which is what the NES's does and where its buzz
/// comes from.
fn triangle(out: &mut [f32], at: usize, freq: f32, secs: f32, gain: f32) {
    let total = ((secs * RATE) as usize).min(out.len().saturating_sub(at));
    let period = RATE / freq.max(1.0);
    for n in 0..total {
        let phase = (n as f32 % period) / period;
        let raw = if phase < 0.5 {
            4.0 * phase - 1.0
        } else {
            3.0 - 4.0 * phase
        };
        let stepped = (raw * 8.0).round() / 8.0;
        let attack = (n as f32 / (RATE * 0.003)).min(1.0);
        let release = ((total - n) as f32 / (RATE * 0.01)).min(1.0);
        out[at + n] += stepped * gain * attack * release;
    }
}

/// One drum hit.
fn drum(out: &mut [f32], at: usize, kind: Drum, state: &mut u32) {
    let (secs, gain) = match kind {
        Drum::Kick => (0.11, 0.55),
        Drum::Snare => (0.09, 0.30),
        Drum::Hat => (0.025, 0.11),
    };
    let total = ((secs * RATE) as usize).min(out.len().saturating_sub(at));
    let mut last = 0.0f32;
    for n in 0..total {
        let progress = n as f32 / total.max(1) as f32;
        let envelope = (1.0 - progress) * (1.0 - progress);
        let value = match kind {
            Drum::Kick => {
                // A sine sweeping from 150 Hz down to 45, which is a kick drum.
                let freq = 150.0 * (1.0 - progress) + 45.0 * progress;
                (std::f32::consts::TAU * freq * n as f32 / RATE).sin()
            }
            Drum::Snare | Drum::Hat => {
                // xorshift, so the loop is identical every run and two players hear the same
                // thing.
                *state ^= *state << 13;
                *state ^= *state >> 17;
                *state ^= *state << 5;
                let white = ((*state & 0xFFFF) as f32 / 32768.0) - 1.0;
                if matches!(kind, Drum::Hat) {
                    // High-passed, or a hat is a burst of mud in the same range as the bass.
                    last = 0.6 * last + 0.4 * white;
                    white - last
                } else {
                    last = 0.75 * last + 0.25 * white;
                    (white - last) * 0.6 + last * 0.8
                }
            }
        };
        out[at + n] += value * gain * envelope;
    }
}

/// Lay one part down on the buffer.
fn play(
    out: &mut [f32],
    part: &[(usize, usize, i32)],
    voice: impl Fn(&mut [f32], usize, f32, f32),
) {
    let step = step_secs();
    for (at, length, semitones) in part {
        let start = (*at as f32 * step * RATE) as usize;
        if start >= out.len() {
            continue;
        }
        voice(out, start, hz(*semitones), *length as f32 * step);
    }
}

/// Render the loop.
///
/// Returned as 16-bit mono at [`RATE`], the same as everything else the mixer handles, and
/// **seamless**: the last sample joins the first, so playing it end to end has no click and no
/// gap. Everything that decays is given room to finish inside the loop rather than being cut at
/// the boundary.
pub fn render() -> Vec<i16> {
    let step = step_secs();
    let frames = (STEPS as f32 * step * RATE) as usize;
    // Rendered with an overhang, which is then folded back onto the start. A note or a cymbal
    // that is still ringing when the loop ends is ringing at the start of the next repetition
    // too, so that is where it belongs — fading it out instead leaves a quiet quarter-second
    // before every repeat, which is more obvious than the click it was avoiding.
    let tail = (0.4 * RATE) as usize;
    let mut out = vec![0f32; frames + tail];

    play(&mut out, BASS, |out, at, freq, secs| {
        triangle(out, at, freq, secs, 0.34)
    });
    play(&mut out, LEAD, |out, at, freq, secs| {
        pulse(out, at, freq, secs, 0.25, 0.16)
    });
    let arp = arpeggio();
    play(&mut out, &arp, |out, at, freq, secs| {
        pulse(out, at, freq, secs, 0.125, 0.075)
    });

    let mut noise = 0x1234_5678u32;
    for (at, kind) in drums() {
        let start = (at as f32 * step * RATE) as usize;
        if start < out.len() {
            drum(&mut out, start, kind, &mut noise);
        }
    }

    for n in 0..tail {
        out[n] += out[frames + n];
    }
    out.truncate(frames);

    // Normalised with headroom, because this plays *under* everything else.
    let loudest = out.iter().fold(0.0f32, |held, v| held.max(v.abs()));
    let scale = if loudest > 0.0 { 0.72 / loudest } else { 0.0 };
    out.iter()
        .map(|v| (v * scale).clamp(-1.0, 1.0))
        .map(|v| (v * 32767.0) as i16)
        .collect()
}

/// How long the rendered loop is, in seconds.
pub fn length_secs() -> f32 {
    STEPS as f32 * step_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_is_long_enough_not_to_nag() {
        // Sixteen bars at 132. Anything much shorter is a jingle, and a jingle heard while
        // somebody browses eight thousand songs is the thing they turn off.
        let secs = length_secs();
        assert!((secs - 29.09).abs() < 0.1, "{secs:.2}s");
        assert_eq!(render().len(), (secs * RATE) as usize);
    }

    #[test]
    fn it_joins_up_with_itself() {
        // Played end to end forever, so the seam is heard more often than any other moment in
        // it. What matters is not that both ends are near zero — that would mean a gap — but
        // that the step across the join is no bigger than the steps the music takes anyway.
        // Anything larger is a discontinuity, which is a click once every thirty seconds.
        let music = render();
        let biggest = music
            .windows(2)
            .map(|pair| (i32::from(pair[1]) - i32::from(pair[0])).abs())
            .max()
            .unwrap_or(0);
        let across = (i32::from(music[0]) - i32::from(music[music.len() - 1])).abs();
        assert!(
            across <= biggest,
            "the join steps {across} where the music never steps more than {biggest}"
        );
    }

    #[test]
    fn it_leaves_room_for_everything_it_plays_under() {
        let music = render();
        let peak = music.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(peak > 15_000, "too quiet to be worth playing: {peak}");
        assert!(peak < 25_000, "no headroom for a song over it: {peak}");
    }

    #[test]
    fn it_is_the_same_every_time() {
        // The drums are noise, and noise from a clock would mean two machines hearing
        // different music and a rebuild producing a different file.
        assert_eq!(render(), render());
    }

    #[test]
    fn nothing_is_silent_for_more_than_a_moment() {
        // A part written with the wrong step numbers leaves a hole, which is not obvious from
        // reading a table of note positions.
        let music = render();
        let window = (0.5 * RATE) as usize;
        let mut quiet_run = 0;
        for chunk in music.chunks(window) {
            let peak = chunk.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            quiet_run = if peak < 500 { quiet_run + 1 } else { 0 };
            assert!(
                quiet_run < 2,
                "a second of silence in the middle of the loop"
            );
        }
    }

    #[test]
    fn the_key_is_the_one_the_interface_sounds_are_in() {
        // A blip lands on top of this music constantly. Both are built on A, so it lands in
        // key rather than against it.
        assert!((hz(A) - 220.0).abs() < 0.01);
        assert!((hz(A + 12) - 440.0).abs() < 0.01);
        // Every note in the melody is in A natural minor, and the ones it *rests* on — the
        // long notes — are in the pentatonic core, which has no semitone clashes in it.
        for (at, length, semitones) in LEAD {
            let class = semitones.rem_euclid(12);
            assert!(
                [A, B, C, D, E, F, G].contains(&class),
                "the melody leaves A minor on {semitones}"
            );
            // A note the melody *rests* on has to agree with the chord under it. A passing
            // note can be anything in the scale — that is what makes it a passing note — but
            // a held one that the harmony disagrees with is heard as a wrong note, and this
            // music is under an interface for hours at a time.
            if *length >= 4 {
                let chord = CHORDS[(at / 16) % 4];
                let consonant = chord.iter().any(|note| note.rem_euclid(12) == class)
                    || [A, C, D, E, G].contains(&class);
                assert!(
                    consonant,
                    "the melody rests on {semitones} over {chord:?} at step {at}"
                );
            }
        }
    }
}
