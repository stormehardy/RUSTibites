<#
.SYNOPSIS
    Shared helpers for the RUSTibites benchmark harness (`tools/bench`).

.DESCRIPTION
    Dot-source this file from the other tools/bench scripts:

        . (Join-Path $PSScriptRoot 'bench_common.ps1')

    It only uses PowerShell core cmdlets, .NET APIs and command-line tools that
    are already installed (`git`, `rustc`, `cargo`; all optional). It is
    intentionally compatible with Windows PowerShell 5.1 as well as PowerShell
    7+ on Windows, macOS and Linux (no external modules, no PS7-only syntax).

    Nothing in this file writes to the repository; callers decide where
    artifacts go.
#>

# All numbers written to CSV/Markdown are formatted with the invariant culture
# so artifacts are reproducible regardless of the contributor's locale.
$script:BenchInvariant = [System.Globalization.CultureInfo]::InvariantCulture

# Phase names printed by `rustibites bench` (human output), in engine order.
# Keep in sync with `bibites_core::sim::Phase::ALL` (crates/bibites-core/src/sim.rs).
$script:BenchPhaseNames = @(
    'clock',
    'animators',
    'tools',
    'zones',
    'sensing',
    'body',
    'eggs',
    'pheromones',
    'decay',
    'rad',
    'red-death',
    'colour-killers',
    'producers',
    'deaths',
    'physics',
    'events-stats',
    'hash-sync'
)

function Test-BenchWindows {
    if ($null -ne $env:OS -and $env:OS -eq 'Windows_NT') { return $true }
    return ([System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT)
}

# Resolves the repository root from a script directory (<repo>/tools/bench).
function Get-BenchRepo {
    param([string]$ScriptRoot = $PSScriptRoot)
    $tools = Split-Path -Parent $ScriptRoot
    return (Resolve-Path (Split-Path -Parent $tools)).Path
}

# Invariant-culture formatting. Returns '' for $null so optional CSV fields stay empty.
function Format-BenchNumber {
    param($Value, [string]$Format = '0.000')
    if ($null -eq $Value) { return '' }
    if ($Value -is [string]) { return $Value }
    $composite = '{0:' + $Format + '}'
    return [string]::Format([System.Globalization.CultureInfo]::InvariantCulture, $composite, $Value)
}

# Writes text as UTF-8 without a BOM (PowerShell 5.1's Set-Content would add one).
function Write-BenchTextFile {
    param([string]$Path, [string]$Text)
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $encoding)
}

function ConvertTo-BenchCsvField {
    param($Value)
    if ($null -eq $Value) { return '' }
    $text = [string]$Value
    if ($text.IndexOfAny([char[]]@(',', '"', "`r", "`n")) -ge 0) {
        return '"' + $text.Replace('"', '""') + '"'
    }
    return $text
}

function ConvertTo-BenchCsvLine {
    param($Fields)
    $cells = @()
    foreach ($field in $Fields) { $cells += (ConvertTo-BenchCsvField $field) }
    return ($cells -join ',')
}

# Quotes a program argument for the Windows command line (paths with spaces).
function ConvertTo-BenchCommandLine {
    param([string[]]$ArgumentList)
    $parts = @()
    foreach ($argument in $ArgumentList) {
        $text = [string]$argument
        if ($text.Length -eq 0) { $parts += '""'; continue }
        if ($text -match '[\s"]') {
            $parts += ('"' + ($text -replace '"', '\"') + '"')
        } else {
            $parts += $text
        }
    }
    return ($parts -join ' ')
}

function Get-BenchMedian {
    param([double[]]$Values)
    $sorted = @($Values | Where-Object { $null -ne $_ } | Sort-Object)
    if ($sorted.Count -eq 0) { return $null }
    $middle = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return [double]$sorted[$middle] }
    return ([double]$sorted[$middle - 1] + [double]$sorted[$middle]) / 2.0
}

