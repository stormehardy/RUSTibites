<#
.SYNOPSIS
    Captures the per-phase timing breakdown printed by `rustibites bench` (human output).

.DESCRIPTION
    The engine's JSON report only carries totals; the per-phase split
    (clock, zones, sensing, body, physics, deaths, hash-sync, ...) is printed by
    the human `bench` output (`World::phase_timings()`, always on, sub-microsecond
    overhead). This script runs the human output for a population x threads
    matrix, parses every phase table, and writes:

        phases.csv       one row per population x threads: totals + every phase
        phases.md        Markdown table with the dominant phases
        phases_raw.txt   the raw `bench` stdout for each run (audit trail)
        config.json      exact invocation parameters
        environment.json machine/build metadata (see env.ps1)

    Phase values are *means over all measured ticks*. Use `-Ticks 120` or more so
    the mean is meaningful; use `-Repeat 1` (the default here) because each
    repetition prints its own table.

    Engine phases come from `bibites_core::sim::Phase::ALL` (see
    crates/bibites-core/src/sim.rs and ARCHITECTURE.md section 10).

.EXAMPLE
    # Default: 1k/2k/5k/10k/20k serial, 120 ticks each.
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/bench/run_phases.ps1

.EXAMPLE
    # Thread effect on the phase split for the 2k reference world.
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/bench/run_phases.ps1 -Pops 2000,10000 -Threads 1,8 -Ticks 300

.NOTES
    Read-only with respect to crates/; no rebuild unless the binary is missing
    or -Build is passed.
#>
[CmdletBinding()]
param(
    # String arrays: `powershell -File` passes "-Pops 1000,2000" as one string;
    # ConvertTo-BenchIntList splits commas/semicolons/spaces.
    [string[]]$Pops = @('1000,2000,5000,10000,20000'),
    [string[]]$Threads = @('1'),
    [int]$Ticks = 120,
    [string]$Scenario = '',
    [string]$Exe = '',
    [switch]$Build,
    [string]$OutputDir = '',
    [int]$TimeoutSec = 300,
    [int]$SampleMs = 50,
    [double]$MinFreeGB = 0.5,
    [switch]$UpdateDocs,
    [string]$DocsPath = '',
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'bench_common.ps1')

$optionExe = $Exe
$optionBuild = [bool]$Build
$optionQuiet = [bool]$Quiet
$optionOutputDir = $OutputDir
. (Join-Path $PSScriptRoot 'env.ps1')

$repo = Get-BenchRepo -ScriptRoot $PSScriptRoot
$exeInfo = Resolve-BenchExecutable -Repo $repo -Exe $optionExe -Build:$optionBuild
$resultsRoot = Join-Path (Join-Path (Join-Path $repo 'tools') 'bench') 'results'
$outDir = if ($optionOutputDir) {
    if ([System.IO.Path]::IsPathRooted($optionOutputDir)) { $optionOutputDir } else { Join-Path (Get-Location).Path $optionOutputDir }
} else {
    Join-Path $resultsRoot 'phases'
}
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$scenarioPath = ''
$scenarioLabel = 'synthetic'
if ($Scenario) {
    $scenarioPath = (Resolve-Path -LiteralPath $Scenario).Path
    $scenarioLabel = Split-Path -Leaf $scenarioPath
}

$popList = @((ConvertTo-BenchIntList -Values $Pops) | Where-Object { $_ -ge 0 } | Sort-Object -Unique)
$threadList = @((ConvertTo-BenchIntList -Values $Threads) | Where-Object { $_ -ge 1 } | Sort-Object -Unique)
if ($scenarioPath) { $popList = @(0) }
if ($Ticks -lt 1) { throw '-Ticks must be >= 1' }

$environment = Get-BenchEnvironment -Repo $repo -Executable $exeInfo
Write-BenchEnvironment -Environment $environment -OutputDir $outDir
$startedUtc = (Get-Date).ToUniversalTime()
$config = [ordered]@{
    schema           = 'rustibites.bench.phases/1'
    started_at_utc   = $startedUtc.ToString('o')
    scenario         = $scenarioPath
    scenario_label   = $scenarioLabel
    requested_pops   = $popList
    threads          = $threadList
    ticks_per_run    = $Ticks
    timeout_sec_run  = $TimeoutSec
    binary           = [ordered]@{ original_path = $exeInfo.OriginalPath; measured_copy = $exeInfo.RunPath; sha256 = $exeInfo.Sha256 }
    artifacts        = [ordered]@{ phases_csv = 'phases.csv'; phases_md = 'phases.md'; raw = 'phases_raw.txt' }
    notes            = @(
        'each row is one process, one repetition; phase values are means over all measured ticks',
        'phase table is printed only by the human bench output (JSON has totals only)',
        'phase_total_ms is the sum of the per-phase means; ms_per_tick = 1000 / ticks_per_second'
    )
}
Write-BenchTextFile -Path (Join-Path $outDir 'config.json') -Text ($config | ConvertTo-Json -Depth 6)

