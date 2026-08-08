# Fonts

Empty on purpose.

A packaged build looks here first, for `RungStar-Regular.ttf` and `RungStar-Bold.ttf`, before
it falls back to borrowing a face from the system. Drop a font in before packaging — see
`packaging/README.md`.

It is not committed because a font binary is a megabyte of something nobody reviews in a diff,
and choosing one is a licensing decision rather than a technical one. Any face whose licence
permits redistribution works. **DejaVu Sans** and **Noto Sans** are the obvious candidates:
both cover the Latin, Greek and Cyrillic that a real song library turns out to contain, and
both are already on most Linux machines.

Without one the game still starts. It borrows Segoe UI on Windows and DejaVu on Linux, and says
so if it can find neither — which is fine for development and wrong for a release, because it
makes the game look different on every machine and a Flatpak has almost nothing to borrow.
