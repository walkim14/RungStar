#!/usr/bin/env python3
"""Generate the interface sounds.

Run from the repository root:

    python tools/make-sounds.py

The sounds are generated rather than downloaded, and the generator is committed rather than
the reasoning behind it being lost. A WAV is a binary nobody reviews in a diff; this script is
forty lines of arithmetic anybody can read and change, and re-running it is how the set gets
tuned. It also sidesteps the licensing question entirely — there is nothing here to be
licensed from anybody.

Three things shape every sound in the set:

**They play over music.** A karaoke game is never silent, so an interface sound has to cut
through a mix without fighting it. Everything here sits between 400 Hz and 3 kHz — above the
bass, below the air — and nothing lasts longer than a third of a second except the two that
mark the end of something.

**They are in tune with nothing in particular.** The notes come from a pentatonic scale on A,
which has no semitone clashes in it, so a blip landing on top of a song in any key sounds like
a percussion hit rather than a wrong note. Picking a major triad instead would clash with
every song in a minor key, which is most of them.

**A menu blip is heard hundreds of times an hour.** Short, quiet, and with the attack softened
so it is a tap rather than a click: a hard-edged sample repeated that often becomes the thing
you hear instead of the music.
"""

import array
import math
import os
import struct
import wave

RATE = 44100
CHANNELS = 1

# A minor pentatonic on A. Frequencies rather than note names, because the arithmetic below
# multiplies them and a name would only have to be converted back.
A3, C4, D4, E4, G4, A4, C5, D5, E5, G5, A5 = (
    220.00, 261.63, 293.66, 329.63, 392.00, 440.00, 523.25, 587.33, 659.25, 783.99, 880.00,
)


def envelope(n, total, attack, release, sustain=1.0):
    """A linear attack and an exponential release.

    Exponential on the way out because a linear fade still ends on a discontinuity you can
    hear as a faint tick, and these are played hundreds of times an hour.
    """
    a = max(1, int(attack * RATE))
    r = max(1, int(release * RATE))
    if n < a:
        return (n / a) * sustain
    if n > total - r:
        left = (total - n) / r
        return sustain * left * left
    return sustain


def tone(freq, seconds, gain=0.5, attack=0.004, release=None, harmonics=(1.0, 0.25, 0.08),
         bend=1.0):
    """One note, as a few harmonics of a sine.

    A pure sine is thin over a mix and a square is harsh; three harmonics with the upper two
    well down is a soft mallet, which is what an interface wants.
    """
    total = int(seconds * RATE)
    release = seconds * 0.6 if release is None else release
    out = []
    phase = [0.0] * len(harmonics)
    for n in range(total):
        # `bend` slides the pitch over the note, which is what makes a two-note confirm read as
        # one gesture rather than as two separate blips.
        f = freq * (bend ** (n / total))
        value = 0.0
        for index, amount in enumerate(harmonics):
            phase[index] += 2.0 * math.pi * f * (index + 1) / RATE
            value += amount * math.sin(phase[index])
        out.append(value * gain * envelope(n, total, attack, release))
    return out


def noise(seconds, gain=0.2, attack=0.001, release=None, seed=1):
    """Filtered noise, for the sparkle in the golden-note sound."""
    total = int(seconds * RATE)
    release = seconds * 0.8 if release is None else release
    state = seed
    last = 0.0
    out = []
    for n in range(total):
        # xorshift, so the sound is identical every time this script is run and a rebuild does
        # not produce a diff.
        state ^= (state << 13) & 0xFFFFFFFF
        state ^= state >> 17
        state ^= (state << 5) & 0xFFFFFFFF
        white = ((state & 0xFFFF) / 32768.0) - 1.0
        # A one-pole high-pass: the low end of noise is rumble that muddies a mix.
        last = 0.85 * last + 0.15 * white
        out.append((white - last) * gain * envelope(n, total, attack, release))
    return out


def mix(*layers, delays=None):
    """Sum layers, each optionally started later."""
    delays = delays or [0.0] * len(layers)
    length = max(int(d * RATE) + len(layer) for layer, d in zip(layers, delays))
    out = [0.0] * length
    for layer, delay in zip(layers, delays):
        at = int(delay * RATE)
        for i, value in enumerate(layer):
            out[at + i] += value
    return out