# Parses a comma/semicolon/space separated integer list.
#
# `powershell -File script.ps1 -Pops 1000,2000` passes one string
# "1000,2000" to a [string[]] parameter, so the scripts accept strings and split
# here instead of relying on -File to build a real array.
function ConvertTo-BenchIntList {
    param([string[]]$Values, [int[]]$Default = @())
    $result = New-Object System.Collections.Generic.List[int]
    $source = @()
    if ($null -ne $Values) { $source = @($Values) }
    if ($source.Count -eq 0) { $source = @($Default | ForEach-Object { [string]$_ }) }
    foreach ($value in $source) {
        foreach ($part in ([string]$value -split '[,; ]+')) {
            $trimmed = $part.Trim()
            if ($trimmed.Length -eq 0) { continue }
            $parsed = 0
            if (-not [int]::TryParse($trimmed, [ref]$parsed)) {
                throw "invalid integer list value '$trimmed' (expected e.g. 1000,2000,5000)"
            }
            if (-not $result.Contains($parsed)) { $result.Add($parsed) }
        }
    }
    return $result.ToArray()
}

# ---------------------------------------------------------------------------
# Executable discovery
# ---------------------------------------------------------------------------

# Resolves the release CLI binary. Never rebuilds implicitly: `-Build` is an
# explicit opt-in so a shared machine is not disturbed by a surprise LTO build.
# Returns a descriptor with both the original path and a stable temporary copy.
# The copy exists so a concurrent `cargo build` cannot swap the binary in the
# middle of a matrix (and so we do not lock target/release against other tools).
function Resolve-BenchExecutable {
    param(
        [Parameter(Mandatory = $true)][string]$Repo,
        [string]$Exe = '',
        [switch]$Build,
        [switch]$NoCopy
    )

    $extension = if (Test-BenchWindows) { '.exe' } else { '' }
    $defaultPath = Join-Path (Join-Path (Join-Path $Repo 'target') 'release') ('rustibites' + $extension)
    if ($Exe) {
        if (-not (Test-Path -LiteralPath $Exe)) { throw "benchmark binary not found: $Exe" }
        $original = (Resolve-Path -LiteralPath $Exe).Path
    } else {
        $original = $defaultPath
    }

    if ($Build) {
        $cargo = Get-Command cargo -ErrorAction SilentlyContinue
        if (-not $cargo) { throw 'cargo was not found on PATH; cannot build the release CLI' }
        $cargoArgs = @('build', '--release', '-p', 'bibites-cli', '--manifest-path', (Join-Path $Repo 'Cargo.toml'))
        Write-Host ('[bench] building release CLI: cargo ' + ($cargoArgs -join ' '))
        Push-Location $Repo
        try {
            & cargo @cargoArgs
            $buildExit = $LASTEXITCODE
        } finally {
            Pop-Location
        }
        if ($buildExit -ne 0) { throw "cargo build failed with exit code $buildExit" }
    } elseif (-not (Test-Path -LiteralPath $original)) {
        # Never rebuild implicitly: on shared machines a concurrent build can make
        # the binary briefly disappear and a surprise LTO build would disturb it.
        throw "benchmark binary not found: $original (run: cargo build --release -p bibites-cli, or pass -Build to let the harness build it)"
    }

    $item = Get-Item -LiteralPath $original
    $hash = (Get-FileHash -LiteralPath $original -Algorithm SHA256).Hash
    $runPath = $original
    $wasCopied = $false
    if (-not $NoCopy) {
        $tempBin = Join-Path (Join-Path ([System.IO.Path]::GetTempPath()) 'rustibites-bench') 'bin'
        New-Item -ItemType Directory -Force -Path $tempBin | Out-Null
        $runPath = Join-Path $tempBin ('rustibites-' + $hash.Substring(0, 16) + $extension)
        if (-not (Test-Path -LiteralPath $runPath)) {
            Copy-Item -LiteralPath $original -Destination $runPath -Force
        }
        $copyHash = (Get-FileHash -LiteralPath $runPath -Algorithm SHA256).Hash
        if ($copyHash -ne $hash) { throw "temporary benchmark binary copy does not match $original" }
        $wasCopied = $true
    }

    return [pscustomobject]@{
        OriginalPath     = $original
        RunPath          = $runPath
        Sha256           = $hash
        SizeBytes        = $item.Length
        LastWriteTimeUtc = $item.LastWriteTimeUtc.ToString('o')
        WasCopied        = $wasCopied
    }
}

