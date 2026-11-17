# Launch the RUSTibites viewer with good defaults and (optionally) verify a
# headless screenshot. Read-only on crates/bibites-view.
#
#   pwsh -File tools/viewer/launch.ps1                 # headless screenshot + verify
#   pwsh -File tools/viewer/launch.ps1 -Interactive    # open the window
#   pwsh -File tools/viewer/launch.ps1 -InputPath testdata/real/Default.zip
#   pwsh -File tools/viewer/launch.ps1 -Fit world

[CmdletBinding()]
param(
    [string]$InputPath = "testdata/real/Default.zip",
    [ValidateSet("bibites", "world")] [string]$Fit = "bibites",
    [string]$Screenshot = "target/viewer-launch.png",
    [int]$Frames = 60,
    [int]$Width = 960,
    [int]$Height = 600,
    [switch]$Interactive,
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repo
try {
    $exe = Join-Path $repo "target/release/rustibites-view.exe"
    if (-not $NoBuild) {
        Write-Host "==> cargo build --release -p bibites-view"
        cargo build --release -p bibites-view
    }
    if (-not (Test-Path $exe)) { throw "missing $exe (build failed?)" }
    if (-not (Test-Path $InputPath)) {
        throw "input not found: $InputPath (testdata/ is gitignored; omit the path for a generated world)"
    }

    $common = @($InputPath, "--width", "$Width", "--height", "$Height", "--fit", $Fit)
    if ($Interactive) {
        Write-Host "==> $exe $($common -join ' ')"
        & $exe @common
        exit $LASTEXITCODE
    }

    $out = Join-Path $repo $Screenshot
    $outDir = Split-Path -Parent $out
    if ($outDir) { New-Item -ItemType Directory -Force $outDir | Out-Null }
    Remove-Item -Force $out -ErrorAction SilentlyContinue
    & $exe @common --screenshot $out --frames $Frames
    if ($LASTEXITCODE -ne 0) { throw "viewer screenshot failed with exit $LASTEXITCODE" }
    if (-not (Test-Path $out)) { throw "screenshot not written: $out" }

    Add-Type -AssemblyName System.Drawing
    $img = [System.Drawing.Image]::FromFile((Resolve-Path $out))
    $size = "$($img.Width)x$($img.Height)"
    $img.Dispose()
    $bytes = (Get-Item $out).Length
    if ($bytes -lt 5000) { throw "screenshot is suspiciously small: $bytes bytes" }
    if ($size -ne "${Width}x${Height}") { throw "screenshot size $size does not match ${Width}x${Height}" }
    Write-Host "==> screenshot $Screenshot ($size, $([int]($bytes / 1KB)) KB) verified"
} finally {
    Pop-Location
}
