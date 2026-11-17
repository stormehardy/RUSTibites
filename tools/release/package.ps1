# Builds the portable Windows release zip for RUSTibites.
#
#   pwsh -File tools/release/package.ps1
#
# Outputs (all gitignored, under target/):
#   target/release-package/RUSTibites-<version>-windows-x64/  (portable folder)
#   target/release-package/RUSTibites-<version>-windows-x64.zip
#   target/release-package/SHA256SUMS.txt
#
# The viewer binary is included when it has been built (`rustibites-view.exe`);
# this script never builds the viewer crate itself.

[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$OutputRoot = "target/release-package",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repo
try {
    if (-not $Version) {
        $cargo = Get-Content "Cargo.toml" -Raw
        $m = [regex]::Match($cargo, '(?m)^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"')
        if (-not $m.Success) { throw "cannot read [workspace.package] version from Cargo.toml" }
        $Version = $m.Groups[1].Value
    }
    $commit = (git rev-parse --short HEAD 2>$null); if (-not $commit) { $commit = "unknown" }
    $date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    $rustc = (rustc --version 2>$null); if (-not $rustc) { $rustc = "unknown" }

    if (-not $SkipBuild) {
        Write-Host "==> cargo build --release -p bibites-cli"
        cargo build --release -p bibites-cli
    }
    $exe = Join-Path $repo "target/release/rustibites.exe"
    if (-not (Test-Path $exe)) { throw "missing $exe (build failed or --SkipBuild without a build)" }

    $outRoot = Join-Path $repo $OutputRoot
    $name = "RUSTibites-$Version-windows-x64"
    $stage = Join-Path $outRoot $name
    $zipPath = Join-Path $outRoot "$name.zip"
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
    Remove-Item -Force $zipPath -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $stage | Out-Null

    Copy-Item $exe (Join-Path $stage "rustibites.exe")
    $viewer = Join-Path $repo "target/release/rustibites-view.exe"
    if (Test-Path $viewer) { Copy-Item $viewer (Join-Path $stage "rustibites-view.exe") }
    foreach ($file in @("README.md", "LICENSE-MIT", "LICENSE-APACHE")) {
        if (Test-Path $file) { Copy-Item $file $stage }
    }
    $docs = Join-Path $stage "docs"
    New-Item -ItemType Directory -Force $docs | Out-Null
    foreach ($doc in @("USER_GUIDE.md", "INSTALL.md", "ARCHITECTURE.md", "DEVIATIONS.md", "PARITY.md", "INTERFACES.md", "VIEWER_RELEASE_CHECKLIST.md")) {
        $path = Join-Path $repo "docs/$doc"
        if (Test-Path $path) { Copy-Item $path $docs }
    }
    if (Test-Path "assets") {
        Copy-Item -Recurse "assets" (Join-Path $stage "assets")
    }
    # Runtime HUD data: the viewer discovers the UI atlas + runtime layout
    # relative to its working directory (`tools/ui/...`), so the portable build
    # must ship these small JSON/gzip files (the PNG sprites live under assets).
    $uiStage = Join-Path $stage "tools/ui"
    New-Item -ItemType Directory -Force $uiStage | Out-Null
    foreach ($uiFile in @("ui_atlas.json", "ui_runtime_atlas.json.gz", "ui_runtime_atlas.index.json")) {
        $uiPath = Join-Path $repo "tools/ui/$uiFile"
        if (Test-Path $uiPath) { Copy-Item $uiPath $uiStage }
    }

    @(
        "RUSTibites $Version (windows-x64, portable)",
        "commit:  $commit",
        "built:   $date",
        "rustc:   $rustc",
        "artifacts: rustibites.exe" + ($(if (Test-Path $viewer) { ", rustibites-view.exe" } else { "" })),
        "",
        "Run:  .\rustibites.exe help",
        "See docs/INSTALL.md and docs/USER_GUIDE.md."
    ) | Set-Content (Join-Path $stage "VERSION.txt") -Encoding utf8

    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zipPath -CompressionLevel Optimal
    $hash = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $shaFile = Join-Path $outRoot "SHA256SUMS.txt"
    "$hash  $(Split-Path $zipPath -Leaf)" | Set-Content $shaFile -Encoding ascii

    $sizeMb = [math]::Round((Get-Item $zipPath).Length / 1MB, 2)
    Write-Host "==> $zipPath ($sizeMb MB)"
    Write-Host "==> sha256 $hash"
    Write-Host "==> $shaFile"
} finally {
    Pop-Location
}