function Format-BenchOptional {
    param($Value, [string]$Format = '')
    if ($null -eq $Value) { return '' }
    if ($Format) { return (Format-BenchNumber $Value $Format) }
    return [string]$Value
}

# Markdown table used both by phases.md and by the -UpdateDocs block in
# docs/BENCHMARKS.md.
function Get-PhaseTableLines {
    param($PhaseRows, [string]$TicksPerRun, [string]$ScenarioName)
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add('Ticks per run: ' + $TicksPerRun + '; scenario: `' + $ScenarioName + '`; binary `' + $exeInfo.Sha256 + '`.')
    $lines.Add('')
    $lines.Add('| Population | Threads | ticks/s | xRT | ms/tick | physics | body | sensing | zones | decay | deaths | hash-sync | other |')
    $lines.Add('| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |')
    $highlightPhases = @('physics', 'body', 'sensing', 'zones', 'decay', 'deaths', 'hash-sync')
    foreach ($row in $PhaseRows) {
        if ($null -eq $row.ms_per_tick) {
            $lines.Add('| ' + (Format-BenchOptional $row.population 'N0') + ' | ' + $row.threads + ' | - | - | - | - | - | - | - | - | - | - | ' + $row.status + ' |')
            continue
        }
        $highlightedSum = 0.0
        $cells = @()
        foreach ($phaseName in $highlightPhases) {
            if ($row.phases.Contains($phaseName)) {
                $ms = [double]$row.phases[$phaseName].ms
                $highlightedSum += $ms
                $cells += ((Format-BenchNumber $ms '0.000') + ' (' + (Format-BenchNumber $row.phases[$phaseName].share '0.0') + '%)')
            } else {
                $cells += '-'
            }
        }
        $otherMs = [double]$row.phase_total_ms - $highlightedSum
        $lines.Add('| ' + (Format-BenchOptional $row.population 'N0') + ' | ' + $row.threads + ' | ' +
            (Format-BenchOptional $row.ticks_per_second 'N0') + ' | ' +
            (Format-BenchOptional $row.realtime_factor '0.0') + ' | ' +
            (Format-BenchOptional $row.ms_per_tick '0.000') + ' | ' + ($cells -join ' | ') + ' | ' +
            (Format-BenchNumber $otherMs '0.000') + ' |')
    }
    $lines.Add('')
    $lines.Add('`other` = `phase_total_ms` minus the highlighted phases (the long tail: clock, animators, tools, eggs, pheromones, rad, red-death, colour-killers, producers, events-stats).')
    return $lines.ToArray()
}

# Runs one human-mode bench invocation and returns the parsed run (or $null).
function Invoke-PhaseRun {
    param([int]$Population, [int]$ThreadCount)
    $arguments = @('bench')
    if ($scenarioPath) {
        $arguments += @('--scenario', $scenarioPath)
    } else {
        $arguments += @('--bibites', [string]$Population)
    }
    $arguments += @('--ticks', [string]$Ticks, '--threads', [string]$ThreadCount, '--repeat', '1')
    $result = Invoke-BenchProcess -FilePath $exeInfo.RunPath -ArgumentList $arguments -TimeoutSec $TimeoutSec -SampleMs $SampleMs
    $parsed = @(Parse-BenchPrettyOutput $result.StdOut)
    $run = if ($parsed.Count -gt 0) { $parsed[$parsed.Count - 1] } else { $null }
    return [pscustomobject]@{
        Process  = $result
        Run      = $run
        RawOutput = $result.StdOut
        PeakMB   = if ($result.PeakWorkingSetBytes -gt 0) { [math]::Round([double]$result.PeakWorkingSetBytes / 1MB, 1) } else { $null }
    }
}

