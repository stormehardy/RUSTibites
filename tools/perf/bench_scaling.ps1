<#
.SYNOPSIS
    Population-scaling sweep for the RUSTibites release benchmark.

.DESCRIPTION
    Runs `rustibites bench --bibites N --ticks T --repeat R --json` for every
    population in a list, then writes one CSV row per run with the reported
    wall time/throughput plus the process wall time, sampled peak working set
    and failure signature.

    Why this exists: the benchmark's synthetic spawner currently places every
    starter bibite at the same position (constant `Rng::fork` stream tag in
    crates/bibites-cli/src/engine.rs:274; see docs/PERF_DIAGNOSIS.md). The
    timings below are therefore the pathological coincident-population case,
    not a uniform-random population. Keep `-Ticks` small (default 1): a tick at
    N=4000 costs tens of seconds and several GiB.

    Per-row status:
      ok               JSON report parsed and exit code 0
      oom              allocator abort ("memory allocation of ... bytes failed")
      timeout          exceeded -TimeoutSec; process tree killed
      error            non-zero exit or no parsable JSON report
      skipped_low_mem  free physical memory below -MinFreeGB before launch

.PARAMETER N
    Populations to sweep. Defaults cover the range in the bug report.

.PARAMETER Ticks
    Ticks per run (`bench --ticks`). Default 1.

.PARAMETER Repeat
    Repetitions inside one process (`bench --repeat`). Default 1 so each row is
    one world; the sweep itself repeats across populations.

.PARAMETER OutFile
    CSV destination. Defaults to bench_scaling.csv next to this script.

.PARAMETER Config
    Cargo profile: release (default) or a custom profile name.

.PARAMETER SkipBuild
    Use the existing binary without running cargo.

.PARAMETER TimeoutSec
    Per-run wall-clock limit. Default 900 s.

.PARAMETER MinFreeGB
    Skip a run when free physical memory is below this many GiB. Default 2.
    Set 0 to disable the guard (e.g. to reproduce an OOM deliberately).

.EXAMPLE
    ./tools/perf/bench_scaling.ps1

.EXAMPLE
    ./tools/perf/bench_scaling.ps1 -N 500,1000,2000,3000 -Ticks 5 -OutFile .\scaling.csv

.NOTES
    Read-only with respect to crates/: it builds the existing workspace and
    writes only the CSV.
#>
[CmdletBinding()]
param(
    [int[]]$N = @(500, 1000, 2000, 3000, 4000, 5000, 6000, 8000, 10000),
    [int]$Ticks = 1,
    [int]$Repeat = 1,
    [string]$OutFile = (Join-Path $PSScriptRoot 'bench_scaling.csv'),
    [string]$Config = 'release',
    [switch]$SkipBuild,
    [int]$TimeoutSec = 900,
    [double]$MinFreeGB = 2
)

$ErrorActionPreference = 'Stop'

if ($Ticks -lt 0) { throw '-Ticks must be >= 0' }
if ($Repeat -lt 1) { throw '-Repeat must be >= 1' }
if ($TimeoutSec -lt 1) { throw '-TimeoutSec must be >= 1' }

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$manifest = Join-Path $repo 'Cargo.toml'
$targetRoot = if ($env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR
} else {
    Join-Path $repo 'target'
}
$profileDir = if ($Config -eq 'release') { 'release' } else { $Config }
$exe = Join-Path $targetRoot (Join-Path $profileDir 'rustibites.exe')

function Get-FreeGB {
    try {
        $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
        return [math]::Round($os.FreePhysicalMemory / 1MB, 2)
    } catch {
        return [double]::PositiveInfinity
    }
}

if (-not $SkipBuild) {
    Write-Host "building $Config bibites-cli ..."
    $buildArgs = @('build', '-p', 'bibites-cli', '--manifest-path', $manifest)
    if ($Config -eq 'release') { $buildArgs += '--release' } else { $buildArgs += @('--profile', $Config) }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
}

if (-not (Test-Path $exe)) {
    throw "benchmark binary not found: $exe (run: cargo build --release -p bibites-cli)"
}

# Keep a private copy of the population list: the loop variable must not be
# named $n because PowerShell variable names are case-insensitive and would
# alias the $N parameter.
$populations = @($N | Sort-Object -Unique)
$rows = New-Object System.Collections.Generic.List[object]
Write-Host ("sweep: N=[{0}] ticks={1} repeat={2}" -f ($populations -join ','), $Ticks, $Repeat)
Write-Host ("binary: {0}" -f $exe)
if (($populations | Measure-Object -Maximum).Maximum -ge 8000) {
    Write-Warning ("N>=8000 is expected to request 20+ GiB of physics scratch; on hosts " +
        "with less than ~64 GiB this can OOM or page heavily. Limit with -N, or see " +
        "docs/PERF_DIAGNOSIS.md §5.")
}