# ---------------------------------------------------------------------------
# Process execution
# ---------------------------------------------------------------------------

# Runs one benchmark process with a wall-clock timeout, sampling peak working set
# while it is alive. `wall_secs` reported by the engine only covers
# `world.run(ticks)`; this helper measures the full process lifetime (spawn +
# world setup + ticks) which is why both numbers are recorded.
function Invoke-BenchProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [int]$TimeoutSec = 600,
        [int]$SampleMs = 50,
        [string]$WorkingDirectory = ''
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = ConvertTo-BenchCommandLine $ArgumentList
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.CreateNoWindow = $true
    if ($WorkingDirectory) { $startInfo.WorkingDirectory = $WorkingDirectory }

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = [System.Diagnostics.Process]::Start($startInfo)
    $stdoutTask = $proc.StandardOutput.ReadToEndAsync()
    $stderrTask = $proc.StandardError.ReadToEndAsync()

    $peakBytes = 0L
    $timedOut = $false
    $deadline = if ($TimeoutSec -gt 0) { [DateTime]::UtcNow.AddSeconds([double]$TimeoutSec) } else { [DateTime]::MaxValue }
    while (-not $proc.HasExited) {
        if ([DateTime]::UtcNow -gt $deadline) {
            $timedOut = $true
            try { $proc.Kill() } catch { }
            break
        }
        try {
            $proc.Refresh()
            if (-not $proc.HasExited) {
                $workingSet = $proc.WorkingSet64
                if ($workingSet -gt $peakBytes) { $peakBytes = $workingSet }
            }
        } catch {
            # Process exited between HasExited and Refresh; the code below reads final values.
        }
        Start-Sleep -Milliseconds $SampleMs
    }
    try { $proc.WaitForExit() } catch { }
    $watch.Stop()
    try {
        $proc.Refresh()
        $peakAfter = $proc.PeakWorkingSet64
        if ($peakAfter -gt $peakBytes) { $peakBytes = $peakAfter }
    } catch { }

    $exitCode = $null
    try { $exitCode = $proc.ExitCode } catch { }
    $stdout = ''
    $stderr = ''
    try { $stdout = $stdoutTask.Result } catch { }
    try { $stderr = $stderrTask.Result } catch { }

    return [pscustomobject]@{
        ExitCode            = $exitCode
        TimedOut            = $timedOut
        WallSecs            = $watch.Elapsed.TotalSeconds
        StdOut              = $stdout
        StdErr              = $stderr
        PeakWorkingSetBytes = $peakBytes
    }
}

# Parses the one-JSON-object-per-repetition output of `bench --json`.
function ConvertFrom-BenchJsonLines {
    param([string]$Text)
    $reports = New-Object System.Collections.Generic.List[object]
    if (-not $Text) { return @() }
    foreach ($line in ($Text -split "`r?`n")) {
        if ($line -notmatch '^\s*\{') { continue }
        try { $reports.Add(($line | ConvertFrom-Json)) } catch { }
    }
    return $reports.ToArray()
}

