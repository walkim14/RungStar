# Vendored SDL3

SDL 3.4.14, official prebuilt Windows x64 binaries from
<https://github.com/libsdl-org/SDL/releases/tag/release-3.4.14>.

Vendored rather than built from source because building SDL needs a CMake toolchain that
finds a C compiler, and the Visual Studio generator does not locate one when only the Build
Tools are installed — which is a very common setup. The official binaries are what a release
would ship anyway.

`rungstar-platform/build.rs` points the linker here and copies `SDL3.dll` next to the built
executable.

On Linux, SDL3 comes from the system package (`libsdl3-dev`) or the Flatpak runtime; nothing
is vendored.

SDL is zlib-licensed; see `LICENSE.txt`.
