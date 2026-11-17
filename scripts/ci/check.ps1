# RUSTibites local gate (contributor one-command check).
#
# Usage:
#   pwsh -File scripts/ci/check.ps1                 # fmt + clippy + tests
#   pwsh -File scripts/ci/check.ps1 -Bench          # ... + release bench smoke
#   pwsh -File scripts/ci/check.ps1 -Bench -BenchBibites 10000 -BenchTicks 300
#   powershell -ExecutionPolicy Bypass -File scripts\ci\check.ps1
#
# This is a superset of the older scripts/check.ps1 (which stays as the quick
# three-command gate). Exits 0 only when every selected step passes.

[CmdletBinding()]
param(
    [switch]$Bench,
    [int]$BenchBibites = 2000,
    [int]$BenchTicks = 300,
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$script:results = @()
$script:started = Get-Date

function Write-Section([string]$Title) {
    Write-Host ""
    Write-Host "== $Title ==" -ForegroundColor Cyan
}

function Record-Result([string]$Name, [int]$Code, [double]$Seconds) {
    $passed = ($Code -eq 0)
    $elapsed = [math]::Round($Seconds, 1)
    if ($passed) {
        Write-Host ("PASS  {0}  ({1}s)" -f $Name, $elapsed) -ForegroundColor Green
    } else {
        Write-Host ("FAIL  {0}  (exit {1}, {2}s)" -f $Name, $Code, $elapsed) -ForegroundColor Red
    }
    $script:results += [pscustomobject]@{
        Name = $Name
        Passed = $passed
        Code = $Code
        Seconds = $elapsed
    }
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "FAIL: cargo not found on PATH. Install stable Rust from https://rustup.rs and retry." -ForegroundColor Red
    exit 127
}
if (-not (Test-Path (Join-Path $root "Cargo.toml"))) {
    Write-Host "FAIL: $root is not the RUSTibites workspace (Cargo.toml missing)." -ForegroundColor Red
    exit 126
}

# Preflight: the gate needs the rustfmt and clippy toolchain components. Report
# the install command instead of letting `cargo fmt`/`cargo clippy` fail obscurely.
function Test-CargoComponent([string]$Component) {
    try {
        $null = & cargo $Component --version 2>&1
        return ($LASTEXITCODE -eq 0)
    } catch {
        return $false
    }
}
$fmtAvailable = Test-CargoComponent 'fmt'
$clippyAvailable = Test-CargoComponent 'clippy'

$rustc = (& rustc --version) 2>&1
$cargoVersion = (& cargo --version) 2>&1
Write-Host "RUSTibites local gate" -ForegroundColor White
Write-Host "  root:  $root"
Write-Host "  rust:  $rustc"
Write-Host "  cargo: $cargoVersion"
Write-Host "  fmt:   $fmtAvailable   clippy: $clippyAvailable"
if (-not $fmtAvailable -or -not $clippyAvailable) {
    Write-Host "  note:  install missing components with: rustup component add rustfmt clippy" -ForegroundColor Yellow
}
if (-not (Test-Path (Join-Path $root "testdata"))) {
    Write-Host "  note:  testdata/ absent (game-dependent tests skip; see docs/CI.md)" -ForegroundColor Yellow
}

# 1. Formatting
if (-not $fmtAvailable) {
    Write-Host "FAIL  fmt  (rustfmt component not installed)" -ForegroundColor Red
    $script:results += [pscustomobject]@{ Name = "fmt"; Passed = $false; Code = 127; Seconds = 0 }
} else {
    Write-Section "cargo fmt --all -- --check"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    cargo fmt --all -- --check
    $sw.Stop()
    Record-Result "fmt" $LASTEXITCODE $sw.Elapsed.TotalSeconds
}

# 2. Clippy (deny warnings)
if (-not $clippyAvailable) {
    Write-Host "FAIL  clippy  (clippy component not installed)" -ForegroundColor Red
    $script:results += [pscustomobject]@{ Name = "clippy"; Passed = $false; Code = 127; Seconds = 0 }
} else {
    Write-Section "cargo clippy --workspace --all-targets -- -D warnings"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    cargo clippy --workspace --all-targets -- -D warnings
    $sw.Stop()
    Record-Result "clippy" $LASTEXITCODE $sw.Elapsed.TotalSeconds
}

# 3. Tests
if ($SkipTests) {
    Write-Host "SKIP  tests (-SkipTests)" -ForegroundColor Yellow
} else {
    Write-Section "cargo test --workspace"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    cargo test --workspace
    $sw.Stop()
    Record-Result "tests" $LASTEXITCODE $sw.Elapsed.TotalSeconds
}

# 4. Optional release bench smoke (also proves the release profile builds).
if ($Bench) {
    Write-Section "release bench smoke"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    cargo run --release --quiet --bin rustibites -- bench `
        --bibites $BenchBibites --ticks $BenchTicks --threads 1 --repeat 1
    $sw.Stop()
    Record-Result "release bench" $LASTEXITCODE $sw.Elapsed.TotalSeconds
}

# Summary
$elapsed = [math]::Round(((Get-Date) - $script:started).TotalSeconds, 1)
$failed = @($script:results | Where-Object { -not $_.Passed })
Write-Host ""
Write-Host "=============== gate summary ===============" -ForegroundColor White
foreach ($result in $script:results) {
    $label = if ($result.Passed) { "PASS" } else { "FAIL" }
    $color = if ($result.Passed) { "Green" } else { "Red" }
    Write-Host ("  {0}  {1,-16} {2,8}s" -f $label, $result.Name, $result.Seconds) -ForegroundColor $color
}
Write-Host ("  total {0}s" -f $elapsed)
if ($failed.Count -gt 0) {
    Write-Host ("RESULT: FAIL ({0} of {1} steps failed)" -f $failed.Count, $script:results.Count) -ForegroundColor Red
    exit 1
}
Write-Host ("RESULT: PASS ({0} steps)" -f $script:results.Count) -ForegroundColor Green
exit 0