# Parses the human `bench` output: one summary + phase table per repetition.
function Parse-BenchPrettyOutput {
    param([string]$Text)
    $runs = New-Object System.Collections.Generic.List[object]
    $current = $null
    if (-not $Text) { return $runs.ToArray() }
    $summaryPattern = '^(?<label>.+?): (?<pop>\d+) bibites, (?<ticks>\d+) ticks in (?<wall>[0-9.]+)s = (?<tps>[0-9.]+) ticks/s, (?<rt>[0-9.]+)x realtime, (?<upd>[0-9.]+) bibite-updates/s(?:, (?<threads>\d+) threads)?$'
    $phasePattern = '^(?<name>[A-Za-z0-9\-]+)\s+(?<ms>[0-9.]+)\s+(?<share>[0-9.]+)%$'
    foreach ($rawLine in ($Text -split "`r?`n")) {
        $line = $rawLine.TrimEnd()
        if ($line -match '^best: ') { continue }
        $match = [regex]::Match($line, $summaryPattern)
        if ($match.Success) {
            $current = [ordered]@{
                label          = $match.Groups['label'].Value.Trim()
                bibites        = [int]$match.Groups['pop'].Value
                ticks          = [int]$match.Groups['ticks'].Value
                wall_secs      = [double]$match.Groups['wall'].Value
                ticks_per_second = [double]$match.Groups['tps'].Value
                realtime_factor  = [double]$match.Groups['rt'].Value
                bibite_updates_per_second = [double]$match.Groups['upd'].Value
                threads        = if ($match.Groups['threads'].Success) { [int]$match.Groups['threads'].Value } else { 0 }
                phases         = [ordered]@{}
                total_phase_ms = 0.0
            }
            $runs.Add($current)
            continue
        }
        if ($null -ne $current) {
            $phaseMatch = [regex]::Match($line, $phasePattern)
            if ($phaseMatch.Success) {
                $name = $phaseMatch.Groups['name'].Value
                $ms = [double]$phaseMatch.Groups['ms'].Value
                $phases = $current['phases']
                $phases[$name] = [pscustomobject]@{
                    ms    = $ms
                    share = [double]$phaseMatch.Groups['share'].Value
                }
                $current['total_phase_ms'] = [double]$current['total_phase_ms'] + $ms
            }
        }
    }
    return $runs.ToArray()
}

# ---------------------------------------------------------------------------
# Machine / load probes
# ---------------------------------------------------------------------------

function Get-BenchFreeGB {
    try {
        if (Test-BenchWindows) {
            $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
            return [math]::Round([double]$os.FreePhysicalMemory / 1MB, 2)
        }
        foreach ($line in (Get-Content -LiteralPath '/proc/meminfo' -ErrorAction Stop)) {
            if ($line -match '^MemAvailable:\s+(\d+)\s+kB') {
                return [math]::Round([double]$matches[1] / 1048576.0, 2)
            }
        }
    } catch { }
    return $null
}

function Get-BenchCpuLoadPct {
    try {
        if (Test-BenchWindows) {
            $samples = @(Get-CimInstance Win32_Processor -ErrorAction Stop | ForEach-Object { [double]$_.LoadPercentage })
            if ($samples.Count -gt 0) { return [math]::Round((($samples | Measure-Object -Average).Average), 1) }
            return $null
        }
        $loadLine = (Get-Content -LiteralPath '/proc/loadavg' -ErrorAction Stop | Select-Object -First 1)
        $loadOne = [double]($loadLine -split '\s+')[0]
        $cores = [int][System.Environment]::ProcessorCount
        if ($cores -gt 0) { return [math]::Round(100.0 * $loadOne / $cores, 1) }
    } catch { }
    return $null
}

# Snapshot of factors that make measurements noisy: CPU load, concurrent
# cargo/rustc processes (other agents/builds) and other running RUSTibites
# processes (another benchmark or the viewer).
function Get-BenchLoadSnapshot {
    $rustcProcs = 0
    $cargoProcs = 0
    try {
        $procs = @(Get-Process -Name rustc, cargo -ErrorAction SilentlyContinue)
        foreach ($proc in $procs) {
            if ($proc.ProcessName -eq 'rustc') { $rustcProcs++ } else { $cargoProcs++ }
        }
    } catch { }
    $otherRustibites = 0
    try {
        # Our own bench children run as `rustibites-<hash>` and have exited
        # before this snapshot is taken (it is sampled between cells), so any
        # remaining RUSTibites process belongs to another agent or the viewer.
        $otherRustibites = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
            $_.ProcessName -like 'rustibites*' -and
            $_.ProcessName -notlike 'rustibites-view*' -and
            $_.Id -ne $PID
        }).Count
    } catch { }
    return [pscustomobject]@{
        captured_at_utc        = (Get-Date).ToUniversalTime().ToString('o')
        cpu_load_pct           = Get-BenchCpuLoadPct
        free_gb                = Get-BenchFreeGB
        rustc_procs            = $rustcProcs
        cargo_procs            = $cargoProcs
        other_rustibites_procs = $otherRustibites
    }
}

