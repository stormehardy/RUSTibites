<#
.SYNOPSIS
    Reproducible benchmark matrix for RUSTibites: populations x worker threads.

.DESCRIPTION
    Runs the release CLI's benchmark harness (`rustibites bench ... --json`) for a
    matrix of synthetic populations and worker-thread counts, then writes:

        runs.csv          one row per process/repetition (raw, parseable)
        summary.csv       one row per population x threads cell (aggregated)
        matrix.md         Markdown tables (throughput, thread scaling, detail)
        matrix.json       config + aggregated cells + determinism checks
        config.json       exact invocation parameters
        environment.json  machine/build metadata (see env.ps1)
        environment.txt   human-readable environment summary

    The binary is snapshotted to a temporary directory before the matrix starts,
    so a concurrent `cargo build` cannot change the measured binary mid-run and
    the matrix does not lock target/release against other tools. Its SHA-256 is
    recorded in environment.json.

    Modes (explicit parameters override the mode's defaults):
        quick: populations 1k/2k/5k/10k/20k, threads 1/4/8, 30 ticks, repeat 1
        full:  populations 1k..100k, threads 1/4/8/12, 60 ticks, repeat 3

    `bench --json` reports the engine's own `wall_secs`, which covers only
    `world.run(ticks)` (setup/spawn excluded). This script also measures the whole
    process (`process_wall_secs`) and samples peak working set, because `bench`
    reports `peak_bytes = null`.

.EXAMPLE
    # Quick matrix on an existing release build (no rebuild), ~1-2 min.
    pwsh -File tools/bench/run_matrix.ps1 -Mode quick

.EXAMPLE
    # Full matrix, one repetition, 60 ticks, bounded to ~8 minutes of wall time.
    pwsh -File tools/bench/run_matrix.ps1 -Mode full -Repeat 1 -BudgetSec 480

.EXAMPLE
    # Custom matrix, best-of-3 on a quiet machine.
    pwsh -File tools/bench/run_matrix.ps1 -Pops 1000,2000,10000 -Threads 1,8 -Ticks 120 -Repeat 3

.EXAMPLE
    # Benchmark the canonical real scenario (testing policy) instead of the synthetic world.
    # Other real saves/scenarios are historical fixtures: keep the world name with the numbers.
    pwsh -File tools/bench/run_matrix.ps1 -Scenario testdata/real/Default.zip -Threads 1,8 -Ticks 300

.NOTES
    - Read-only with respect to crates/: builds only on -Build or a missing binary.
    - `-UpdateDocs` replaces the block between <!-- BEGIN BENCH:MATRIX --> and
      <!-- END BENCH:MATRIX --> in docs/BENCHMARKS.md with the generated table.
    - `-DeterminismCheck` reruns a couple of cells in separate processes and
      compares `state_fingerprint` (same seed => identical state).
#>
[CmdletBinding()]
param(
    [ValidateSet('quick', 'full')][string]$Mode = 'quick',
    # String arrays: `powershell -File` passes "-Pops 1000,2000" as one string;
    # ConvertTo-BenchIntList splits commas/semicolons/spaces.
    [string[]]$Pops,
    [string[]]$Threads,
    [int]$Ticks = 0,
    [int]$Repeat = 0,
    [string]$Scenario = '',
    [string]$Exe = '',
    [switch]$Build,
    [string]$OutputDir = '',
    [string]$RunLabel = '',
    [int]$TimeoutSec = 0,
    [int]$BudgetSec = 0,
    [int]$SampleMs = 50,
    [double]$MinFreeGB = 0.5,
    [double]$MaxLoadPct = 0,
    [int]$LoadWaitSec = 120,
    [switch]$DeterminismCheck,
    [switch]$UpdateDocs,
    [string]$DocsPath = '',
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'bench_common.ps1')

# env.ps1 has its own param block for standalone use; dot-sourcing it would
# overwrite our parameters, so preserve the values we need first.
$optionExe = $Exe
$optionBuild = [bool]$Build
$optionQuiet = [bool]$Quiet
$optionOutputDir = $OutputDir
$optionDocsPath = $DocsPath
$optionUpdateDocs = [bool]$UpdateDocs
. (Join-Path $PSScriptRoot 'env.ps1')

# Mode presets. $PSBoundParameters tells us which parameters the user supplied so
# explicit values win over the mode defaults.
if ($Mode -eq 'quick') {
    $defaultPops = @(1000, 2000, 5000, 10000, 20000)
    $defaultThreads = @(1, 4, 8)
    $defaultTicks = 30
    $defaultRepeat = 1
    $defaultTimeout = 180
} else {
    $defaultPops = @(1000, 2000, 5000, 10000, 20000, 50000, 100000)
    $defaultThreads = @(1, 4, 8, 12)
    $defaultTicks = 60
    $defaultRepeat = 3
    $defaultTimeout = 900
}
if (-not $PSBoundParameters.ContainsKey('Pops')) { $Pops = @($defaultPops -join ',') }
if (-not $PSBoundParameters.ContainsKey('Threads')) { $Threads = @($defaultThreads -join ',') }
if ($Ticks -le 0) { $Ticks = $defaultTicks }
if ($Repeat -le 0) { $Repeat = $defaultRepeat }
if ($TimeoutSec -le 0) { $TimeoutSec = $defaultTimeout }

$repo = Get-BenchRepo -ScriptRoot $PSScriptRoot
$exeInfo = Resolve-BenchExecutable -Repo $repo -Exe $optionExe -Build:$optionBuild
$resultsRoot = Join-Path (Join-Path (Join-Path $repo 'tools') 'bench') 'results'
$outDir = if ($optionOutputDir) {
    if ([System.IO.Path]::IsPathRooted($optionOutputDir)) { $optionOutputDir } else { Join-Path (Get-Location).Path $optionOutputDir }
} else {
    Join-Path $resultsRoot $Mode
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
if ($scenarioPath) { $popList = @(0) }  # population comes from the scenario
if ($popList.Count -eq 0) { throw 'no populations to benchmark (check -Pops)' }
if ($threadList.Count -eq 0) { throw 'no thread counts to benchmark (check -Threads)' }

# --- configuration + environment artifacts ----------------------------------
$environment = Get-BenchEnvironment -Repo $repo -Executable $exeInfo
Write-BenchEnvironment -Environment $environment -OutputDir $outDir

$startedUtc = (Get-Date).ToUniversalTime()
$runLabel = if ($RunLabel) { $RunLabel } else { $Mode }
$config = [ordered]@{
    schema            = 'rustibites.bench.matrix/1'
    mode              = $Mode
    run_label         = $runLabel
    started_at_utc    = $startedUtc.ToString('o')
    scenario          = $scenarioPath
    scenario_label    = $scenarioLabel
    requested_pops    = $popList
    threads           = $threadList
    ticks_per_run     = $Ticks
    repeat_per_cell   = $Repeat
    timeout_sec_run   = $TimeoutSec
    budget_sec        = $BudgetSec
    min_free_gb       = $MinFreeGB
    max_load_pct      = $MaxLoadPct
    load_wait_sec     = $LoadWaitSec
    sample_ms         = $SampleMs
    determinism_check = [bool]$DeterminismCheck
    binary            = [ordered]@{
        original_path = $exeInfo.OriginalPath
        measured_copy = $exeInfo.RunPath
        sha256        = $exeInfo.Sha256
    }
    artifacts         = [ordered]@{
        runs_csv        = 'runs.csv'
        summary_csv     = 'summary.csv'
        markdown        = 'matrix.md'
        json            = 'matrix.json'
        environment     = 'environment.json'
        environment_txt = 'environment.txt'
    }
    notes             = @(
        'engine wall_secs covers only world.run(ticks); process_wall_secs adds setup/spawn/startup',
        'one operating-system process per repetition (--repeat 1) so repetitions are independent',
        'synthetic world area scales as 2000*sqrt(N/2000) to keep density constant',
        'state_fingerprint should be identical for the same repetition seed regardless of thread count'
    )
}
Write-BenchTextFile -Path (Join-Path $outDir 'config.json') -Text ($config | ConvertTo-Json -Depth 6)

# --- helpers ----------------------------------------------------------------
function Format-BenchOptional {
    param($Value, [string]$Format = '')
    if ($null -eq $Value) { return '' }
    if ($Format) { return (Format-BenchNumber $Value $Format) }
    return [string]$Value
}

function New-BenchRunRow {
    param(
        [string]$Status,
        $ExitCode,
        $Report,
        $ProcessResult,
        $PeakMB,
        [int]$RequestedPopulation,
        [int]$ThreadsRequested,
        [int]$Rep,
        [int]$TicksPlanned,
        $LoadBefore
    )
    $hasReport = ($null -ne $Report)
    $row = [ordered]@{
        mode                        = $Mode
        requested_population        = $RequestedPopulation
        population                  = if ($hasReport) { [int]$Report.bibites_start } else { $RequestedPopulation }
        threads_requested           = $ThreadsRequested
        threads_engine              = if ($hasReport) { [int]$Report.threads } else { $null }
        rep                         = $Rep
        status                      = $Status
        exit_code                   = $ExitCode
        label                       = if ($hasReport) { [string]$Report.label } else { $scenarioLabel }
        ticks                       = if ($hasReport) { [int]$Report.ticks } else { $TicksPlanned }
        bibites_start               = if ($hasReport) { [int]$Report.bibites_start } else { $null }
        bibites_end                 = if ($hasReport) { [int]$Report.bibites_end } else { $null }
        wall_secs                   = if ($hasReport) { [double]$Report.wall_secs } else { $null }
        sim_secs                    = if ($hasReport) { [double]$Report.sim_secs } else { $null }
        ticks_per_second            = if ($hasReport) { [double]$Report.ticks_per_second } else { $null }
        realtime_factor             = if ($hasReport) { [double]$Report.realtime_factor } else { $null }
        bibite_updates_per_second   = if ($hasReport) { [double]$Report.bibite_updates_per_second } else { $null }
        state_fingerprint           = if ($hasReport) { [string]$Report.state_fingerprint } else { $null }
        process_wall_secs           = if ($null -ne $ProcessResult) { [double]$ProcessResult.WallSecs } else { $null }
        peak_ws_mb                  = $PeakMB
        free_gb_before              = if ($null -ne $LoadBefore) { $LoadBefore.free_gb } else { $null }
        cpu_load_pct_before         = if ($null -ne $LoadBefore) { $LoadBefore.cpu_load_pct } else { $null }
        rustc_procs_before          = if ($null -ne $LoadBefore) { $LoadBefore.rustc_procs } else { $null }
        cargo_procs_before          = if ($null -ne $LoadBefore) { $LoadBefore.cargo_procs } else { $null }
        other_rustibites_before     = if ($null -ne $LoadBefore) { $LoadBefore.other_rustibites_procs } else { $null }
        free_gb_after               = $null
        cpu_load_pct_after          = $null
        rustc_procs_after           = $null
        cargo_procs_after           = $null
        other_rustibites_after      = $null
        error                       = $null
    }
    return $row
}

function Write-RunCsvFields {
    param($Row)
    return @(
        $Row.mode
        (Format-BenchOptional $Row.requested_population '0')
        (Format-BenchOptional $Row.population '0')
        (Format-BenchOptional $Row.threads_requested '0')
        (Format-BenchOptional $Row.threads_engine '0')
        (Format-BenchOptional $Row.rep '0')
        $Row.status
        (Format-BenchOptional $Row.exit_code '0')
        $Row.label
        (Format-BenchOptional $Row.ticks '0')
        (Format-BenchOptional $Row.bibites_start '0')
        (Format-BenchOptional $Row.bibites_end '0')
        (Format-BenchOptional $Row.wall_secs '0.000000')
        (Format-BenchOptional $Row.sim_secs '0.000000')
        (Format-BenchOptional $Row.ticks_per_second '0.000')
        (Format-BenchOptional $Row.realtime_factor '0.000')
        (Format-BenchOptional $Row.bibite_updates_per_second '0.0')
        (Format-BenchOptional $Row.state_fingerprint '')
        (Format-BenchOptional $Row.process_wall_secs '0.000000')
        (Format-BenchOptional $Row.peak_ws_mb '0.0')
        (Format-BenchOptional $Row.free_gb_before '0.00')
        (Format-BenchOptional $Row.cpu_load_pct_before '0.0')
        (Format-BenchOptional $Row.rustc_procs_before '0')
        (Format-BenchOptional $Row.cargo_procs_before '0')
        (Format-BenchOptional $Row.other_rustibites_before '0')
        (Format-BenchOptional $Row.free_gb_after '0.00')
        (Format-BenchOptional $Row.cpu_load_pct_after '0.0')
        (Format-BenchOptional $Row.rustc_procs_after '0')
        (Format-BenchOptional $Row.cargo_procs_after '0')
        (Format-BenchOptional $Row.other_rustibites_after '0')
        $Row.error
    )
}

$runCsvHeader = 'mode,requested_population,population,threads_requested,threads_engine,rep,status,exit_code,label,ticks,bibites_start,bibites_end,wall_secs,sim_secs,ticks_per_second,realtime_factor,bibite_updates_per_second,state_fingerprint,process_wall_secs,peak_ws_mb,free_gb_before,cpu_load_pct_before,rustc_procs_before,cargo_procs_before,other_rustibites_before,free_gb_after,cpu_load_pct_after,rustc_procs_after,cargo_procs_after,other_rustibites_after,error'

function Write-ProgressLine {
    param($Row, [int]$Index, [int]$Total)
    $prefix = '[{0}/{1}] pop={2} threads={3} rep={4}/{5}: {6}' -f $Index, $Total, $Row.requested_population, $Row.threads_requested, $Row.rep, $Repeat, $Row.status
    if ($Row.status -eq 'ok') {
        $line = $prefix +
            ' | ' + (Format-BenchNumber $Row.ticks_per_second 'N0') + ' ticks/s' +
            ' (' + (Format-BenchNumber $Row.realtime_factor '0.0') + 'x RT), ' +
            (Format-BenchNumber $Row.bibite_updates_per_second 'N0') + ' updates/s, engine ' +
            (Format-BenchNumber $Row.wall_secs '0.000') + 's, process ' +
            (Format-BenchNumber $Row.process_wall_secs '0.00') + 's, peak ' +
            (Format-BenchOptional $Row.peak_ws_mb '0.0') + ' MB'
    } else {
        $line = $prefix + ' | ' + $Row.error
    }
    if (-not $optionQuiet) { Write-Host $line }
}

# --- run the matrix ---------------------------------------------------------
$cells = New-Object System.Collections.Generic.List[object]
foreach ($population in $popList) {
    foreach ($threadCount in $threadList) {
        $cells.Add([pscustomobject]@{ requested_population = [int]$population; threads = [int]$threadCount })
    }
}

$rows = New-Object System.Collections.Generic.List[object]
$budgetExhausted = $false
$cellIndex = 0
foreach ($cell in $cells) {
    $cellIndex++
    $elapsedSec = ((Get-Date).ToUniversalTime() - $startedUtc).TotalSeconds
    if ($BudgetSec -gt 0 -and -not $budgetExhausted -and $elapsedSec -gt $BudgetSec) {
        $budgetExhausted = $true
        if (-not $optionQuiet) { Write-Warning ("wall-clock budget of {0}s exhausted after {1:N0}s; remaining cells will be marked skipped_budget" -f $BudgetSec, $elapsedSec) }
    }

    $loadBefore = Get-BenchLoadSnapshot
    if ($budgetExhausted) {
        $row = New-BenchRunRow -Status 'skipped_budget' -ExitCode $null -Report $null -ProcessResult $null -PeakMB $null -RequestedPopulation $cell.requested_population -ThreadsRequested $cell.threads -Rep 0 -TicksPlanned $Ticks -LoadBefore $loadBefore
        $row.error = 'wall-clock budget exhausted (-BudgetSec)'
        $rows.Add($row)
        if (-not $optionQuiet) { Write-Host ("[{0}/{1}] pop={2} threads={3}: skipped_budget" -f $cellIndex, $cells.Count, $cell.requested_population, $cell.threads) }
        continue
    }
    if ($MinFreeGB -gt 0 -and $null -ne $loadBefore.free_gb -and [double]$loadBefore.free_gb -lt $MinFreeGB) {
        $row = New-BenchRunRow -Status 'skipped_low_mem' -ExitCode $null -Report $null -ProcessResult $null -PeakMB $null -RequestedPopulation $cell.requested_population -ThreadsRequested $cell.threads -Rep 0 -TicksPlanned $Ticks -LoadBefore $loadBefore
        $row.error = 'free memory below -MinFreeGB'
        $rows.Add($row)
        if (-not $optionQuiet) { Write-Host ("[{0}/{1}] pop={2} threads={3}: skipped_low_mem ({4} GB free)" -f $cellIndex, $cells.Count, $cell.requested_population, $cell.threads, (Format-BenchOptional $loadBefore.free_gb '0.00')) }
        continue
    }

    # Optional idle-machine guard for shared hosts: wait for no builds, no other
    # benchmarks and CPU load below -MaxLoadPct before starting a cell.
    if ($MaxLoadPct -gt 0) {
        $waitDeadline = (Get-Date).AddSeconds($LoadWaitSec)
        $ready = $false
        while ($true) {
            $probe = Get-BenchLoadSnapshot
            $buildProcs = [int]$probe.rustc_procs + [int]$probe.cargo_procs
            $otherBenches = [int]$probe.other_rustibites_procs
            $loadOk = ($null -eq $probe.cpu_load_pct) -or ([double]$probe.cpu_load_pct -le $MaxLoadPct)
            if ($buildProcs -eq 0 -and $otherBenches -eq 0 -and $loadOk) { $ready = $true; $loadBefore = $probe; break }
            if ((Get-Date) -ge $waitDeadline) { break }
            if (-not $optionQuiet) {
                Write-Host ("waiting for idle machine: load={0}%, rustc+cargo={1}, other rustibites procs={2}" -f
                    (Format-BenchOptional $probe.cpu_load_pct '0.0'), $buildProcs, $otherBenches)
            }
            Start-Sleep -Seconds 3
        }
        if (-not $ready) {
            $row = New-BenchRunRow -Status 'skipped_load' -ExitCode $null -Report $null -ProcessResult $null -PeakMB $null -RequestedPopulation $cell.requested_population -ThreadsRequested $cell.threads -Rep 0 -TicksPlanned $Ticks -LoadBefore $loadBefore
            $row.error = "-MaxLoadPct $MaxLoadPct not reached within -LoadWaitSec $LoadWaitSec"
            $rows.Add($row)
            if (-not $optionQuiet) { Write-Host ("[{0}/{1}] pop={2} threads={3}: skipped_load" -f $cellIndex, $cells.Count, $cell.requested_population, $cell.threads) }
            continue
        }
    }

    $cellRows = New-Object System.Collections.Generic.List[object]
    for ($repetition = 1; $repetition -le $Repeat; $repetition++) {
        $arguments = @('bench')
        if ($scenarioPath) {
            $arguments += @('--scenario', $scenarioPath)
        } else {
            $arguments += @('--bibites', [string]$cell.requested_population)
        }
        $arguments += @('--ticks', [string]$Ticks, '--threads', [string]$cell.threads, '--repeat', '1', '--json')

        $processResult = Invoke-BenchProcess -FilePath $exeInfo.RunPath -ArgumentList $arguments -TimeoutSec $TimeoutSec -SampleMs $SampleMs
        $reports = @(ConvertFrom-BenchJsonLines $processResult.StdOut)
        $peakMB = if ($processResult.PeakWorkingSetBytes -gt 0) { [math]::Round([double]$processResult.PeakWorkingSetBytes / 1MB, 1) } else { $null }

        $status = 'ok'
        $errorText = $null
        $report = $null
        if ($processResult.TimedOut) {
            $status = 'timeout'
            $errorText = "exceeded -TimeoutSec $TimeoutSec"
        } elseif ($processResult.StdErr -match 'memory allocation of (\d+) bytes failed') {
            $status = 'oom'
            $errorText = "memory allocation of $($matches[1]) bytes failed"
        } elseif ($processResult.ExitCode -ne 0) {
            $status = 'error'
            $firstErr = @($processResult.StdErr -split "`r?`n" | Where-Object { $_.Trim().Length -gt 0 } | Select-Object -First 1)
            $errorText = if ($firstErr.Count -gt 0) { $firstErr[0].Trim() } else { "exit code $($processResult.ExitCode)" }
        } elseif ($reports.Count -eq 0) {
            $status = 'error'
            $errorText = 'no JSON report on stdout'
        } else {
            $report = $reports[$reports.Count - 1]
        }

        $row = New-BenchRunRow -Status $status -ExitCode $processResult.ExitCode -Report $report -ProcessResult $processResult -PeakMB $peakMB -RequestedPopulation $cell.requested_population -ThreadsRequested $cell.threads -Rep $repetition -TicksPlanned $Ticks -LoadBefore $loadBefore
        $row.error = $errorText
        $cellRows.Add($row)
        $rows.Add($row)
        Write-ProgressLine -Row $row -Index $cellIndex -Total $cells.Count
    }

    $loadAfter = Get-BenchLoadSnapshot
    foreach ($row in $cellRows) {
        $row.free_gb_after = $loadAfter.free_gb
        $row.cpu_load_pct_after = $loadAfter.cpu_load_pct
        $row.rustc_procs_after = $loadAfter.rustc_procs
        $row.cargo_procs_after = $loadAfter.cargo_procs
        $row.other_rustibites_after = $loadAfter.other_rustibites_procs
    }
}

# --- aggregate per cell -----------------------------------------------------
$cellSummaries = New-Object System.Collections.Generic.List[object]
foreach ($cell in $cells) {
    $cellRows = @($rows | Where-Object { $_.requested_population -eq $cell.requested_population -and $_.threads_requested -eq $cell.threads })
    $okRows = @($cellRows | Where-Object { $_.status -eq 'ok' })
    $summary = [ordered]@{
        requested_population = [int]$cell.requested_population
        population           = if ($okRows.Count -gt 0) { [int]$okRows[0].population } else { [int]$cell.requested_population }
        threads              = [int]$cell.threads
        reps_total           = $cellRows.Count
        reps_ok              = $okRows.Count
        status               = if ($okRows.Count -eq $cellRows.Count) { 'ok' } elseif ($okRows.Count -gt 0) { 'partial' } else { [string]$cellRows[$cellRows.Count - 1].status }
        best_wall_secs       = $null
        median_wall_secs     = $null
        best_ticks_per_second = $null
        median_ticks_per_second = $null
        best_realtime_factor = $null
        median_realtime_factor = $null
        best_bibite_updates_per_second = $null
        median_bibite_updates_per_second = $null
        peak_ws_mb           = $null
        state_fingerprint    = $null
        serial_ticks_per_second = $null
        speedup_vs_serial    = $null
        best_threads         = $null
        cpu_load_pct_min     = $null
        cpu_load_pct_max     = $null
        error                = $null
    }
    if ($okRows.Count -gt 0) {
        $bestRow = @($okRows | Sort-Object -Property wall_secs | Select-Object -First 1)[0]
        $summary.best_wall_secs = [double]$bestRow.wall_secs
        $summary.median_wall_secs = Get-BenchMedian @($okRows | ForEach-Object { [double]$_.wall_secs })
        $summary.best_ticks_per_second = [double]$bestRow.ticks_per_second
        $summary.median_ticks_per_second = Get-BenchMedian @($okRows | ForEach-Object { [double]$_.ticks_per_second })
        $summary.best_realtime_factor = [double]$bestRow.realtime_factor
        $summary.median_realtime_factor = Get-BenchMedian @($okRows | ForEach-Object { [double]$_.realtime_factor })
        $summary.best_bibite_updates_per_second = [double]$bestRow.bibite_updates_per_second
        $summary.median_bibite_updates_per_second = Get-BenchMedian @($okRows | ForEach-Object { [double]$_.bibite_updates_per_second })
        $peakValues = @($okRows | Where-Object { $null -ne $_.peak_ws_mb } | ForEach-Object { [double]$_.peak_ws_mb })
        if ($peakValues.Count -gt 0) { $summary.peak_ws_mb = ($peakValues | Measure-Object -Maximum).Maximum }
        $summary.state_fingerprint = [string]$okRows[0].state_fingerprint
        $loads = @()
        foreach ($okRow in $okRows) {
            if ($null -ne $okRow.cpu_load_pct_before) { $loads += [double]$okRow.cpu_load_pct_before }
            if ($null -ne $okRow.cpu_load_pct_after) { $loads += [double]$okRow.cpu_load_pct_after }
        }
        if ($loads.Count -gt 0) {
            $summary.cpu_load_pct_min = ($loads | Measure-Object -Minimum).Minimum
            $summary.cpu_load_pct_max = ($loads | Measure-Object -Maximum).Maximum
        }
    }
    $failedRows = @($cellRows | Where-Object { $_.status -ne 'ok' })
    if ($failedRows.Count -gt 0) {
        $summary.error = (@($failedRows | ForEach-Object { [string]$_.status }) -join ',')
    }
    $cellSummaries.Add([pscustomobject]$summary)
}

# Thread scaling relative to the serial cell of the same population.
foreach ($populationGroup in ($cellSummaries | Group-Object -Property requested_population)) {
    $groupCells = @($populationGroup.Group)
    $serialCell = @($groupCells | Where-Object { $_.threads -eq 1 -and $_.reps_ok -gt 0 } | Select-Object -First 1)
    $serialTps = if ($serialCell.Count -gt 0) { $serialCell[0].best_ticks_per_second } else { $null }
    $bestCell = @($groupCells | Where-Object { $_.reps_ok -gt 0 } | Sort-Object -Property best_ticks_per_second -Descending | Select-Object -First 1)
    $bestThreads = if ($bestCell.Count -gt 0) { $bestCell[0].threads } else { $null }
    foreach ($cell in $groupCells) {
        $cell.serial_ticks_per_second = $serialTps
        $cell.best_threads = $bestThreads
        if ($null -ne $serialTps -and $serialTps -gt 0 -and $null -ne $cell.best_ticks_per_second) {
            $cell.speedup_vs_serial = [double]$cell.best_ticks_per_second / [double]$serialTps
        }
    }
}

# Fingerprint consistency: same repetition seed (rep 1) across thread counts must
# produce bit-identical state; a mismatch means nondeterminism or a stale binary.
# The row objects are ordered dictionaries, so group with a script block:
# Group-Object -Property <name> cannot see hashtable keys as properties.
$fingerprintChecks = @()
foreach ($populationGroup in ($rows | Where-Object { $_.status -eq 'ok' -and $_.rep -eq 1 } | Group-Object -Property { $_.population })) {
    $hashes = @($populationGroup.Group | ForEach-Object { [string]$_.state_fingerprint } | Sort-Object -Unique)
    $fingerprintChecks += [pscustomobject]@{
        population          = [int]$populationGroup.Name
        unique_fingerprints = $hashes.Count
        consistent          = ($hashes.Count -le 1)
        fingerprints        = $hashes
    }
}

# Optional cross-process determinism check on the same seed.
$determinism = @()
if ($DeterminismCheck) {
    $checkPops = @($popList | Where-Object { $_ -gt 0 } | Select-Object -First 2)
    foreach ($population in $checkPops) {
        $fingerprints = @()
        for ($attempt = 1; $attempt -le 2; $attempt++) {
            $arguments = @('bench', '--bibites', [string]$population, '--ticks', [string]$Ticks, '--threads', '1', '--repeat', '1', '--json')
            $result = Invoke-BenchProcess -FilePath $exeInfo.RunPath -ArgumentList $arguments -TimeoutSec $TimeoutSec -SampleMs $SampleMs
            $reports = @(ConvertFrom-BenchJsonLines $result.StdOut)
            if ($reports.Count -gt 0) { $fingerprints += [string]$reports[0].state_fingerprint } else { $fingerprints += 'ERROR' }
        }
        $matrixRow = @($rows | Where-Object { $_.status -eq 'ok' -and $_.rep -eq 1 -and $_.requested_population -eq $population -and $_.threads_requested -eq 1 } | Select-Object -First 1)
        if ($matrixRow.Count -gt 0) { $fingerprints += [string]$matrixRow[0].state_fingerprint }
        $unique = @($fingerprints | Sort-Object -Unique)
        $determinism += [pscustomobject]@{
            population   = [int]$population
            fingerprints = $fingerprints
            identical    = ($unique.Count -le 1)
        }
        if (-not $optionQuiet) {
            Write-Host ("determinism: pop={0} identical={1} fingerprints={2}" -f $population, ($unique.Count -le 1), ($fingerprints -join ','))
        }
    }
    Write-BenchTextFile -Path (Join-Path $outDir 'determinism.json') -Text ($determinism | ConvertTo-Json -Depth 5)
}

# --- write CSV artifacts ----------------------------------------------------
$runLines = New-Object System.Collections.Generic.List[string]
$runLines.Add($runCsvHeader)
foreach ($row in $rows) { $runLines.Add((ConvertTo-BenchCsvLine (Write-RunCsvFields $row))) }
Write-BenchTextFile -Path (Join-Path $outDir 'runs.csv') -Text (($runLines -join "`r`n") + "`r`n")

$summaryHeader = 'requested_population,population,threads,reps_ok,reps_total,status,best_wall_secs,median_wall_secs,best_ticks_per_second,median_ticks_per_second,best_realtime_factor,median_realtime_factor,best_bibite_updates_per_second,median_bibite_updates_per_second,serial_ticks_per_second,speedup_vs_serial,best_threads,peak_ws_mb,state_fingerprint,cpu_load_pct_min,cpu_load_pct_max,error'
$summaryLines = New-Object System.Collections.Generic.List[string]
$summaryLines.Add($summaryHeader)
foreach ($cell in $cellSummaries) {
    $summaryLines.Add((ConvertTo-BenchCsvLine @(
        (Format-BenchOptional $cell.requested_population '0')
        (Format-BenchOptional $cell.population '0')
        (Format-BenchOptional $cell.threads '0')
        (Format-BenchOptional $cell.reps_ok '0')
        (Format-BenchOptional $cell.reps_total '0')
        $cell.status
        (Format-BenchOptional $cell.best_wall_secs '0.000000')
        (Format-BenchOptional $cell.median_wall_secs '0.000000')
        (Format-BenchOptional $cell.best_ticks_per_second '0.000')
        (Format-BenchOptional $cell.median_ticks_per_second '0.000')
        (Format-BenchOptional $cell.best_realtime_factor '0.000')
        (Format-BenchOptional $cell.median_realtime_factor '0.000')
        (Format-BenchOptional $cell.best_bibite_updates_per_second '0.0')
        (Format-BenchOptional $cell.median_bibite_updates_per_second '0.0')
        (Format-BenchOptional $cell.serial_ticks_per_second '0.000')
        (Format-BenchOptional $cell.speedup_vs_serial '0.000')
        (Format-BenchOptional $cell.best_threads '0')
        (Format-BenchOptional $cell.peak_ws_mb '0.0')
        (Format-BenchOptional $cell.state_fingerprint '')
        (Format-BenchOptional $cell.cpu_load_pct_min '0.0')
        (Format-BenchOptional $cell.cpu_load_pct_max '0.0')
        $cell.error
    )))
}
Write-BenchTextFile -Path (Join-Path $outDir 'summary.csv') -Text (($summaryLines -join "`r`n") + "`r`n")

# --- Markdown tables --------------------------------------------------------
function Get-MatrixTableLines {
    param($Cells)
    $threadValues = @($Cells | ForEach-Object { $_.threads } | Sort-Object -Unique)
    $populationValues = @($Cells | ForEach-Object { $_.population } | Sort-Object -Unique)
    $cellLookup = @{}
    foreach ($cell in $Cells) { $cellLookup[('{0}|{1}' -f $cell.population, $cell.threads)] = $cell }

    $lines = New-Object System.Collections.Generic.List[string]

    $headerCells = @('Population')
    foreach ($threadValue in $threadValues) {
        if ($threadValue -eq 1) { $headerCells += '1 thread' } else { $headerCells += "$threadValue threads" }
    }
    $lines.Add('| ' + ($headerCells -join ' | ') + ' |')
    $lines.Add('| ' + ((1..$headerCells.Count | ForEach-Object { '---' }) -join ' | ') + ' |')
    foreach ($population in $populationValues) {
        $cellsText = @()
        foreach ($threadValue in $threadValues) {
            $cell = $cellLookup[('{0}|{1}' -f $population, $threadValue)]
            if ($null -eq $cell) { $cellsText += '-'; continue }
            if ($cell.reps_ok -le 0) { $cellsText += ('**' + $cell.status + '**'); continue }
            $cellsText += ((Format-BenchNumber $cell.best_ticks_per_second 'N0') + ' (' + (Format-BenchNumber $cell.best_realtime_factor '0.0') + 'x)')
        }
        $lines.Add('| ' + (Format-BenchNumber $population 'N0') + ' | ' + ($cellsText -join ' | ') + ' |')
    }

    $lines.Add('')
    $lines.Add('Cells show **ticks/s (real-time factor)** at 30 simulated TPS on the benchmark world (synthetic density-scaled unless `-Scenario` was used); best of the repetitions. `-` = cell not run.')
    $lines.Add('')
    $lines.Add('| Population | Serial ticks/s | Best ticks/s | Best @ threads | Speedup vs serial | Serial updates/s | Peak WS (MB) |')
    $lines.Add('| --- | --- | --- | --- | --- | --- | --- |')
    foreach ($population in $populationValues) {
        $populationCells = @($Cells | Where-Object { $_.population -eq $population -and $_.reps_ok -gt 0 })
        if ($populationCells.Count -eq 0) { continue }
        $serialCell = @($populationCells | Where-Object { $_.threads -eq 1 } | Select-Object -First 1)
        $bestCell = @($populationCells | Sort-Object -Property best_ticks_per_second -Descending | Select-Object -First 1)[0]
        $serialTpsText = if ($serialCell.Count -gt 0) { Format-BenchNumber $serialCell[0].best_ticks_per_second 'N0' } else { '-' }
        $speedupText = if ($serialCell.Count -gt 0 -and $serialCell[0].best_ticks_per_second -gt 0) { Format-BenchNumber ($bestCell.best_ticks_per_second / $serialCell[0].best_ticks_per_second) '0.00' } else { '-' }
        $serialUpdatesText = if ($serialCell.Count -gt 0) { Format-BenchNumber $serialCell[0].best_bibite_updates_per_second 'N0' } else { '-' }
        $peaks = @($populationCells | Where-Object { $null -ne $_.peak_ws_mb } | ForEach-Object { [double]$_.peak_ws_mb })
        $peakText = if ($peaks.Count -gt 0) { Format-BenchNumber (($peaks | Measure-Object -Maximum).Maximum) '0.0' } else { '-' }
        $lines.Add('| ' + (Format-BenchNumber $population 'N0') + ' | ' + $serialTpsText + ' | ' + (Format-BenchNumber $bestCell.best_ticks_per_second 'N0') + ' | ' + $bestCell.best_threads + ' | ' + $speedupText + ' | ' + $serialUpdatesText + ' | ' + $peakText + ' |')
    }

    $lines.Add('')
    $lines.Add('| Population | Threads | Reps ok/total | Best wall (s) | Median wall (s) | Best ticks/s | Median ticks/s | Best xRT | Updates/s (best) | Peak WS (MB) | Status |')
    $lines.Add('| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |')
    foreach ($population in $populationValues) {
        foreach ($threadValue in $threadValues) {
            $cell = $cellLookup[('{0}|{1}' -f $population, $threadValue)]
            if ($null -eq $cell) { continue }
            $lines.Add('| ' + (Format-BenchNumber $population 'N0') + ' | ' + $threadValue + ' | ' + $cell.reps_ok + '/' + $cell.reps_total + ' | ' +
                (Format-BenchOptional $cell.best_wall_secs '0.000') + ' | ' +
                (Format-BenchOptional $cell.median_wall_secs '0.000') + ' | ' +
                (Format-BenchOptional $cell.best_ticks_per_second 'N0') + ' | ' +
                (Format-BenchOptional $cell.median_ticks_per_second 'N0') + ' | ' +
                (Format-BenchOptional $cell.best_realtime_factor '0.0') + ' | ' +
                (Format-BenchOptional $cell.best_bibite_updates_per_second 'N0') + ' | ' +
                (Format-BenchOptional $cell.peak_ws_mb '0.0') + ' | ' +
                $cell.status + ' |')
        }
    }
    return $lines.ToArray()
}

function Get-EnvironmentSummaryLines {
    param($Environment, $Config)
    $lines = New-Object System.Collections.Generic.List[string]
    $executable = if ($null -ne $Environment.binary) { $Environment.binary } else { $null }
    $lines.Add('- **Host:** ' + $Environment.hostname + ' - ' + $Environment.os.description + ' (' + $Environment.os.version + ')')
    $lines.Add('- **CPU:** ' + $Environment.cpu.name + ' - ' + $Environment.cpu.physical_cores + ' physical / ' + $Environment.cpu.logical_cores + ' logical cores')
    $lines.Add('- **Memory:** ' + (Format-BenchNumber $Environment.memory_total_gb '0.0') + ' GB total, ' + (Format-BenchNumber $Environment.memory_free_gb '0.00') + ' GB free at capture')
    $lines.Add('- **Rust:** rustc ' + $Environment.rust.rustc + ' (release ' + $Environment.rust.release + ', ' + $Environment.rust.host + '), cargo ' + $Environment.rust.cargo)
    $lines.Add('- **Git:** ' + $Environment.git.head_short + ' on ' + $Environment.git.branch + ' (' + $Environment.git.commit_date + '), dirty = ' + $Environment.git.dirty + ' (' + $Environment.git.dirty_files + ' paths)')
    $lines.Add('- **Release profile:** ' + $Environment.release_profile.raw.Replace("`n", '; '))
    if ($null -ne $executable) {
        $lines.Add('- **Measured binary:** `' + $executable.sha256 + '` (' + $executable.size_bytes + ' bytes, built ' + $executable.last_write_utc + ')')
    }
    $lines.Add('- **Captured (UTC):** ' + $Environment.captured_at_utc)
    if ($null -ne $Config) {
        $lines.Add('- **Matrix:** mode=`' + $Config.mode + '`, ticks=' + $Config.ticks_per_run + ', repeat=' + $Config.repeat_per_cell + ', scenario=`' + $Config.scenario_label + '`')
    }
    return $lines.ToArray()
}

$generatedAtUtc = (Get-Date).ToUniversalTime()
$tableLines = @(Get-MatrixTableLines -Cells $cellSummaries)
$environmentLines = @(Get-EnvironmentSummaryLines -Environment $environment -Config ([pscustomobject]$config))

$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add('# RUSTibites benchmark matrix - `' + $Mode + '` (' + $runLabel + ')')
$markdown.Add('')
$markdown.Add('_Auto-generated by `tools/bench/run_matrix.ps1` at ' + $generatedAtUtc.ToString('o') + '._')
$markdown.Add('')
if ($scenarioPath) {
    $markdown.Add('Scenario: `' + $scenarioLabel + '` (population comes from the scenario) - threads: ' + ($threadList -join ', ') + ' - ticks per run: ' + $Ticks + ' - repetitions per cell: ' + $Repeat + '.')
} else {
    $markdown.Add('Populations: ' + (($popList | ForEach-Object { (Format-BenchNumber $_ 'N0') }) -join ', ') + ' - threads: ' + ($threadList -join ', ') + ' - ticks per run: ' + $Ticks + ' - repetitions per cell: ' + $Repeat + ' - scenario: `' + $scenarioLabel + '`.')
}
$markdown.Add('')
$markdown.Add('## Environment')
$markdown.Add('')
foreach ($environmentLine in $environmentLines) { $markdown.Add($environmentLine) }
$markdown.Add('')
$markdown.Add('## Throughput (best of ' + $Repeat + ')')
$markdown.Add('')
foreach ($tableLine in $tableLines) { $markdown.Add($tableLine) }
$markdown.Add('')
if ($fingerprintChecks.Count -gt 0) {
    $consistent = @($fingerprintChecks | Where-Object { -not $_.consistent })
    if ($consistent.Count -eq 0) {
        $markdown.Add('## Determinism')
        $markdown.Add('')
        $markdown.Add('`state_fingerprint` is identical for the same repetition seed across all thread counts (' + $fingerprintChecks.Count + ' populations checked).')
    } else {
        $markdown.Add('## Determinism - MISMATCH')
        $markdown.Add('')
        foreach ($check in $consistent) {
            $markdown.Add('- population ' + $check.population + ': ' + ($check.fingerprints -join ', '))
        }
    }
    $markdown.Add('')
}
if ($determinism.Count -gt 0) {
    $markdown.Add('## Cross-process determinism (`-DeterminismCheck`)')
    $markdown.Add('')
    $markdown.Add('| Population | Identical | Fingerprints |')
    $markdown.Add('| --- | --- | --- |')
    foreach ($check in $determinism) {
        $markdown.Add('| ' + $check.population + ' | ' + $check.identical + ' | ' + ($check.fingerprints -join ', ') + ' |')
    }
    $markdown.Add('')
}
$markdown.Add('## Notes')
$markdown.Add('')
$markdown.Add('- `ticks/s`, `real-time factor` and `bibite-updates/s` come from the engine report (`bench --json`); `wall_secs` covers only `world.run(ticks)`, not setup/spawn.')
$markdown.Add('- `process_wall_secs` and `peak_ws_mb` are measured by this harness for the whole process (peak working set sampled every ' + $SampleMs + ' ms; `bench` reports `peak_bytes = null`).')
$markdown.Add('- Synthetic worlds scale the world area as `2000*sqrt(N/2000)` so every population is benchmarked at the same density as the 2,000-bibite reference world.')
$markdown.Add('- Measurements are valid only on an otherwise idle machine; see `runs.csv` (`cpu_load_pct_before/after`, `rustc_procs_before/after`) for concurrent load during this run.')
$markdown.Add('')
$markdownText = ($markdown -join "`r`n") + "`r`n"
Write-BenchTextFile -Path (Join-Path $outDir 'matrix.md') -Text $markdownText

$matrixJson = [ordered]@{
    schema           = 'rustibites.bench.matrix/1'
    generated_at_utc = $generatedAtUtc.ToString('o')
    config           = $config
    cells            = $cellSummaries
    fingerprint_checks = $fingerprintChecks
    determinism      = $determinism
    environment_binary = if ($null -ne $environment.binary) { $environment.binary } else { $null }
}
Write-BenchTextFile -Path (Join-Path $outDir 'matrix.json') -Text ($matrixJson | ConvertTo-Json -Depth 8)

# --- optional docs update ---------------------------------------------------
if ($optionUpdateDocs) {
    $docsFile = if ($optionDocsPath) {
        if ([System.IO.Path]::IsPathRooted($optionDocsPath)) { $optionDocsPath } else { Join-Path (Get-Location).Path $DocsPath }
    } else {
        Join-Path (Join-Path $repo 'docs') 'BENCHMARKS.md'
    }
    if (-not (Test-Path -LiteralPath $docsFile)) {
        throw "-UpdateDocs requested but $docsFile does not exist"
    }
    $docsText = [System.IO.File]::ReadAllText($docsFile)
    $pattern = '(?s)<!-- BEGIN BENCH:MATRIX -->.*?<!-- END BENCH:MATRIX -->'
    if ($docsText -notmatch $pattern) {
        throw "-UpdateDocs: markers '<!-- BEGIN BENCH:MATRIX -->' / '<!-- END BENCH:MATRIX -->' not found in $docsFile"
    }
    $autoLines = New-Object System.Collections.Generic.List[string]
    $autoLines.Add('<!-- BEGIN BENCH:MATRIX -->')
    $autoLines.Add('_Auto-generated by `tools/bench/run_matrix.ps1` on ' + $generatedAtUtc.ToString('o') + ' (`' + $Mode + '` mode, ticks=' + $Ticks + ', repeat=' + $Repeat + '). Do not edit by hand: rerun the script with `-UpdateDocs`._')
    $autoLines.Add('')
    foreach ($environmentLine in $environmentLines) { $autoLines.Add($environmentLine) }
    $autoLines.Add('')
    foreach ($tableLine in $tableLines) { $autoLines.Add($tableLine) }
    $autoLines.Add('<!-- END BENCH:MATRIX -->')
    $replacement = ($autoLines -join "`r`n")
    $docsText = [regex]::Replace($docsText, $pattern, { param($match) $replacement })
    Write-BenchTextFile -Path $docsFile -Text $docsText
    if (-not $optionQuiet) { Write-Host "updated matrix block in $docsFile" }
}

# --- console summary --------------------------------------------------------
if (-not $optionQuiet) {
    Write-Host ''
    Write-Host '--- summary (best of repetitions) ---'
    $cellSummaries | Format-Table requested_population, population, threads, reps_ok, status, best_ticks_per_second, best_realtime_factor, best_bibite_updates_per_second, speedup_vs_serial, peak_ws_mb -AutoSize
    Write-Host ''
    Write-Host "artifacts: $outDir"
    Write-Host '  runs.csv, summary.csv, matrix.md, matrix.json, config.json, environment.json, environment.txt'
    $failed = @($cellSummaries | Where-Object { $_.status -ne 'ok' })
    if ($failed.Count -gt 0) {
        Write-Warning ("{0} cell(s) did not complete cleanly: {1}" -f $failed.Count, (($failed | ForEach-Object { 'pop=' + $_.requested_population + '/threads=' + $_.threads + ':' + $_.status }) -join ', '))
    }
}

$okCells = @($cellSummaries | Where-Object { $_.reps_ok -gt 0 })
if ($okCells.Count -eq 0) { exit 2 }
exit 0
