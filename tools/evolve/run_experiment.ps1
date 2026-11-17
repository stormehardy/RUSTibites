#!/usr/bin/env pwsh
# Checkpointed long-run evolution experiments (task-61).
#
# Runs the release CLI in fixed-size chunks, saving a checkpoint after each chunk
# and resuming from it, so a long experiment can be stopped/resumed and every
# chunk's stats are kept. Results go to testdata/evolve/<run-id>/.
#
# Usage (canonical testing-policy world first; historical fixtures labelled):
#   pwsh -File tools/evolve/run_experiment.ps1 -WorldPath testdata/real/Default.zip \
#        -RunId default_baseline -Chunks 5 -ChunkTicks 2000 -Threads 8
#   # historical evolved-save fixture (only for documented regressions):
#   pwsh -File tools/evolve/run_experiment.ps1 -WorldPath testdata/real/saves/lol.zip \
#        -RunId lol_100k -Chunks 50 -ChunkTicks 2000 -Threads 8
#
# Continuation guarantee is checked by tools/evolve/check_resume.ps1 (or -Verify
# on this script): chunked run fingerprint must equal a single-run fingerprint.
param(
    [Parameter(Mandatory = $true)][string]$WorldPath,
    [Parameter(Mandatory = $true)][string]$RunId,
    [int]$Chunks = 50,
    [int]$ChunkTicks = 2000,
    [int]$Threads = 8,
    [int]$Seed = 12345,
    [string]$Rustibites = "",
    [switch]$CaptureFinal,
    [switch]$Verify
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root
if (-not $Rustibites) {
    $Rustibites = Join-Path $root "target\release\rustibites.exe"
}
if (-not (Test-Path $Rustibites)) {
    throw "missing release binary $Rustibites (run: cargo build --release -p bibites-cli)"
}
if (-not (Test-Path $WorldPath)) { throw "missing input $WorldPath" }

$outDir = Join-Path $root "testdata\evolve\$RunId"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$statsPath = Join-Path $outDir "stats.jsonl"
$logPath = Join-Path $outDir "runner.log"
$summaryPath = Join-Path $outDir "summary.json"
if (Test-Path $statsPath) { Remove-Item $statsPath }
if (Test-Path $logPath) { Remove-Item $logPath }

function Invoke-Run {
    param([string]$Source, [int]$Ticks, [string]$SavePath, [string]$CaptureDir)
    $cliArgs = @(
        "run", $Source, "--ticks", "$Ticks", "--seed", "$Seed",
        "--threads", "$Threads", "--save", $SavePath, "--stats", "json"
    )
    if ($CaptureDir) { $cliArgs += @("--capture", $CaptureDir) }
    $cliArgs += "--quiet"
    # Windows PowerShell 5.1 surfaces native stderr as a terminating error when
    # $ErrorActionPreference is Stop (the CLI logs template counts to stderr);
    # keep those lines in runner.log and rely on $LASTEXITCODE instead.
    $previousEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $Rustibites @cliArgs 2>> $logPath
    } finally {
        $ErrorActionPreference = $previousEap
    }
    if ($LASTEXITCODE -ne 0) { throw "rustibites exit $LASTEXITCODE for $Source" }
    $line = ($output | Where-Object { $_ -match '^\{' } | Select-Object -Last 1)
    if (-not $line) { throw "no JSON stats from $Source" }
    return ($line | ConvertFrom-Json)
}

$current = (Resolve-Path $WorldPath).Path
$records = New-Object System.Collections.Generic.List[object]
$startWall = Get-Date
for ($chunk = 0; $chunk -lt $Chunks; $chunk++) {
    $save = Join-Path $outDir ("checkpoint_{0:D3}.zip" -f $chunk)
    $capture = $null
    if ($CaptureFinal -and $chunk -eq ($Chunks - 1)) {
        $capture = Join-Path $outDir "capture_final"
    }
    $t0 = Get-Date
    $stats = Invoke-Run -Source $current -Ticks $ChunkTicks -SavePath $save -CaptureDir $capture
    $elapsed = ((Get-Date) - $t0).TotalSeconds
    $record = [ordered]@{
        chunk      = $chunk
        source     = $current
        chunk_ticks = $ChunkTicks
        wall_s     = [math]::Round($elapsed, 3)
        ticks      = $stats.ticks
        bibites_start = $stats.bibites_start
        bibites_end   = $stats.bibites_end
        ticks_per_second = $stats.ticks_per_second
        realtime_factor  = $stats.realtime_factor
        state_fingerprint = $stats.state_fingerprint
    }
    $records.Add([pscustomobject]$record) | Out-Null
    Add-Content -Path $statsPath -Value ($record | ConvertTo-Json -Compress)
    Write-Host ("chunk {0}/{1}: {2} bibites, {3:N0} tps, {4:N1}s" -f ($chunk + 1), $Chunks, $stats.bibites_end, $stats.ticks_per_second, $elapsed)
    $current = $save
    if ($stats.bibites_end -eq 0) {
        Write-Warning "population extinct at chunk $chunk; stopping"
        break
    }
}

$totalWall = ((Get-Date) - $startWall).TotalSeconds
$first = $records[0]
$last = $records[-1]
$summary = [ordered]@{
    run_id          = $RunId
    input           = (Resolve-Path $WorldPath).Path
    seed            = $Seed
    threads         = $Threads
    chunks          = $records.Count
    chunk_ticks     = $ChunkTicks
    total_ticks     = [int]$records.Count * $ChunkTicks
    wall_s          = [math]::Round($totalWall, 3)
    bibites_start   = $first.bibites_start
    bibites_end     = $last.bibites_end
    ticks_per_second = [math]::Round(([int]$records.Count * $ChunkTicks) / [math]::Max($totalWall, 1e-9), 1)
    realtime_factor = [math]::Round((([int]$records.Count * $ChunkTicks) / 30.0) / [math]::Max($totalWall, 1e-9), 2)
    final_fingerprint = $last.state_fingerprint
    checkpoints     = ($records.Count)
    final_capture   = if ($CaptureFinal) { "capture_final" } else { $null }
}
$summary | ConvertTo-Json | Set-Content -Path $summaryPath
$summary | ConvertTo-Json

if ($Verify) {
    # Chunked vs single-run continuation check: same world/seed/ticks.
    $verifyDir = Join-Path $outDir "verify"
    New-Item -ItemType Directory -Force -Path $verifyDir | Out-Null
    $chunkLen = 500
    $half = 2
    $one = $half * $chunkLen
    $singleSave = Join-Path $verifyDir "single.zip"
    $single = Invoke-Run -Source (Resolve-Path $WorldPath).Path -Ticks $one -SavePath $singleSave
    $source = (Resolve-Path $WorldPath).Path
    for ($i = 0; $i -lt $half; $i++) {
        $save = Join-Path $verifyDir ("step_{0}.zip" -f $i)
        $st = Invoke-Run -Source $source -Ticks $chunkLen -SavePath $save
        $source = $save
    }
    $chunked = $st
    if ($single.state_fingerprint -ne $chunked.state_fingerprint) {
        throw "RESUME MISMATCH: single-run fingerprint $($single.state_fingerprint) != chunked $($chunked.state_fingerprint)"
    }
    Write-Host "RESUME OK: single and chunked fingerprints both $($single.state_fingerprint)"
}