# ---------------------------------------------------------------------------
# Environment capture
# ---------------------------------------------------------------------------

function Get-BenchOsInfo {
    $os = [ordered]@{
        description = [System.Environment]::OSVersion.VersionString
        version     = [System.Environment]::OSVersion.Version.ToString()
        architecture = if (Test-BenchWindows) { $env:PROCESSOR_ARCHITECTURE } else { ( & uname -m 2>$null ) }
    }
    try {
        if (Test-BenchWindows) {
            $cim = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
            $os['description'] = [string]$cim.Caption
            $os['version'] = [string]$cim.Version
            $os['build'] = [string]$cim.BuildNumber
        } else {
            $pretty = (Get-Content -LiteralPath '/etc/os-release' -ErrorAction Stop | Where-Object { $_ -match '^PRETTY_NAME=' } | Select-Object -First 1)
            if ($pretty) { $os['description'] = ($pretty -replace '^PRETTY_NAME="?', '' -replace '"$', '') }
            if (Test-Path -LiteralPath '/proc/version') {
                $os['build'] = ((Get-Content -LiteralPath '/proc/version' -ErrorAction SilentlyContinue | Select-Object -First 1) -replace '\s+', ' ').Trim()
            }
        }
    } catch { }
    return [pscustomobject]$os
}

function Get-BenchCpuInfo {
    $info = [ordered]@{
        name            = $null
        physical_cores  = $null
        logical_cores   = [int][System.Environment]::ProcessorCount
        max_clock_mhz   = $null
    }
    try {
        if (Test-BenchWindows) {
            $cpus = @(Get-CimInstance Win32_Processor -ErrorAction Stop)
            if ($cpus.Count -gt 0) {
                $info['name'] = ([string]$cpus[0].Name).Trim()
                $info['physical_cores'] = [int](($cpus | Measure-Object -Property NumberOfCores -Sum).Sum)
                $info['logical_cores'] = [int](($cpus | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum)
                $info['max_clock_mhz'] = [int]$cpus[0].MaxClockSpeed
            }
        } else {
            if (Test-Path -LiteralPath '/proc/cpuinfo') {
                foreach ($line in (Get-Content -LiteralPath '/proc/cpuinfo' -ErrorAction SilentlyContinue)) {
                    if ($line -match '^model name\s*:\s*(.+)$') { $info['name'] = $matches[1].Trim(); break }
                }
            }
        }
    } catch { }
    return [pscustomobject]$info
}

function Get-BenchMemoryTotalGB {
    try {
        if (Test-BenchWindows) {
            $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
            return [math]::Round([double]$os.TotalVisibleMemorySize / 1MB, 2)
        }
        foreach ($line in (Get-Content -LiteralPath '/proc/meminfo' -ErrorAction Stop)) {
            if ($line -match '^MemTotal:\s+(\d+)\s+kB') {
                return [math]::Round([double]$matches[1] / 1048576.0, 2)
            }
        }
    } catch { }
    return $null
}

function Get-BenchRustInfo {
    $info = [ordered]@{
        rustc       = $null
        cargo       = $null
        release     = $null
        commit_hash = $null
        host        = $null
        llvm        = $null
    }
    try {
        if (Get-Command rustc -ErrorAction SilentlyContinue) {
            $versionText = (& rustc -Vv 2>$null) -join "`n"
            foreach ($line in ($versionText -split "`r?`n")) {
                if ($line -match '^rustc (.+)$') { $info['rustc'] = $matches[1].Trim() }
                elseif ($line -match '^release:\s*(.+)$') { $info['release'] = $matches[1].Trim() }
                elseif ($line -match '^commit-hash:\s*(.+)$') { $info['commit_hash'] = $matches[1].Trim() }
                elseif ($line -match '^host:\s*(.+)$') { $info['host'] = $matches[1].Trim() }
                elseif ($line -match '^LLVM version:\s*(.+)$') { $info['llvm'] = $matches[1].Trim() }
            }
        }
        if (Get-Command cargo -ErrorAction SilentlyContinue) {
            $cargoText = (& cargo -V 2>$null) -join ' '
            if ($cargoText -match '^cargo\s+(.+)$') { $info['cargo'] = $matches[1].Trim() }
        }
    } catch { }
    return [pscustomobject]$info
}

function Get-BenchGitInfo {
    param([string]$Repo)
    $info = [ordered]@{
        head         = $null
        head_short   = $null
        branch       = $null
        subject      = $null
        commit_date  = $null
        describe     = $null
        dirty        = $null
        dirty_files  = $null
        status_lines = @()
    }
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) { return [pscustomobject]$info }
    Push-Location $Repo
    try {
        $head = (& git rev-parse HEAD 2>$null | Select-Object -First 1)
        if ($head) {
            $info['head'] = [string]$head
            $info['head_short'] = [string]($head.Substring(0, [Math]::Min(12, $head.Length)))
        }
        $info['branch'] = [string]((& git branch --show-current 2>$null | Select-Object -First 1))
        $logLine = [string]((& git log -1 --format='%cI %s' 2>$null | Select-Object -First 1))
        if ($logLine) {
            $parts = $logLine -split ' ', 2
            $info['commit_date'] = $parts[0]
            if ($parts.Count -gt 1) { $info['subject'] = $parts[1] }
        }
        $describe = (& git describe --tags --always --dirty 2>$null | Select-Object -First 1)
        if ($describe) { $info['describe'] = [string]$describe }
        $status = @(& git status --porcelain 2>$null)
        $info['status_lines'] = $status
        $info['dirty_files'] = $status.Count
        $info['dirty'] = ($status.Count -gt 0)
    } catch { }
    finally { Pop-Location }
    return [pscustomobject]$info
}