$rawSections = New-Object System.Collections.Generic.List[string]
$phaseRows = New-Object System.Collections.Generic.List[object]
$cellIndex = 0
$cellCount = $popList.Count * $threadList.Count
foreach ($population in $popList) {
    foreach ($threadCount in $threadList) {
        $cellIndex++
        $loadBefore = Get-BenchLoadSnapshot
        if ($MinFreeGB -gt 0 -and $null -ne $loadBefore.free_gb -and [double]$loadBefore.free_gb -lt $MinFreeGB) {
            if (-not $optionQuiet) { Write-Host ("[{0}/{1}] pop={2} threads={3}: skipped_low_mem" -f $cellIndex, $cellCount, $population, $threadCount) }
            $phaseRows.Add([ordered]@{ population = $population; threads = $threadCount; status = 'skipped_low_mem'; ticks = $Ticks; bibites = $null; ticks_per_second = $null; realtime_factor = $null; wall_secs = $null; ms_per_tick = $null; phase_total_ms = $null; phases = [ordered]@{}; peak_ws_mb = $null; error = 'free memory below -MinFreeGB' })
            continue
        }

        $invocation = Invoke-PhaseRun -Population $population -ThreadCount $threadCount
        $run = $invocation.Run
        $status = 'ok'
        $errorText = $null
        if ($invocation.Process.TimedOut) {
            $status = 'timeout'
            $errorText = "exceeded -TimeoutSec $TimeoutSec"
        } elseif ($invocation.Process.StdErr -match 'memory allocation of (\d+) bytes failed') {
            $status = 'oom'
            $errorText = "memory allocation of $($matches[1]) bytes failed"
        } elseif ($null -eq $run) {
            $status = 'error'
            $firstErr = @($invocation.Process.StdErr -split "`r?`n" | Where-Object { $_.Trim().Length -gt 0 } | Select-Object -First 1)
            $errorText = if ($firstErr.Count -gt 0) { $firstErr[0].Trim() } else { 'could not parse bench output' }
        }

        $phases = [ordered]@{}
        $msPerTick = $null
        $phaseTotalMs = $null
        if ($null -ne $run) {
            foreach ($phaseName in $script:BenchPhaseNames) {
                if ($run.phases.Contains($phaseName)) {
                    $phases[$phaseName] = [pscustomobject]@{ ms = [double]$run.phases[$phaseName].ms; share = [double]$run.phases[$phaseName].share }
                }
            }
            $phaseTotalMs = [double]$run.total_phase_ms
            if ($run.ticks_per_second -gt 0) { $msPerTick = 1000.0 / [double]$run.ticks_per_second }
        }

        $row = [ordered]@{
            population        = [int]$population
            threads           = [int]$threadCount
            status            = $status
            ticks             = if ($null -ne $run) { [int]$run.ticks } else { $Ticks }
            bibites           = if ($null -ne $run) { [int]$run.bibites } else { $null }
            ticks_per_second  = if ($null -ne $run) { [double]$run.ticks_per_second } else { $null }
            realtime_factor   = if ($null -ne $run) { [double]$run.realtime_factor } else { $null }
            wall_secs         = if ($null -ne $run) { [double]$run.wall_secs } else { $null }
            ms_per_tick       = $msPerTick
            phase_total_ms    = $phaseTotalMs
            phases            = $phases
            peak_ws_mb        = $invocation.PeakMB
            error             = $errorText
        }
        $phaseRows.Add($row)

        $rawSection = New-Object System.Collections.Generic.List[string]
        $rawSection.Add('================================================================================')
        $rawSection.Add(('# pop={0} threads={1} ticks={2} status={3} binary={4}' -f $population, $threadCount, $Ticks, $status, $exeInfo.Sha256))
        $rawSection.Add('================================================================================')
        $rawSection.Add($invocation.RawOutput.TrimEnd())
        $rawSection.Add('')
        $rawSections.Add(($rawSection -join "`r`n"))

        if (-not $optionQuiet) {
            $line = ("[{0}/{1}] pop={2} threads={3}: {4}" -f $cellIndex, $cellCount, $population, $threadCount, $status)
            if ($null -ne $run) {
                $line += ' | ' + (Format-BenchNumber $run.ticks_per_second 'N0') + ' ticks/s, ' + (Format-BenchNumber $msPerTick '0.000') + ' ms/tick'
                $top = @($phases.GetEnumerator() | Sort-Object { [double]$_.Value.ms } -Descending | Select-Object -First 3)
                foreach ($entry in $top) {
                    $line += ', ' + $entry.Key + ' ' + (Format-BenchNumber $entry.Value.ms '0.000') + ' ms'
                }
            } else {
                $line += ' | ' + $errorText
            }
            Write-Host $line
        }
    }
}

Write-BenchTextFile -Path (Join-Path $outDir 'phases_raw.txt') -Text (($rawSections -join "`r`n") + "`r`n")

