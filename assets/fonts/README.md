# Fonts

Committed, not dropped in at packaging time.

| File | Face | Licence |
|---|---|---|
| `RungStar-Regular.ttf` | Poppins Regular | OFL 1.1 — `OFL-Poppins.txt` |
| `RungStar-Bold.ttf` | Poppins SemiBold | OFL 1.1 — `OFL-Poppins.txt` |
| `RungStar-Lyrics.ttf` | Poppins Bold | OFL 1.1 — `OFL-Poppins.txt` |
| `RungStar-Fallback.ttf` | Fira Sans Regular | OFL 1.1 — `OFL-FiraSans.txt` |

## Why Poppins

A karaoke game is read from across a room, usually at an angle, usually by somebody who has
had a drink. Poppins is geometric with a tall x-height and near-circular bowls, so it stays
legible small and reads as friendly rather than corporate at the size a lyric line is drawn.
The alternative shortlist — Inter, Outfit, Figtree, Baloo 2 — was ruled out by something
duller: **fontdue does not apply variable-font axes**, and all four ship from Google Fonts as
variable-only. Loading one gives you its default instance whatever weight you asked for, so
bold and regular come out identical. Poppins and Fira Sans still ship static weights.

## Why there is a fallback at all

Measured over the 8,134-song library, the text is 99.94% ASCII — and the remainder is 160,908
curly quotes, 27,868 accented letters, a few hundred CJK brackets, 202 Cyrillic characters and
some Hangul. Poppins covers the first two and not the rest.

Without a chain, choosing a font is choosing between character and coverage, and getting it
wrong fails *silently*: an empty box where a letter should be. That already happened once — the
USDB star ratings drew as empty squares for weeks because the borrowed system face had no `★`.

So `Face` carries a fallback chain and Fira Sans rides along behind the visible faces. It is
never drawn for ordinary text; it exists for the 0.06%.

## Changing them

Any face whose licence permits redistribution works. Replace the file, keep the name, add the
licence beside it, and run:

```
cargo test -p rungstar-platform --test fonts
```

That asserts what the faces can actually draw — every character class a real library contains,
the same coverage across all three weights, and that the fallback still covers something the
chosen face does not. It is what makes a binary blob reviewable.

The game still starts with this folder empty: it borrows Segoe UI on Windows and DejaVu on
Linux, and says so if it can find neither. That is fine for development and wrong for a
release, because a Flatpak has almost nothing to borrow.