# Parses [profile.release] from the workspace Cargo.toml (read-only) so the
# benchmark artifacts record exactly how the measured binary was built.
function Get-BenchReleaseProfile {
    param([string]$Repo)
    $entries = [ordered]@{}
    $raw = @()
    $manifest = Join-Path $Repo 'Cargo.toml'
    try {
        $inSection = $false
        foreach ($line in (Get-Content -LiteralPath $manifest -ErrorAction Stop)) {
            if ($line -match '^\s*\[(.+?)\]\s*$') {
                $inSection = ($matches[1].Trim() -eq 'profile.release')
                if ($inSection) { $raw += $line.Trim() }
                continue
            }
            if ($inSection) {
                $trimmed = $line.Trim()
                if ($trimmed.Length -gt 0) { $raw += $trimmed }
                if ($trimmed -match '^([A-Za-z0-9_.-]+)\s*=\s*(.+?)\s*$') {
                    $entries[$matches[1]] = $matches[2]
                }
            }
        }
    } catch { }
    return [pscustomobject]@{
        manifest = 'Cargo.toml'
        entries  = [pscustomobject]$entries
        raw      = ($raw -join "`n")
    }
}

# Assembles the machine/build metadata written next to every benchmark run.
function Get-BenchEnvironment {
    param(
        [Parameter(Mandatory = $true)][string]$Repo,
        $Executable = $null
    )
    $load = Get-BenchLoadSnapshot
    $binary = $null
    if ($null -ne $Executable) {
        $binary = [ordered]@{
            original_path     = $Executable.OriginalPath
            benchmark_copy    = $Executable.RunPath
            sha256            = $Executable.Sha256
            size_bytes        = $Executable.SizeBytes
            last_write_utc    = $Executable.LastWriteTimeUtc
        }
    }
    return [pscustomobject]([ordered]@{
        schema              = 'rustibites.bench.environment/1'
        captured_at_utc     = (Get-Date).ToUniversalTime().ToString('o')
        hostname            = [System.Environment]::MachineName
        os                  = Get-BenchOsInfo
        cpu                 = Get-BenchCpuInfo
        logical_processors  = [int][System.Environment]::ProcessorCount
        memory_total_gb     = Get-BenchMemoryTotalGB
        memory_free_gb      = $load.free_gb
        cpu_load_pct        = $load.cpu_load_pct
        powershell          = $PSVersionTable.PSVersion.ToString()
        rust                = Get-BenchRustInfo
        git                 = Get-BenchGitInfo -Repo $Repo
        release_profile     = Get-BenchReleaseProfile -Repo $Repo
        binary              = $binary
        simulator           = [pscustomobject]([ordered]@{
            default_sim_tps = 30
            note            = 'SimSettings::game_defaults() fixed_dt = 1/30 s; bench reports sim_secs = ticks * fixed_dt'
        })
        benchmark           = [pscustomobject]([ordered]@{
            engine_wall_secs_excludes = 'world construction/spawn and state_fingerprint sampling; process_wall_secs includes them'
            seed_base                 = '0x5255535400000000'
            seed_formula              = 'seed = 0x5255535400000000 xor repetition_index (repetition starts at 0)'
            metric                    = 'bibite_updates_per_second = ticks_per_second * bibites_start (primary throughput)'
        })
    })
}

