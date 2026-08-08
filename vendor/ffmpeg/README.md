# FFmpeg

Prebuilt Windows x64 shared libraries from [BtbN/FFmpeg-Builds][builds], GPL configuration,
vendored the same way and for the same reason as `vendor/sdl3/`: the game should build and run
on a clean machine without a separate install step.

Only what the game links is here — `avcodec`, `avformat`, `avutil`, `swscale`, `swresample`.
The build's `ffmpeg.exe`, `ffplay`, `ffprobe`, `avfilter` and `avdevice` are not included.

## Why FFmpeg at all

A real 8,000 song library turned out to be **88.8% AV1** and 11.2% H.264. A pure-Rust H.264
decoder — which is the only codec Rust has a practical one for — would play one video in nine.
AV1 needs dav1d or FFmpeg, and FFmpeg brings the containers and the colour conversion with it.

## Licensing

This is a **GPL** build of FFmpeg, which suits RungStar being GPL-3.0-or-later. `LICENSE.txt`
is the copy that shipped with it.

## Building against it

`crates/rungstar-video/build.rs` points the linker at `lib/` and copies `bin/*.dll` next to the
built executable. On Linux nothing is vendored and FFmpeg comes from the system package
(`libavcodec-dev`, `libavformat-dev`, `libswscale-dev`).

**Generating the bindings needs libclang**, which `ffmpeg-sys` uses to read the headers:

    winget install LLVM.LLVM

[builds]: https://github.com/BtbN/FFmpeg-Builds