foreach ($population in $populations) {
    if ($population -lt 0) { continue }

    $freeGB = Get-FreeGB
    if ($MinFreeGB -gt 0 -and $freeGB -lt $MinFreeGB) {
        Write-Warning ("N={0}: skipping, only {1} GiB free (MinFreeGB={2})" -f $population, $freeGB, $MinFreeGB)
        $rows.Add([pscustomobject]@{
            n                         = $population
            ticks                     = $Ticks
            repeat                    = $Repeat
            status                    = 'skipped_low_mem'
            exit_code                 = $null
            process_wall_secs         = $null
            report_wall_secs          = $null
            ticks_per_second          = $null
            bibite_updates_per_second = $null
            bibites_start             = $null
            bibites_end               = $null
            peak_ws_sampled_mb        = $null
            free_gb_before            = $freeGB
            error                     = $null
        })
        continue
    }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $exe
    $startInfo.Arguments = "bench --bibites $population --ticks $Ticks --repeat $Repeat --json"
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.CreateNoWindow = $true

    Write-Host ("N={0} ... " -f $population) -NoNewline
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = [System.Diagnostics.Process]::Start($startInfo)
    $stdoutTask = $proc.StandardOutput.ReadToEndAsync()
    $stderrTask = $proc.StandardError.ReadToEndAsync()

    # Sample working set while the process is alive: .NET's PeakWorkingSet64 is
    # not populated after exit for a redirected child process.
    $peakBytes = 0L
    $timedOut = $false
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSec)
    while (-not $proc.HasExited) {
        if ([DateTime]::UtcNow -gt $deadline) {
            $timedOut = $true
            try { $proc.Kill() } catch { }
            break
        }
        try {
            $proc.Refresh()
            if (-not $proc.HasExited -and $proc.WorkingSet64 -gt $peakBytes) {
                $peakBytes = $proc.WorkingSet64
            }
        } catch {
            # Process exited between HasExited and Refresh; ignore.
        }
        Start-Sleep -Milliseconds 100
    }
    $proc.WaitForExit()
    $sw.Stop()

    $exitCode = $null
    try { $exitCode = $proc.ExitCode } catch { }
    $stdout = ''
    $stderr = ''
    try { $stdout = $stdoutTask.Result } catch { }
    try { $stderr = $stderrTask.Result } catch { }

    $report = $null
    if ($stdout) {
        $jsonLine = ($stdout -split "`r?`n" | Where-Object { $_ -match '^\s*\{' } | Select-Object -Last 1)
        if ($jsonLine) {
            try { $report = $jsonLine | ConvertFrom-Json } catch { $report = $null }
        }
    }

    $status = 'ok'
    $errorText = $null
    if ($timedOut) {
        $status = 'timeout'
        $errorText = "exceeded ${TimeoutSec}s"
    } elseif ($stderr -match 'memory allocation of (\d+) bytes failed') {
        $status = 'oom'
        $errorText = "memory allocation of $($Matches[1]) bytes failed"
    } elseif ($exitCode -ne 0 -or -not $report) {
        $status = 'error'
        $firstErr = ($stderr -split "`r?`n" | Where-Object { $_.Trim().Length -gt 0 } | Select-Object -First 1)
        $errorText = if ($firstErr) { $firstErr.Trim() } else { "exit code $exitCode, no JSON report" }
    }

    $peakMB = if ($peakBytes -gt 0) { [math]::Round($peakBytes / 1MB, 1) } else { $null }

    $rows.Add([pscustomobject]@{
        n                         = $population
        ticks                     = $Ticks
        repeat                    = $Repeat
        status                    = $status
        exit_code                 = $exitCode
        process_wall_secs         = [math]::Round($sw.Elapsed.TotalSeconds, 6)
        report_wall_secs          = if ($report) { $report.wall_secs } else { $null }
        ticks_per_second          = if ($report) { $report.ticks_per_second } else { $null }
        bibite_updates_per_second = if ($report) { $report.bibite_updates_per_second } else { $null }
        bibites_start             = if ($report) { $report.bibites_start } else { $null }
        bibites_end               = if ($report) { $report.bibites_end } else { $null }
        peak_ws_sampled_mb        = $peakMB
        free_gb_before            = $freeGB
        error                     = $errorText
    })

    $summary = if ($report) {
        "ok {0:N3}s ({1:N0} updates/s, {2} MB peak)" -f $report.wall_secs, $report.bibite_updates_per_second, $peakMB
    } else {
        "$status $(if ($errorText) { $errorText } else { '' })"
    }
    Write-Host $summary
}

$outFull = if ([System.IO.Path]::IsPathRooted($OutFile)) { $OutFile } else { Join-Path (Get-Location).Path $OutFile }
$outDir = Split-Path -Parent $outFull
if ($outDir -and -not (Test-Path $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
$rows | Export-Csv -Path $outFull -NoTypeInformation -Encoding UTF8
Write-Host ""
Write-Host ("wrote {0} rows to {1}" -f $rows.Count, $outFull)
$rows | Format-Table n, ticks, status, report_wall_secs, bibite_updates_per_second, peak_ws_sampled_mb -AutoSize