function Format-BenchEnvironmentText {
    param($Environment)
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add('# RUSTibites benchmark environment')
    $lines.Add('')
    $lines.Add("Captured (UTC):        " + $Environment.captured_at_utc)
    $lines.Add("Host:                  " + $Environment.hostname)
    $lines.Add("OS:                    " + $Environment.os.description + " | version=" + $Environment.os.version)
    $cpu = $Environment.cpu
    $lines.Add("CPU:                   " + $cpu.name + " | physical=" + $cpu.physical_cores + " logical=" + $cpu.logical_cores + " | max_clock_mhz=" + $cpu.max_clock_mhz)
    $lines.Add("Memory:                " + (Format-BenchNumber $Environment.memory_total_gb '0.0') + " GB total, " + (Format-BenchNumber $Environment.memory_free_gb '0.00') + " GB free at capture")
    $lines.Add("CPU load at capture:   " + (Format-BenchNumber $Environment.cpu_load_pct '0.0') + " %")
    $lines.Add("PowerShell:            " + $Environment.powershell)
    $lines.Add("rustc:                 " + $Environment.rust.rustc + " (release " + $Environment.rust.release + ", " + $Environment.rust.host + ")")
    $lines.Add("cargo:                 " + $Environment.rust.cargo)
    $lines.Add("Git HEAD:              " + $Environment.git.head + " (" + $Environment.git.branch + ") " + $Environment.git.commit_date)
    $lines.Add("Git subject:           " + $Environment.git.subject)
    $lines.Add("Git dirty:             " + $Environment.git.dirty + " (" + $Environment.git.dirty_files + " changed paths)")
    if ($Environment.git.status_lines.Count -gt 0) {
        $lines.Add("Git status:")
        foreach ($statusLine in $Environment.git.status_lines) { $lines.Add("  " + $statusLine) }
    }
    $lines.Add("Release profile:")
    foreach ($property in $Environment.release_profile.entries.PSObject.Properties) {
        $lines.Add("  " + $property.Name + " = " + $property.Value)
    }
    if ($null -ne $Environment.binary) {
        $lines.Add("Benchmark binary:      " + $Environment.binary.original_path)
        $lines.Add("  sha256:              " + $Environment.binary.sha256)
        $lines.Add("  size:                " + $Environment.binary.size_bytes + " bytes")
        $lines.Add("  mtime (UTC):         " + $Environment.binary.last_write_utc)
        $lines.Add("  stable copy:         " + $Environment.binary.benchmark_copy)
    }
    $lines.Add("Simulator:             " + $Environment.simulator.default_sim_tps + " TPS (fixed_dt = 1/" + $Environment.simulator.default_sim_tps + " s)")
    $lines.Add("Seed:                  " + $Environment.benchmark.seed_formula)
    return ($lines -join "`r`n") + "`r`n"
}

function Write-BenchEnvironment {
    param($Environment, [string]$OutputDir)
    if (-not (Test-Path -LiteralPath $OutputDir)) { New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null }
    Write-BenchTextFile -Path (Join-Path $OutputDir 'environment.json') -Text ($Environment | ConvertTo-Json -Depth 8)
    Write-BenchTextFile -Path (Join-Path $OutputDir 'environment.txt') -Text (Format-BenchEnvironmentText $Environment)
}