# --- phases.csv -------------------------------------------------------------
$csvHeaderFields = @('population', 'threads', 'status', 'ticks', 'bibites', 'ticks_per_second', 'realtime_factor', 'wall_secs', 'ms_per_tick', 'phase_total_ms', 'peak_ws_mb')
foreach ($phaseName in $script:BenchPhaseNames) {
    $csvHeaderFields += ($phaseName + '_ms')
    $csvHeaderFields += ($phaseName + '_share_pct')
}
$csvHeaderFields += 'error'
$csvLines = New-Object System.Collections.Generic.List[string]
$csvLines.Add((ConvertTo-BenchCsvLine $csvHeaderFields))
foreach ($row in $phaseRows) {
    $fields = @(
        (Format-BenchOptional $row.population '0')
        (Format-BenchOptional $row.threads '0')
        $row.status
        (Format-BenchOptional $row.ticks '0')
        (Format-BenchOptional $row.bibites '0')
        (Format-BenchOptional $row.ticks_per_second '0.000')
        (Format-BenchOptional $row.realtime_factor '0.000')
        (Format-BenchOptional $row.wall_secs '0.000000')
        (Format-BenchOptional $row.ms_per_tick '0.000000')
        (Format-BenchOptional $row.phase_total_ms '0.000000')
        (Format-BenchOptional $row.peak_ws_mb '0.0')
    )
    foreach ($phaseName in $script:BenchPhaseNames) {
        if ($row.phases.Contains($phaseName)) {
            $fields += (Format-BenchOptional $row.phases[$phaseName].ms '0.000000')
            $fields += (Format-BenchOptional $row.phases[$phaseName].share '0.000')
        } else {
            $fields += ''
            $fields += ''
        }
    }
    $fields += $row.error
    $csvLines.Add((ConvertTo-BenchCsvLine $fields))
}
Write-BenchTextFile -Path (Join-Path $outDir 'phases.csv') -Text (($csvLines -join "`r`n") + "`r`n")

# --- phases.md --------------------------------------------------------------
$phaseTableLines = @(Get-PhaseTableLines -PhaseRows $phaseRows -TicksPerRun ([string]$Ticks) -ScenarioName $scenarioLabel)
$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add('# RUSTibites phase timing breakdown')
$markdown.Add('')
$markdown.Add('_Auto-generated by `tools/bench/run_phases.ps1` at ' + (Get-Date).ToUniversalTime().ToString('o') + '._')
$markdown.Add('')
$markdown.Add('Phase values are means over all measured ticks (`World::phase_timings()`), in ms per tick. `phase_total_ms` is the sum of the phase means; `ms_per_tick` is `1000 / ticks/s`; they should be close (small rounding/mean differences allowed).')
$markdown.Add('')
foreach ($tableLine in $phaseTableLines) { $markdown.Add($tableLine) }
$markdown.Add('')
Write-BenchTextFile -Path (Join-Path $outDir 'phases.md') -Text (($markdown -join "`r`n") + "`r`n")

# --- optional docs update ---------------------------------------------------
if ($UpdateDocs) {
    $docsFile = if ($DocsPath) {
        if ([System.IO.Path]::IsPathRooted($DocsPath)) { $DocsPath } else { Join-Path (Get-Location).Path $DocsPath }
    } else {
        Join-Path (Join-Path $repo 'docs') 'BENCHMARKS.md'
    }
    if (-not (Test-Path -LiteralPath $docsFile)) {
        throw "-UpdateDocs requested but $docsFile does not exist"
    }
    $docsText = [System.IO.File]::ReadAllText($docsFile)
    $pattern = '(?s)<!-- BEGIN BENCH:PHASES -->.*?<!-- END BENCH:PHASES -->'
    if ($docsText -notmatch $pattern) {
        throw "-UpdateDocs: markers '<!-- BEGIN BENCH:PHASES -->' / '<!-- END BENCH:PHASES -->' not found in $docsFile"
    }
    $autoLines = New-Object System.Collections.Generic.List[string]
    $autoLines.Add('<!-- BEGIN BENCH:PHASES -->')
    $autoLines.Add('_Auto-generated by `tools/bench/run_phases.ps1` on ' + (Get-Date).ToUniversalTime().ToString('o') + '._')
    $autoLines.Add('')
    foreach ($tableLine in $phaseTableLines) { $autoLines.Add($tableLine) }
    $autoLines.Add('<!-- END BENCH:PHASES -->')
    $replacement = ($autoLines -join "`r`n")
    $docsText = [regex]::Replace($docsText, $pattern, { param($match) $replacement })
    Write-BenchTextFile -Path $docsFile -Text $docsText
    if (-not $optionQuiet) { Write-Host "updated phase block in $docsFile" }
}

if (-not $optionQuiet) {
    Write-Host ''
    Write-Host "artifacts: $outDir"
    Write-Host '  phases.csv, phases.md, phases_raw.txt, config.json, environment.json, environment.txt'
}

$okRows = @($phaseRows | Where-Object { $_.status -eq 'ok' })
if ($okRows.Count -eq 0) { exit 2 }
exit 0
