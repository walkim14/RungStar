# Assemble a portable Windows build: a folder that runs from anywhere, and a zip of it.
#
# Portable means no install, no registry and no PATH. Everything the executable needs sits
# beside it, because that is the first place Windows looks for a DLL and the only place that
# works from a memory stick.
#
# What is deliberately *not* in it: settings, songs, profiles or the USDB catalog. Those live
# in %APPDATA%\RungStar even for a portable build, so that unzipping a new version over an old
# one cannot take somebody's highscores with it.

param(
    [string]$Configuration = "release",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$out = Join-Path $root "target\package\RungStar"
$zip = Join-Path $root "target\package\RungStar-windows-x64.zip"

if (-not $SkipBuild) {
    Write-Host "building..." -ForegroundColor Cyan
    Push-Location $root
    try {
        cargo build --release -p rungstar-app
        if ($LASTEXITCODE -ne 0) { throw "the build failed" }
    } finally {
        Pop-Location
    }
}

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force -Path $out | Out-Null

$built = Join-Path $root "target\$Configuration"
Copy-Item (Join-Path $built "rungstar.exe") $out
foreach ($tool in @("rungstar-diagnostics.exe", "rungstar-sing.exe")) {
    $path = Join-Path $built $tool
    if (Test-Path $path) { Copy-Item $path $out }
}

# The DLLs the build script already put beside the executable: SDL3 and the FFmpeg libraries
# the game links. Copied by wildcard rather than by name so a version bump in vendor/ does not
# silently produce a zip that cannot start.
$dlls = Get-ChildItem -Path $built -Filter *.dll -File
if ($dlls.Count -eq 0) { throw "no DLLs beside the executable; the build script did not run" }
foreach ($dll in $dlls) { Copy-Item $dll.FullName $out }

# ffmpeg.exe, which is not for the game - it links the libraries directly and never runs the
# program. yt-dlp runs it, to pull audio out of a container and to merge the separate video and
# audio streams YouTube serves above 360p, and it looks for ffmpeg on the PATH and nowhere else.
# Without this the download screen works right up to the last step and then says ffmpeg is not
# installed, which is how this was found.
$ffmpeg = Join-Path $built "ffmpeg.exe"
if (Test-Path $ffmpeg) {
    Copy-Item $ffmpeg $out
} else {
    Write-Warning "no ffmpeg.exe beside the executable - downloaded videos will be low quality"
}

# Assets, themes and the licence. The licence is not optional: this is GPL-3.0-or-later and a
# binary without it is a licence violation, not an oversight.
foreach ($item in @("assets", "LICENSE", "NOTICE.md", "README.md")) {
    $path = Join-Path $root $item
    if (Test-Path $path) {
        Copy-Item -Recurse -Force $path (Join-Path $out (Split-Path $item -Leaf))
    }
}

# yt-dlp, beside the executable. A release that cannot download a song until it has downloaded
# something else first is a release with a hole in it, and the game looks here before it looks
# anywhere but the PATH. It still fetches a newer one into the data directory when it needs to,
# because a copy frozen at release time stops working within a few months.
$ytdlp = Join-Path $out "yt-dlp.exe"
try {
    Write-Host "fetching yt-dlp..." -ForegroundColor Cyan
    $release = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    Invoke-WebRequest -Uri $release -OutFile $ytdlp -UseBasicParsing
    if ((Get-Item $ytdlp).Length -lt 500KB) {
        throw "what GitHub sent back was not a program"
    }
} catch {
    Remove-Item -Force -ErrorAction SilentlyContinue $ytdlp
    Write-Warning "could not bundle yt-dlp ($_). The game will fetch it on first download."
}

# Fonts and sounds are committed, so a missing one is a broken checkout rather than a step
# somebody forgot. Fail rather than quietly shipping a build that borrows a system face and
# plays nothing - that difference is invisible until somebody runs the release.
foreach ($needed in @("assets\fonts\RungStar-Regular.ttf", "assets\sounds\move.wav", "ffmpeg.exe")) {
    if (-not (Test-Path (Join-Path $out $needed))) {
        throw "$needed is missing from the staged build. See packaging/README.md."
    }
}

if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $out -DestinationPath $zip
$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "packaged $out" -ForegroundColor Green
Write-Host "zipped   $zip ($size MB)" -ForegroundColor Green
