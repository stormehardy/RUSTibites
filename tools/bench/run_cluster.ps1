<#
.SYNOPSIS
    Dense-cluster benchmark mode for RUSTibites (deterministic pile-up world).

.DESCRIPTION
    The normal matrix benchmarks a synthetic spread world. The physics
    broadphase/narrowphase falls off a cliff when many bibites are packed into a
    small area: the core audit `body::perf_probe_clustered_and_spread` shows
    spread2000 ~1.75 ms vs cluster2000 ~393 ms of `BibiteStore::step_physics`
    (cluster4000 ~1.5 s). This script measures that cliff **end-to-end** through
    `rustibites bench` on a deterministic clustered save world, so the whole-tick
    cost (physics + body + hash-sync + ...) is tracked too.

    World generation: `make_cluster_world.py` builds a minimal one-zone scenario
    from tracked files under tools/bench/scenarios/cluster/ (no testdata/ or game
    install needed), runs it once with a fixed seed to get one deterministic
    starter bibite, then clones that bibite N times onto a deterministic
    jittered lattice inside a `box x box` square. The save's content SHA-256 is
    recorded in config/environment artifacts; worlds are cached under
    target/bench-worlds/cluster/ (gitignored) and regenerated when the engine
    binary, scenario, seed, N or box changes.

    For each (population, box) it also runs the synthetic spread population as a
    control row, so the cluster/spread physics ratio is directly visible.
    `-CoreProbe` additionally runs the fast physics-only core probe and records
    its numbers (this is the canonical metric for the task-82 target
    "cluster2000 physics < 50 ms").

    Requires Python 3.8+ for world generation only (standard library, no
    packages). The benchmark runs themselves are pure PowerShell + the CLI.

.EXAMPLE
    # 2,000 bibites in 100x100 and 200x200 boxes + spread control, 10 ticks.
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_cluster.ps1