def write(name, samples, peak=0.7):
    """Normalise to a headroom-leaving peak and write a 16-bit WAV.

    Normalised so the set is level with itself: a confirm that is twice as loud as a move is
    the sound somebody turns the effects off over.
    """
    loudest = max((abs(v) for v in samples), default=0.0)
    if loudest > 0:
        scale = peak / loudest
        samples = [v * scale for v in samples]
    # A short fade at each end, so the file cannot start or stop on a non-zero sample.
    edge = int(0.002 * RATE)
    for i in range(min(edge, len(samples))):
        samples[i] *= i / edge
        samples[-1 - i] *= i / edge

    data = array.array("h", (int(max(-1.0, min(1.0, v)) * 32767) for v in samples))
    path = os.path.join("assets", "sounds", name)
    with wave.open(path, "wb") as handle:
        handle.setnchannels(CHANNELS)
        handle.setsampwidth(2)
        handle.setframerate(RATE)
        handle.writeframes(data.tobytes())
    print(f"  {name:20} {len(samples) / RATE:5.2f}s  {len(data) * 2:7} bytes")


def main():
    os.makedirs(os.path.join("assets", "sounds"), exist_ok=True)
    print("generating interface sounds")

    # Moving the cursor. Heard more than anything else in the game, so it is the quietest and
    # the shortest thing in the set — a tap, not a beep.
    write("move.wav", tone(A4, 0.045, gain=0.35, attack=0.002, harmonics=(1.0, 0.15)), peak=0.35)

    # Confirm: two notes up, overlapping, so it reads as one gesture.
    write(
        "select.wav",
        mix(
            tone(E5, 0.09, gain=0.5, harmonics=(1.0, 0.2, 0.05)),
            tone(A5, 0.14, gain=0.4, harmonics=(1.0, 0.18)),
            delays=[0.0, 0.045],
        ),
    )

    # Back: the same shape downwards, which is the only thing that has to be true of it.
    write(
        "back.wav",
        mix(
            tone(A4, 0.08, gain=0.45, harmonics=(1.0, 0.2)),
            tone(D4, 0.13, gain=0.4, harmonics=(1.0, 0.15)),
            delays=[0.0, 0.04],
        ),
    )

    # Starting a song: a rising three-note arpeggio, the one moment worth a flourish.
    write(
        "start.wav",
        mix(
            tone(A4, 0.12, gain=0.4),
            tone(C5, 0.12, gain=0.4),
            tone(E5, 0.30, gain=0.45),
            delays=[0.0, 0.055, 0.11],
        ),
    )

    # A golden note, while somebody is singing. High, brief and sparkling, so it registers
    # without stepping on the voice it is congratulating.
    write(
        "golden.wav",
        mix(
            tone(A5, 0.16, gain=0.30, attack=0.002, harmonics=(1.0, 0.4, 0.2), bend=1.06),
            noise(0.13, gain=0.10),
            delays=[0.0, 0.0],
        ),
        peak=0.5,
    )

    # A line sung well. Quieter than golden: it happens every few seconds.
    write(
        "line.wav",
        mix(
            tone(E5, 0.07, gain=0.28, harmonics=(1.0, 0.2)),
            tone(G5, 0.10, gain=0.24, harmonics=(1.0, 0.15)),
            delays=[0.0, 0.035],
        ),
        peak=0.4,
    )

    # The end of a song. The one sound allowed to take its time, and the only chord in the set.
    write(
        "finish.wav",
        mix(
            tone(A3, 0.9, gain=0.30, release=0.6),
            tone(A4, 0.9, gain=0.26, release=0.6),
            tone(C5, 0.8, gain=0.22, release=0.55),
            tone(E5, 0.8, gain=0.22, release=0.55),
            tone(A5, 0.7, gain=0.18, release=0.5),
            delays=[0.0, 0.0, 0.06, 0.12, 0.18],
        ),
    )

    # Something refused: a note that does not resolve. Deliberately not harsh — being told no
    # should be informative, not a telling-off.
    write(
        "no.wav",
        mix(
            tone(D4, 0.10, gain=0.35, harmonics=(1.0, 0.3)),
            tone(C4, 0.14, gain=0.30, harmonics=(1.0, 0.25)),
            delays=[0.0, 0.06],
        ),
        peak=0.45,
    )

    print("done")


if __name__ == "__main__":
    main()
