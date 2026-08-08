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

# The DLLs the build script already put beside the executable: SDL3 and the five FFmpeg
# libraries the game links. Copied by wildcard rather than by name so a version bump in
# vendor/ does not silently produce a zip that cannot start.
$dlls = Get-ChildItem -Path $built -Filter *.dll -File
if ($dlls.Count -eq 0) { throw "no DLLs beside the executable; the build script did not run" }
foreach ($dll in $dlls) { Copy-Item $dll.FullName $out }

# Assets, themes and the licence. The licence is not optional: this is GPL-3.0-or-later and a
# binary without it is a licence violation, not an oversight.
foreach ($item in @("assets", "LICENSE", "NOTICE.md", "README.md")) {
    $path = Join-Path $root $item
    if (Test-Path $path) {
        Copy-Item -Recurse -Force $path (Join-Path $out (Split-Path $item -Leaf))
    }
}

$font = Join-Path $out "assets\fonts\RungStar-Regular.ttf"
if (-not (Test-Path $font)) {
    Write-Warning "no bundled font in assets/fonts - the build will borrow one from the system. See packaging/README.md."
}

if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $out -DestinationPath $zip
$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "packaged $out" -ForegroundColor Green
Write-Host "zipped   $zip ($size MB)" -ForegroundColor Green