.EXAMPLE
    # Larger pile, serial and 8 threads, plus the canonical core probe.
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_cluster.ps1 `
        -Pops 2000,4000 -BoxSizes 100 -Threads 1,8 -Ticks 10 -CoreProbe

.NOTES
    Read-only with respect to crates/ and root Cargo.toml. Generated worlds live
    under target/ (not committed). The core probe compiles a release test binary
    on first use (~1 min).
#>
[CmdletBinding()]
param(
    [string[]]$Pops = @('2000'),
    [string[]]$BoxSizes = @('100,200'),
    [string[]]$Threads = @('1'),
    [int]$Ticks = 10,
    [int]$Repeat = 1,
    [int]$Seed = 12345,
    [string]$Exe = '',
    [switch]$Build,
    [string]$OutputDir = '',
    [int]$TimeoutSec = 600,
    [switch]$CoreProbe,
    [switch]$NoSpread,
    [switch]$ForceWorlds,
    [switch]$DeterminismCheck,
    [int]$SampleMs = 50,
    [double]$MaxLoadPct = 0,
    [int]$LoadWaitSec = 60,
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'bench_common.ps1')

$repo = Get-BenchRepo -ScriptRoot $PSScriptRoot
$exeInfo = Resolve-BenchExecutable -Repo $repo -Exe $Exe -Build:$Build
$resultsRoot = Join-Path (Join-Path (Join-Path $repo 'tools') 'bench') 'results'
$outDir = if ($OutputDir) {
    if ([System.IO.Path]::IsPathRooted($OutputDir)) { $OutputDir } else { Join-Path (Get-Location).Path $OutputDir }
} else {
    Join-Path $resultsRoot 'cluster'
}
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$worldRoot = Join-Path (Join-Path (Join-Path $repo 'target') 'bench-worlds') 'cluster'

# --- inputs ----------------------------------------------------------------
$popList = @((ConvertTo-BenchIntList -Values $Pops) | Where-Object { $_ -ge 1 } | Sort-Object -Unique)
if ($popList.Count -eq 0) { throw 'no populations given (-Pops)' }
$threadList = @((ConvertTo-BenchIntList -Values $Threads) | Where-Object { $_ -ge 1 } | Sort-Object -Unique)
if ($threadList.Count -eq 0) { throw 'no thread counts given (-Threads)' }
$boxList = @()
foreach ($value in $BoxSizes) {
    foreach ($part in ([string]$value -split '[,; ]+')) {
        $trimmed = $part.Trim()
        if ($trimmed.Length -eq 0) { continue }
        $parsedBox = 0.0
        if (-not [double]::TryParse($trimmed, [ref]$parsedBox)) { throw "invalid box size '$trimmed' (expected e.g. 100,200)" }
        if ($parsedBox -le 0) { throw "box size must be > 0 (got $trimmed)" }
        $boxList += $parsedBox
    }
}
$boxList = @($boxList | Sort-Object -Unique)
if ($boxList.Count -eq 0) { throw 'no box sizes given (-BoxSizes)' }
if ($Ticks -lt 1) { throw '-Ticks must be >= 1' }
if ($Repeat -lt 1) { throw '-Repeat must be >= 1' }

# --- environment + config ---------------------------------------------------
$environment = Get-BenchEnvironment -Repo $repo -Executable $exeInfo
Write-BenchEnvironment -Environment $environment -OutputDir $outDir
$startedUtc = (Get-Date).ToUniversalTime()
$config = [ordered]@{
    schema          = 'rustibites.bench.cluster/1'
    started_at_utc  = $startedUtc.ToString('o')
    populations     = $popList
    box_sizes       = $boxList
    threads         = $threadList
    ticks_per_run   = $Ticks
    repeat_per_cell = $Repeat
    seed            = $Seed
    spread_control  = (-not $NoSpread)
    core_probe      = [bool]$CoreProbe
    binary          = [ordered]@{ path = $exeInfo.RunPath; original_path = $exeInfo.OriginalPath; sha256 = $exeInfo.Sha256 }
    world_dir       = $worldRoot
    artifacts       = [ordered]@{
        runs_csv        = 'cluster.csv'
        summary_csv     = 'cluster_summary.csv'
        core_probe_csv  = 'core_probe.csv'
        markdown        = 'cluster.md'
        json            = 'cluster.json'
        environment     = 'environment.json'
        environment_txt = 'environment.txt'
    }
    notes           = @(
        'cluster worlds: deterministic save zips (N bibites on a jittered lattice inside box x box) generated by make_cluster_world.py',
        'world content_sha256 identifies the exact world; worlds cache under target/bench-worlds/cluster',
        'spread rows use the normal synthetic density-scaled world (bench --bibites N)',
        'core probe rows are physics-only step_physics measurements with the starter genome (canonical for the dense-cluster optimisation target)',
        'phase values are means over all measured ticks; ms_per_tick = 1000 / ticks_per_second'
    )
}
Write-BenchTextFile -Path (Join-Path $outDir 'config.json') -Text ($config | ConvertTo-Json -Depth 6)

function Format-BenchOptional {
    param($Value, [string]$Format = '')
    if ($null -eq $Value) { return '' }
    if ($Format) { return (Format-BenchNumber $Value $Format) }
    return [string]$Value
}

function Get-LoadGuardReady {
    param($LoadSnapshot, [double]$MaxPct)
    if ($MaxPct -le 0) { return $true }
    $buildProcs = [int]$LoadSnapshot.rustc_procs + [int]$LoadSnapshot.cargo_procs
    $otherBench = [int]$LoadSnapshot.other_rustibites_procs
    $loadOk = ($null -eq $LoadSnapshot.cpu_load_pct) -or ([double]$LoadSnapshot.cpu_load_pct -le $MaxPct)
    return ($buildProcs -eq 0 -and $otherBench -eq 0 -and $loadOk)
}

# --- world generation -------------------------------------------------------
$python = Get-Command python -ErrorAction SilentlyContinue
$generator = Join-Path $PSScriptRoot 'make_cluster_world.py'
if (-not $python) {
    throw 'Python 3.8+ (python on PATH) is required to generate cluster worlds. Install Python or pre-generate worlds with tools/bench/make_cluster_world.py.'
}
if (-not (Test-Path -LiteralPath $generator)) { throw "missing world generator: $generator" }

$worlds = @{}  # key "n|box" -> generator result
function Get-ClusterWorld {
    param([int]$Population, [double]$Box)
    $key = ('{0}|{1}' -f $Population, (Format-BenchNumber $Box '0.####'))
    if ($worlds.ContainsKey($key)) { return $worlds[$key] }
    $arguments = @(
        $generator,
        '--n', [string]$Population,
        '--box', (Format-BenchNumber $Box '0.####'),
        '--seed', [string]$Seed,
        '--exe', $exeInfo.RunPath,
        '--world-dir', $worldRoot,
        '--json'
    )
    if ($ForceWorlds) { $arguments += '--force' }
    if (-not $Quiet) { Write-Host ('generating cluster world: n={0} box={1}' -f $Population, (Format-BenchNumber $Box '0.####')) }
    $result = Invoke-BenchProcess -FilePath $python.Source -ArgumentList $arguments -TimeoutSec 300 -SampleMs 200
    if ($result.TimedOut) { throw "cluster world generation timed out (n=$Population box=$Box)" }
    if ($result.ExitCode -ne 0) {
        throw ("cluster world generation failed (exit {0}): {1}" -f $result.ExitCode, ($result.StdErr.Trim() + ' ' + $result.StdOut.Trim()))
    }
    $jsonLine = @(($result.StdOut -split "`r?`n") | Where-Object { $_ -match '^\s*\{' } | Select-Object -Last 1)
    if ($jsonLine.Count -eq 0) { throw "cluster world generator did not print JSON: $($result.StdOut)" }
    $world = $jsonLine[0] | ConvertFrom-Json
    $worlds[$key] = $world
    if (-not $Quiet) {
        Write-Host ('  world: {0} ({1:N1} MB) content_sha256={2} reused={3}' -f
            $world.world_path, ([double]$world.bytes / 1MB), $world.content_sha256.Substring(0, 16), $world.reused)
    }
    return $world
}

# --- run one bench process and turn it into a row ---------------------------
function Invoke-ClusterCell {
    param(
        [string]$Kind,
        [int]$Population,
        [double]$Box,
        [int]$ThreadsRequested,
        [int]$Rep,
        $World
    )
    $arguments = @('bench')
    if ($Kind -eq 'cluster') {
        $arguments += @('--scenario', $World.world_path)
    } else {
        $arguments += @('--bibites', [string]$Population)
    }
    $arguments += @('--ticks', [string]$Ticks, '--threads', [string]$ThreadsRequested, '--repeat', '1')

    $processStart = Get-Date
    $processResult = Invoke-BenchProcess -FilePath $exeInfo.RunPath -ArgumentList $arguments -TimeoutSec $TimeoutSec -SampleMs $SampleMs
    $parsed = @(Parse-BenchPrettyOutput $processResult.StdOut)
    $run = if ($parsed.Count -gt 0) { $parsed[$parsed.Count - 1] } else { $null }
    $peakMB = if ($processResult.PeakWorkingSetBytes -gt 0) { [math]::Round([double]$processResult.PeakWorkingSetBytes / 1MB, 1) } else { $null }

    $status = 'ok'
    $errorText = $null
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
    } elseif ($null -eq $run) {
        $status = 'error'
        $errorText = 'could not parse bench output'
    }

    $tps = if ($null -ne $run) { [double]$run.ticks_per_second } else { $null }
    $row = [ordered]@{
        kind                  = $Kind
        population            = $Population
        box                   = if ($Kind -eq 'cluster') { $Box } else { $null }
        threads               = $ThreadsRequested
        rep                   = $Rep
        status                = $status
        ticks                 = if ($null -ne $run) { [int]$run.ticks } else { $Ticks }
        bibites               = if ($null -ne $run) { [int]$run.bibites } else { $null }
        wall_secs             = if ($null -ne $run) { [double]$run.wall_secs } else { $null }
        ticks_per_second      = $tps
        realtime_factor       = if ($null -ne $run) { [double]$run.realtime_factor } else { $null }
        bibite_updates_per_second = if ($null -ne $run) { [double]$run.bibite_updates_per_second } else { $null }
        ms_per_tick           = if ($null -ne $tps -and $tps -gt 0) { 1000.0 / $tps } else { $null }
        physics_ms            = if ($null -ne $run -and $run.phases.Contains('physics')) { [double]$run.phases['physics'].ms } else { $null }
        body_ms               = if ($null -ne $run -and $run.phases.Contains('body')) { [double]$run.phases['body'].ms } else { $null }
        sensing_ms            = if ($null -ne $run -and $run.phases.Contains('sensing')) { [double]$run.phases['sensing'].ms } else { $null }
        hash_sync_ms          = if ($null -ne $run -and $run.phases.Contains('hash-sync')) { [double]$run.phases['hash-sync'].ms } else { $null }
        deaths_ms             = if ($null -ne $run -and $run.phases.Contains('deaths')) { [double]$run.phases['deaths'].ms } else { $null }
        total_phase_ms        = if ($null -ne $run) { [double]$run.total_phase_ms } else { $null }
        process_wall_secs     = [double]$processResult.WallSecs
        peak_ws_mb            = $peakMB
        world_content_sha256  = if ($Kind -eq 'cluster') { [string]$World.content_sha256 } else { $null }
        error                 = $errorText
    }
    return $row
}

function Write-RowProgress {
    param($Row, [int]$Index, [int]$Total)
    $label = '[{0}/{1}] {2} pop={3}' -f $Index, $Total, $Row.kind, $Row.population
    if ($null -ne $Row.box) { $label += (' box={0}' -f (Format-BenchOptional $Row.box '0.####')) }
    $label += (' threads={0} rep={1}: {2}' -f $Row.threads, $Row.rep, $Row.status)
    if ($Row.status -eq 'ok') {
        $label += ' | ' + (Format-BenchNumber $Row.ticks_per_second 'N1') + ' ticks/s, ' +
            (Format-BenchNumber $Row.ms_per_tick '0.00') + ' ms/tick, physics ' +
            (Format-BenchOptional $Row.physics_ms '0.00') + ' ms, body ' +
            (Format-BenchOptional $Row.body_ms '0.00') + ' ms, peak ' +
            (Format-BenchOptional $Row.peak_ws_mb '0.0') + ' MB'
    } else {
        $label += ' | ' + $Row.error
    }
    if (-not $Quiet) { Write-Host $label }
}

# --- run the matrix ---------------------------------------------------------
function Wait-ForIdleWindow {
    param([double]$MaxPct, [int]$WaitSec)
    if ($MaxPct -le 0) { return $true }
    $deadline = (Get-Date).AddSeconds($WaitSec)
    while ($true) {
        $probe = Get-BenchLoadSnapshot
        if (Get-LoadGuardReady -LoadSnapshot $probe -MaxPct $MaxPct) { return $true }
        if ((Get-Date) -ge $deadline) { return $false }
        if (-not $Quiet) {
            Write-Host ('waiting for idle machine: load={0}%, rustc+cargo={1}, other rustibites={2}' -f
                (Format-BenchOptional $probe.cpu_load_pct '0.0'),
                ([int]$probe.rustc_procs + [int]$probe.cargo_procs),
                [int]$probe.other_rustibites_procs)
        }
        Start-Sleep -Seconds 3
    }
}

function Add-SkippedLoadRow {
    param([string]$Kind, [int]$Population, $Box, [int]$ThreadsRequested, [int]$Rep, [int]$Index, [int]$Total)
    $row = [ordered]@{
        kind = $Kind
        population = $Population
        box = if ($Kind -eq 'cluster') { $Box } else { $null }
        threads = $ThreadsRequested
        rep = $Rep
        status = 'skipped_load'
        error = "-MaxLoadPct $MaxLoadPct not reached within -LoadWaitSec $LoadWaitSec"
    }
    $script:rows.Add($row)
    Write-RowProgress -Row ([pscustomobject]$row) -Index $Index -Total $Total
}

$rows = New-Object System.Collections.Generic.List[object]
$spreadCells = if ($NoSpread) { 0 } else { $popList.Count * $threadList.Count * $Repeat }
$cellCount = ($popList.Count * $boxList.Count * $threadList.Count * $Repeat) + $spreadCells
$cellIndex = 0
$startedWorlds = @{}
foreach ($population in $popList) {
    foreach ($box in $boxList) {
        $world = Get-ClusterWorld -Population $population -Box $box
        $startedWorlds[('{0}|{1}' -f $population, $box)] = $world
        foreach ($threadCount in $threadList) {
            for ($repetition = 1; $repetition -le $Repeat; $repetition++) {
                $cellIndex++
                if (-not (Wait-ForIdleWindow -MaxPct $MaxLoadPct -WaitSec $LoadWaitSec)) {
                    Add-SkippedLoadRow -Kind 'cluster' -Population $population -Box $box -ThreadsRequested $threadCount -Rep $repetition -Index $cellIndex -Total $cellCount
                    continue
                }
                $row = Invoke-ClusterCell -Kind 'cluster' -Population $population -Box $box -ThreadsRequested $threadCount -Rep $repetition -World $world
                $rows.Add($row)
                Write-RowProgress -Row ([pscustomobject]$row) -Index $cellIndex -Total $cellCount
            }
        }
    }
    # Spread control: run once per (population, threads), not per box.
    if (-not $NoSpread) {
        foreach ($threadCount in $threadList) {
            for ($repetition = 1; $repetition -le $Repeat; $repetition++) {
                $cellIndex++
                if (-not (Wait-ForIdleWindow -MaxPct $MaxLoadPct -WaitSec $LoadWaitSec)) {
                    Add-SkippedLoadRow -Kind 'spread' -Population $population -Box $null -ThreadsRequested $threadCount -Rep $repetition -Index $cellIndex -Total $cellCount
                    continue
                }
                $row = Invoke-ClusterCell -Kind 'spread' -Population $population -Box 0.0 -ThreadsRequested $threadCount -Rep $repetition -World $null
                $rows.Add($row)
                Write-RowProgress -Row ([pscustomobject]$row) -Index $cellIndex -Total $cellCount
            }
        }
    }
}

# --- optional canonical core probe -----------------------------------------
$probeRows = New-Object System.Collections.Generic.List[object]
$probeLogText = $null
if ($CoreProbe) {
    if (-not $Quiet) { Write-Host 'running canonical core probe (physics-only, starter genome)...' }
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) { throw 'cargo not found on PATH (needed for -CoreProbe)' }
    $probeArgs = @('test', '--release', '-p', 'bibites-core', '--lib', '--', '--ignored', '--nocapture', '--test-threads=1', 'perf_probe_clustered_and_spread')
    $probeResult = Invoke-BenchProcess -FilePath $cargo.Source -ArgumentList $probeArgs -TimeoutSec 900 -SampleMs 500 -WorkingDirectory $repo
    $probeLogText = ($probeResult.StdOut + "`r`n" + $probeResult.StdErr).Trim()
    Write-BenchTextFile -Path (Join-Path $outDir 'core_probe.log') -Text ($probeLogText + "`r`n")
    if ($probeResult.ExitCode -ne 0) {
        Write-Warning "core probe failed with exit code $($probeResult.ExitCode); see core_probe.log"
    } else {
        foreach ($line in ($probeLogText -split "`r?`n")) {
            $match = [regex]::Match($line, 'PERF\s+(?<case>\S+)\s+bibites=(?<bibites>\d+)\s+elapsed=(?<ms>[0-9.]+)ms\s+pairs=(?<pairs>\d+)\s+contacts=(?<contacts>\d+)\s+triggers=(?<triggers>\d+)')
            if ($match.Success) {
                $probeRows.Add([ordered]@{
                    case     = $match.Groups['case'].Value
                    bibites  = [int]$match.Groups['bibites'].Value
                    elapsed_ms = [double]$match.Groups['ms'].Value
                    pairs    = [int]$match.Groups['pairs'].Value
                    contacts = [int]$match.Groups['contacts'].Value
                    triggers = [int]$match.Groups['triggers'].Value
                })
            }
        }
        if (-not $Quiet) {
            foreach ($probeRow in $probeRows) {
                Write-Host ('  core probe {0}: {1} bibites, {2} ms, pairs={3}' -f $probeRow.case, $probeRow.bibites, (Format-BenchNumber $probeRow.elapsed_ms '0.00'), $probeRow.pairs)
            }
        }
    }
}

# --- determinism check ------------------------------------------------------
$determinism = @()
if ($DeterminismCheck) {
    if (-not $Quiet) { Write-Host 'determinism check: regenerating a world and re-running a cell...' }
    $checkPopulation = $popList[0]
    $checkBox = $boxList[0]
    $arguments = @(
        $generator, '--n', [string]$checkPopulation, '--box', (Format-BenchNumber $checkBox '0.####'),
        '--seed', [string]$Seed, '--exe', $exeInfo.RunPath, '--world-dir', $worldRoot, '--json', '--force'
    )
    $regen = Invoke-BenchProcess -FilePath $python.Source -ArgumentList $arguments -TimeoutSec 300 -SampleMs 200
    $regenLine = @(($regen.StdOut -split "`r?`n") | Where-Object { $_ -match '^\s*\{' } | Select-Object -Last 1)
    $regenWorld = if ($regenLine.Count -gt 0) { $regenLine[0] | ConvertFrom-Json } else { $null }
    $knownWorld = $startedWorlds[('{0}|{1}' -f $checkPopulation, $checkBox)]
    $worldDeterministic = $null -ne $regenWorld -and $regenWorld.content_sha256 -eq $knownWorld.content_sha256
    $fingerprints = @()
    for ($attempt = 1; $attempt -le 2; $attempt++) {
        $jsonArgs = @('bench', '--scenario', $knownWorld.world_path, '--ticks', [string]$Ticks, '--threads', '1', '--repeat', '1', '--json')
        $jsonResult = Invoke-BenchProcess -FilePath $exeInfo.RunPath -ArgumentList $jsonArgs -TimeoutSec $TimeoutSec -SampleMs $SampleMs
        $reports = @(ConvertFrom-BenchJsonLines $jsonResult.StdOut)
        $fingerprints += if ($reports.Count -gt 0) { [string]$reports[0].state_fingerprint } else { 'ERROR' }
    }
    $determinism = @([pscustomobject]@{
        population        = $checkPopulation
        box               = $checkBox
        world_content_sha256 = $knownWorld.content_sha256
        regenerated_sha256   = if ($null -ne $regenWorld) { $regenWorld.content_sha256 } else { $null }
        world_identical      = $worldDeterministic
        fingerprints         = $fingerprints
        fingerprint_identical = (@($fingerprints | Sort-Object -Unique).Count -le 1)
    })
    Write-BenchTextFile -Path (Join-Path $outDir 'determinism.json') -Text ($determinism | ConvertTo-Json -Depth 6)
}

# --- aggregate --------------------------------------------------------------
$okRows = @($rows | Where-Object { $_.status -eq 'ok' })
$summary = New-Object System.Collections.Generic.List[object]
foreach ($kindGroup in ($okRows | Group-Object -Property { '{0}|{1}|{2}|{3}' -f $_.kind, $_.population, (Format-BenchOptional $_.box '0.####'), $_.threads })) {
    $groupRows = @($kindGroup.Group)
    $first = $groupRows[0]
    $tps = @($groupRows | ForEach-Object { [double]$_.ticks_per_second })
    $physics = @($groupRows | ForEach-Object { if ($null -ne $_.physics_ms) { [double]$_.physics_ms } })
    $body = @($groupRows | ForEach-Object { if ($null -ne $_.body_ms) { [double]$_.body_ms } })
    $peak = @($groupRows | ForEach-Object { if ($null -ne $_.peak_ws_mb) { [double]$_.peak_ws_mb } })
    $summary.Add([pscustomobject][ordered]@{
        kind            = $first.kind
        population      = [int]$first.population
        box             = if ($null -ne $first.box) { [double]$first.box } else { $null }
        threads         = [int]$first.threads
        reps_ok         = $groupRows.Count
        best_ticks_per_second = ($tps | Measure-Object -Maximum).Maximum
        median_ticks_per_second = Get-BenchMedian $tps
        best_ms_per_tick = if (($tps | Measure-Object -Maximum).Maximum -gt 0) { 1000.0 / ($tps | Measure-Object -Maximum).Maximum } else { $null }
        median_physics_ms = Get-BenchMedian $physics
        min_physics_ms  = if ($physics.Count -gt 0) { ($physics | Measure-Object -Minimum).Minimum } else { $null }
        median_body_ms  = Get-BenchMedian $body
        peak_ws_mb      = if ($peak.Count -gt 0) { ($peak | Measure-Object -Maximum).Maximum } else { $null }
        world_content_sha256 = [string]$first.world_content_sha256
    })
}

# --- CSV --------------------------------------------------------------------
$runHeaderFields = @('kind', 'population', 'box', 'threads', 'rep', 'status', 'ticks', 'bibites', 'wall_secs', 'ticks_per_second', 'realtime_factor', 'bibite_updates_per_second', 'ms_per_tick', 'physics_ms', 'body_ms', 'sensing_ms', 'hash_sync_ms', 'deaths_ms', 'total_phase_ms', 'process_wall_secs', 'peak_ws_mb', 'world_content_sha256', 'error')
$runLines = New-Object System.Collections.Generic.List[string]
$runLines.Add((ConvertTo-BenchCsvLine $runHeaderFields))
foreach ($row in $rows) {
    $rowObject = [pscustomobject]$row
    $runLines.Add((ConvertTo-BenchCsvLine @(
        $rowObject.kind
        (Format-BenchOptional $rowObject.population '0')
        (Format-BenchOptional $rowObject.box '0.####')
        (Format-BenchOptional $rowObject.threads '0')
        (Format-BenchOptional $rowObject.rep '0')
        $rowObject.status
        (Format-BenchOptional $rowObject.ticks '0')
        (Format-BenchOptional $rowObject.bibites '0')
        (Format-BenchOptional $rowObject.wall_secs '0.000000')
        (Format-BenchOptional $rowObject.ticks_per_second '0.000')
        (Format-BenchOptional $rowObject.realtime_factor '0.000')
        (Format-BenchOptional $rowObject.bibite_updates_per_second '0.0')
        (Format-BenchOptional $rowObject.ms_per_tick '0.000000')
        (Format-BenchOptional $rowObject.physics_ms '0.000000')
        (Format-BenchOptional $rowObject.body_ms '0.000000')
        (Format-BenchOptional $rowObject.sensing_ms '0.000000')
        (Format-BenchOptional $rowObject.hash_sync_ms '0.000000')
        (Format-BenchOptional $rowObject.deaths_ms '0.000000')
        (Format-BenchOptional $rowObject.total_phase_ms '0.000000')
        (Format-BenchOptional $rowObject.process_wall_secs '0.000000')
        (Format-BenchOptional $rowObject.peak_ws_mb '0.0')
        (Format-BenchOptional $rowObject.world_content_sha256 '')
        $rowObject.error
    )))
}
Write-BenchTextFile -Path (Join-Path $outDir 'cluster.csv') -Text (($runLines -join "`r`n") + "`r`n")

$summaryHeaderFields = @('kind', 'population', 'box', 'threads', 'reps_ok', 'best_ticks_per_second', 'median_ticks_per_second', 'best_ms_per_tick', 'min_physics_ms', 'median_physics_ms', 'median_body_ms', 'peak_ws_mb', 'world_content_sha256')
$summaryLines = New-Object System.Collections.Generic.List[string]
$summaryLines.Add((ConvertTo-BenchCsvLine $summaryHeaderFields))
foreach ($cell in $summary) {
    $summaryLines.Add((ConvertTo-BenchCsvLine @(
        $cell.kind
        (Format-BenchOptional $cell.population '0')
        (Format-BenchOptional $cell.box '0.####')
        (Format-BenchOptional $cell.threads '0')
        (Format-BenchOptional $cell.reps_ok '0')
        (Format-BenchOptional $cell.best_ticks_per_second '0.000')
        (Format-BenchOptional $cell.median_ticks_per_second '0.000')
        (Format-BenchOptional $cell.best_ms_per_tick '0.000000')
        (Format-BenchOptional $cell.min_physics_ms '0.000000')
        (Format-BenchOptional $cell.median_physics_ms '0.000000')
        (Format-BenchOptional $cell.median_body_ms '0.000000')
        (Format-BenchOptional $cell.peak_ws_mb '0.0')
        (Format-BenchOptional $cell.world_content_sha256 '')
    )))
}
Write-BenchTextFile -Path (Join-Path $outDir 'cluster_summary.csv') -Text (($summaryLines -join "`r`n") + "`r`n")

if ($probeRows.Count -gt 0) {
    $probeLines = New-Object System.Collections.Generic.List[string]
    $probeLines.Add('case,bibites,elapsed_ms,pairs,contacts,triggers')
    foreach ($probeRow in $probeRows) {
        $probeLines.Add((ConvertTo-BenchCsvLine @(
            $probeRow.case
            (Format-BenchOptional $probeRow.bibites '0')
            (Format-BenchOptional $probeRow.elapsed_ms '0.000000')
            (Format-BenchOptional $probeRow.pairs '0')
            (Format-BenchOptional $probeRow.contacts '0')
            (Format-BenchOptional $probeRow.triggers '0')
        )))
    }
    Write-BenchTextFile -Path (Join-Path $outDir 'core_probe.csv') -Text (($probeLines -join "`r`n") + "`r`n")
}

# --- Markdown ---------------------------------------------------------------
$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add('# RUSTibites dense-cluster benchmark')
$markdown.Add('')
$markdown.Add('_Auto-generated by `tools/bench/run_cluster.ps1` at ' + (Get-Date).ToUniversalTime().ToString('o') + '._')
$markdown.Add('')
$markdown.Add('Populations: ' + (($popList | ForEach-Object { Format-BenchNumber $_ 'N0' }) -join ', ') +
    ' - boxes: ' + (($boxList | ForEach-Object { Format-BenchNumber $_ '0.####' }) -join ', ') +
    ' - threads: ' + ($threadList -join ', ') + ' - ticks/run: ' + $Ticks + ' - repeat: ' + $Repeat + ' - seed: ' + $Seed + '.')
$markdown.Add('')
$markdown.Add('- **Binary:** `' + $exeInfo.Sha256 + '` (' + $exeInfo.OriginalPath + ')')
if ($null -ne $environment.git) { $markdown.Add('- **Git:** ' + $environment.git.head_short + ' (dirty: ' + $environment.git.dirty + ')') }
$markdown.Add('- **Worlds:** deterministic save zips under `' + $worldRoot + '`; content SHA-256 per cell in `cluster.csv`.')
$markdown.Add('- **Caveat:** whole-tick cluster numbers here use the bundled `cluster.bb8template` (tracked io fixture genome) and a minimal one-zone ecology (no plant/meat seeding, no red death) so the measurement is dominated by bibite physics/body. The canonical physics-only metric with the *starter* genome is the core probe (`-CoreProbe`); the two are not numerically interchangeable.')
$markdown.Add('')
$markdown.Add('## Whole-engine benchmark')
$markdown.Add('')
$markdown.Add('| Kind | Population | Box | Threads | Best ticks/s | Best ms/tick | Physics ms (median) | Body ms (median) | Peak WS (MB) |')
$markdown.Add('| --- | --- | --- | --- | --- | --- | --- | --- | --- |')
foreach ($cell in ($summary | Sort-Object -Property kind, population, box, threads)) {
    $markdown.Add('| ' + $cell.kind + ' | ' + (Format-BenchOptional $cell.population 'N0') + ' | ' +
        (Format-BenchOptional $cell.box '0.####') + ' | ' + $cell.threads + ' | ' +
        (Format-BenchOptional $cell.best_ticks_per_second 'N1') + ' | ' +
        (Format-BenchOptional $cell.best_ms_per_tick '0.00') + ' | ' +
        (Format-BenchOptional $cell.median_physics_ms '0.00') + ' | ' +
        (Format-BenchOptional $cell.median_body_ms '0.00') + ' | ' +
        (Format-BenchOptional $cell.peak_ws_mb '0.0') + ' |')
}
$markdown.Add('')

# Cluster vs spread physics ratio table (same population/threads; cluster box varies).
$markdown.Add('## Dense-pile cliff (cluster vs spread control)')
$markdown.Add('')
$markdown.Add('| Population | Box | Threads | Cluster physics ms | Spread physics ms | Cluster tick ms | Spread tick ms | Spread/Cluster ticks/s |')
$markdown.Add('| --- | --- | --- | --- | --- | --- | --- | --- |')
foreach ($clusterCell in ($summary | Where-Object { $_.kind -eq 'cluster' } | Sort-Object population, box, threads)) {
    $spreadCell = @($summary | Where-Object { $_.kind -eq 'spread' -and $_.population -eq $clusterCell.population -and $_.threads -eq $clusterCell.threads } | Select-Object -First 1)
    $spreadPhysics = if ($spreadCell.Count -gt 0) { $spreadCell[0].median_physics_ms } else { $null }
    $spreadMs = if ($spreadCell.Count -gt 0) { $spreadCell[0].best_ms_per_tick } else { $null }
    $ratio = if ($null -ne $spreadCell.Count -and $spreadCell.Count -gt 0 -and $null -ne $clusterCell.best_ticks_per_second -and $clusterCell.best_ticks_per_second -gt 0) {
        [double]$spreadCell[0].best_ticks_per_second / [double]$clusterCell.best_ticks_per_second
    } else { $null }
    $markdown.Add('| ' + (Format-BenchOptional $clusterCell.population 'N0') + ' | ' +
        (Format-BenchOptional $clusterCell.box '0.####') + ' | ' + $clusterCell.threads + ' | ' +
        (Format-BenchOptional $clusterCell.median_physics_ms '0.00') + ' | ' +
        (Format-BenchOptional $spreadPhysics '0.00') + ' | ' +
        (Format-BenchOptional $clusterCell.best_ms_per_tick '0.00') + ' | ' +
        (Format-BenchOptional $spreadMs '0.00') + ' | ' +
        (Format-BenchOptional $ratio '0.0') + 'x |')
}
$markdown.Add('')
$markdown.Add('Reading: cluster physics is expected to be orders of magnitude above spread at the same population; the ratio is the size of the dense-pile problem. The task-82 target is `< 50 ms` for the physics-only core-probe `cluster2000` case, not for the whole-tick numbers in this table.')
$markdown.Add('')

if ($probeRows.Count -gt 0) {
    $markdown.Add('## Canonical core probe (`body::perf_probe_clustered_and_spread`, physics only)')
    $markdown.Add('')
    $markdown.Add('| Case | Bibites | Physics ms | Candidate pairs | Contacts | Triggers |')
    $markdown.Add('| --- | --- | --- | --- | --- | --- |')
    foreach ($probeRow in $probeRows) {
        $markdown.Add('| ' + $probeRow.case + ' | ' + $probeRow.bibites + ' | ' + (Format-BenchNumber $probeRow.elapsed_ms '0.00') + ' | ' + $probeRow.pairs + ' | ' + $probeRow.contacts + ' | ' + $probeRow.triggers + ' |')
    }
    $markdown.Add('')
    $markdown.Add('This probe uses `BibiteStore::step_physics` only (no body/sensing/decay), starter genome, spread half-extent 1900 and cluster half-extent 50 (100x100 box), so it is the fast iteration signal for the dense-cluster physics work. `core_probe.log` has the raw output.')
    $markdown.Add('')
}

if ($determinism.Count -gt 0) {
    $markdown.Add('## Determinism')
    $markdown.Add('')
    foreach ($check in $determinism) {
        $markdown.Add('- world n=' + $check.population + ' box=' + (Format-BenchNumber $check.box '0.####') + ': content SHA reproducible = ' + $check.world_identical + ', fingerprints identical = ' + $check.fingerprint_identical)
    }
    $markdown.Add('')
}

$markdown.Add('## Notes')
$markdown.Add('')
$markdown.Add('- `ms_per_tick = 1000 / ticks/s`; `physics_ms`/`body_ms` are phase means over all measured ticks.')
$markdown.Add('- Worlds are generated once per (population, box, seed, engine SHA) and cached under `target/bench-worlds/cluster/`; pass `-ForceWorlds` to regenerate.')
$markdown.Add('- The scenario inputs are tracked under `tools/bench/scenarios/cluster/` (no testdata/game install needed); Python 3.8+ is only needed to generate the world zips.')
$markdown.Add('- Run this on an idle machine (or with `-MaxLoadPct`/`-LoadWaitSec`): the whole tick is slow by design, and the cluster box makes runtime scale quickly with population.')
$markdown.Add('')
Write-BenchTextFile -Path (Join-Path $outDir 'cluster.md') -Text (($markdown -join "`r`n") + "`r`n")

$clusterJson = [ordered]@{
    schema            = 'rustibites.bench.cluster/1'
    generated_at_utc  = (Get-Date).ToUniversalTime().ToString('o')
    config            = $config
    worlds            = @($startedWorlds.Values)
    cells             = $summary
    core_probe        = $probeRows
    determinism       = $determinism
    binary            = $exeInfo
}
Write-BenchTextFile -Path (Join-Path $outDir 'cluster.json') -Text ($clusterJson | ConvertTo-Json -Depth 8)

if (-not $Quiet) {
    Write-Host ''
    Write-Host '--- cluster summary (whole-engine best/median per cell) ---'
    $summary | Format-Table kind, population, box, threads, reps_ok, best_ticks_per_second, best_ms_per_tick, median_physics_ms, median_body_ms, peak_ws_mb -AutoSize
    Write-Host ''
    Write-Host "artifacts: $outDir"
    Write-Host '  cluster.csv, cluster_summary.csv, cluster.md, cluster.json, config.json, environment.json, environment.txt'
    if ($probeRows.Count -gt 0) { Write-Host '  core_probe.csv, core_probe.log' }
    if ($determinism.Count -gt 0) { Write-Host '  determinism.json' }
    $failed = @($rows | Where-Object { $_.status -ne 'ok' })
    if ($failed.Count -gt 0) { Write-Warning ("{0} run(s) did not complete cleanly (see cluster.csv status/error)" -f $failed.Count) }
}

if (@($summary | Where-Object { $_.reps_ok -gt 0 }).Count -eq 0) { exit 2 }
exit 0
