# Builds the game-load verification fixture pack under testdata/parity/compat.
#
#   pwsh -File tools/roundtrip/gen_compat_pack.ps1
#
# The pack is gitignored. It contains Rust-written saves/scenarios/templates that
# exercise every writer surface, plus a manifest with SHA-256, counts/ids and the
# exact game load/re-save commands for parity-dev's hidden-game verification.

[CmdletBinding()]
param(
    [string]$OutDir = "testdata/parity/compat",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repo
try {
    if (-not $SkipBuild) {
        cargo build --release -p bibites-cli -p bibites-io
    }
    $dir = Join-Path $repo $OutDir
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $dir | Out-Null

    Write-Host "==> io fixtures"
    cargo run -q --release -p bibites-io --example compat_fixtures -- $OutDir
    if ($LASTEXITCODE -ne 0) { throw "compat_fixtures failed" }

    Write-Host "==> CLI 0.6 scenario-start save"
    & target\release\rustibites.exe run testdata\real\Default.zip --ticks 0 --seed 12345 `
        --stats json --quiet --save (Join-Path $OutDir "rust_default_scenario_save.zip")
    if ($LASTEXITCODE -ne 0) { throw "CLI scenario save failed" }

    Write-Host "==> manifest"
    python tools\roundtrip\make_compat_manifest.py --dir $OutDir
    if ($LASTEXITCODE -ne 0) { throw "manifest generation failed" }
    Write-Host "==> pack ready: $dir"
} finally {
    Pop-Location
}
