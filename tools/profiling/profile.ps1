<#
.SYNOPSIS
    Repeatable profiling workflows for the RUSTibites headless engine.

.DESCRIPTION
    One entry point for the profiling options that work on a stock Windows
    machine without installing anything:

        doctor         Report which profilers/instrumentation are available here.
        phase-audit    Run the ignored `perf_*` audits in bibites-core and save output.
        micro-probes   Run the ignored per-subsystem probes (body/sensing/executor).
        bench-phases   Capture `rustibites bench` human phase tables into CSV/Markdown.
        build-symbols  Build a release CLI with debug symbols WITHOUT editing Cargo.toml
                       (cargo --config overrides, isolated target dir).
        etw-capture    Capture a CPU ETW trace around a bench run with Windows
                       Performance Recorder and convert it with tracerpt.
                       Requires an elevated shell (WPR needs admin).

    Every action writes its artifacts under tools/profiling/results/ (or
    -OutputDir). See docs/PROFILING.md for interpretation and alternatives.

    This script does not edit crates/: adding new phase timers or symbols is a
    crate-owner change; the docs show exactly where those hooks are.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1
    # -> doctor report

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action phase-audit -TimeoutSec 900

.EXAMPLE
    # Build a symbolised profiling binary (isolated target dir, ~2-4 min LTO build).
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action build-symbols

.EXAMPLE
    # Elevated shell only (WPR cannot start a kernel ETW session as a normal user):
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action etw-capture -Pops 2000 -Ticks 400

.NOTES
    - No installs. If a better profiler is desired, the docs list the
      install-requires-approval options (Windows Performance Toolkit/WPA,
      `cargo install samply`, Visual Studio IDE, PerfView).
    - Keep profiling runs short on the shared reference machine: the ignored
      audits are seconds-to-a-minute, not 100k-tick marathon runs.
#>
[CmdletBinding()]
param(
    [ValidateSet('doctor', 'phase-audit', 'micro-probes', 'bench-phases', 'build-symbols', 'etw-capture')]
    [string]$Action = 'doctor',
    [string]$OutputDir = '',
    [string]$Exe = '',
    [switch]$Build,
    [double]$MaxLoadPct = 0,
    [int]$LoadWaitSec = 120,
    # phase-audit / micro-probes
    [string]$TestFilter = '',
    [int]$TestTimeoutSec = 900,
    # bench-phases
    [string]$Pops = '1000,2000,5000,10000,20000',
    [string]$Threads = '1,8',
    [int]$Ticks = 120,
    [string]$Scenario = '',
    [int]$TimeoutSec = 300,
    # build-symbols
    [string]$TargetDir = '',
    [switch]$Force,
    # etw-capture
    [string]$WprProfile = 'CPU',
    [int]$CaptureSeconds = 8,
    [switch]$ConvertWithTracerpt,
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

$benchDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'bench'
. (Join-Path $benchDir 'bench_common.ps1')

# env.ps1 has its own param block; preserve ours before dot-sourcing.
$optionExe = $Exe
$optionBuild = [bool]$Build
$optionOutputDir = $OutputDir
$optionQuiet = [bool]$Quiet
. (Join-Path $benchDir 'env.ps1')

$repo = Get-BenchRepo -ScriptRoot $PSScriptRoot
$resultsRoot = Join-Path (Join-Path $repo 'tools') 'profiling'
if (-not $optionOutputDir) {
    $outDir = Join-Path $resultsRoot 'results'
} elseif ([System.IO.Path]::IsPathRooted($optionOutputDir)) {
    $outDir = $optionOutputDir
} else {
    $outDir = Join-Path (Get-Location).Path $optionOutputDir
}
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function Write-ProfilingBanner {
    param([string]$Text)
    Write-Host ''
    Write-Host ('== ' + $Text + ' ' + ('=' * [Math]::Max(4, 78 - $Text.Length)))
}

function Save-ProfilingJson {
    param($Object, [string]$Path)
    Write-BenchTextFile -Path $Path -Text ($Object | ConvertTo-Json -Depth 8)
}

function Get-ProfilingCommandInfo {
    param([string]$CargoSubcommand, [string[]]$CargoArguments)
    return [pscustomobject]@{
        captured_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        host            = [System.Environment]::MachineName
        os              = (Get-BenchOsInfo).description
        powershell      = $PSVersionTable.PSVersion.ToString()
        rust            = Get-BenchRustInfo
        git             = Get-BenchGitInfo -Repo $repo
        command         = ('cargo ' + $CargoSubcommand + ' ' + ($CargoArguments -join ' '))
        load            = Get-BenchLoadSnapshot
    }
}

# ---------------------------------------------------------------------------
# doctor
# ---------------------------------------------------------------------------
function Invoke-Doctor {
    Write-ProfilingBanner 'profiling doctor'
    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $checks = New-Object System.Collections.Generic.List[object]
    foreach ($tool in @('wpr', 'wpa', 'xperf', 'tracerpt', 'samply', 'perf', 'perfview', 'procdump', 'cargo-flamegraph', 'dotnet-trace')) {
        $command = Get-Command $tool -ErrorAction SilentlyContinue
        $checks.Add([pscustomobject]@{
            tool    = $tool
            found   = ($null -ne $command)
            path    = if ($null -ne $command) { $command.Source } else { $null }
            note    = ''
        })
    }
    # Windows Performance Toolkit / Visual Studio installs are not always on PATH.
    $installChecks = @(
        @{ tool = 'wpa (WPT)'; path = 'C:\Program Files (x86)\Windows Kits\10\Windows Performance Toolkit\wpa.exe' },
        @{ tool = 'wpa (WPT)'; path = 'C:\Program Files\Windows Kits\10\Windows Performance Toolkit\wpa.exe' },
        @{ tool = 'xperf (WPT)'; path = 'C:\Program Files (x86)\Windows Kits\10\Windows Performance Toolkit\xperf.exe' },
        @{ tool = 'VS 2022 install'; path = 'C:\Program Files (x86)\Microsoft Visual Studio\2022' },
        @{ tool = 'VS 2022 IDE (devenv)'; path = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\devenv.exe' }
    )
    foreach ($entry in $installChecks) {
        if (Test-Path -LiteralPath $entry.path) {
            $note = if ($entry.tool -eq 'VS 2022 install') { 'BuildTools detected; no IDE profiler' } else { 'detected (not on PATH)' }
            $checks.Add([pscustomobject]@{ tool = $entry.tool; found = $true; path = $entry.path; note = $note })
        }
    }
    $binary = $null
    $extension = if (Test-BenchWindows) { '.exe' } else { '' }
    $candidateExe = if ($optionExe) { $optionExe } else { Join-Path (Join-Path (Join-Path $repo 'target') 'release') ('rustibites' + $extension) }
    if (Test-Path -LiteralPath $candidateExe) {
        $binary = Get-Item -LiteralPath $candidateExe
    }
    $pdbPath = if ($null -ne $binary) { [System.IO.Path]::ChangeExtension($binary.FullName, '.pdb') } else { $null }
    $pdbItem = if ($pdbPath -and (Test-Path -LiteralPath $pdbPath)) { Get-Item -LiteralPath $pdbPath } else { $null }
    $report = [ordered]@{
        schema    = 'rustibites.profiling.doctor/1'
        generated = (Get-Date).ToUniversalTime().ToString('o')
        elevated  = $isAdmin
        tools     = $checks
        tracerpt  = ($null -ne (Get-Command tracerpt -ErrorAction SilentlyContinue))
        binary    = if ($null -ne $binary) {
            [ordered]@{
                path        = $binary.FullName
                size_bytes  = $binary.Length
                mtime_utc   = $binary.LastWriteTimeUtc.ToString('o')
                sha256      = (Get-FileHash -LiteralPath $binary.FullName -Algorithm SHA256).Hash
                has_pdb     = ($null -ne $pdbItem)
                pdb_path    = $pdbPath
                pdb_bytes   = if ($null -ne $pdbItem) { $pdbItem.Length } else { $null }
                pdb_mtime_utc = if ($null -ne $pdbItem) { $pdbItem.LastWriteTimeUtc.ToString('o') } else { $null }
                debug_stripped = ((Get-BenchReleaseProfile -Repo $repo).entries.strip -eq '"debuginfo"') -or ((Get-BenchReleaseProfile -Repo $repo).entries.strip -eq 'debuginfo')
                note        = 'strip=debuginfo in Cargo.toml means the default release exe is not symbol-friendly even if a stale .pdb exists; use -Action build-symbols for profiling.'
            }
        } else { $null }
        recommendations = [ordered]@{
            best_available_now = if ($isAdmin) { 'WPR CPU capture + tracerpt CSV (WPA not installed -> limited symbolication)' } else { 'phase audits + bench phase tables (WPR/ETW needs an elevated shell)' }
            needs_admin        = 'Windows Performance Recorder (wpr) kernel ETW sessions'
            needs_install      = @(
                'Windows Performance Toolkit / WPA (Windows SDK installer) for .etl analysis with stacks',
                'cargo install samply (or perf on Linux) for flamegraphs',
                'Visual Studio IDE profiling tools (not present: only BuildTools is installed)',
                'PerfView (Microsoft) as an alternative ETW analyzer'
            )
        }
    }
    Save-ProfilingJson -Object $report -Path (Join-Path $outDir 'doctor.json')
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add('profiling doctor - ' + $report.generated)
    $lines.Add('elevated shell: ' + $isAdmin)
    $lines.Add('')
    $lines.Add('tools:')
    foreach ($check in $checks) {
        $lines.Add(('  {0,-18} {1,-6} {2}' -f $check.tool, $(if ($check.found) { 'found' } else { 'missing' }), $check.path))
    }
    if ($null -ne $report.binary) {
        $lines.Add('')
        $lines.Add('release binary: ' + $report.binary.path)
        $lines.Add('  sha256: ' + $report.binary.sha256)
        $lines.Add('  pdb:    ' + $report.binary.has_pdb + '  (bytes: ' + $report.binary.pdb_bytes + ', mtime: ' + $report.binary.pdb_mtime_utc + ')')
        $lines.Add('  strip=debuginfo in Cargo.toml: ' + $report.binary.debug_stripped + ' -> use -Action build-symbols for a profiling build')
    }
    $lines.Add('')
    $lines.Add('recommended now: ' + $report.recommendations.best_available_now)
    $lines.Add('')
    $lines.Add('notes:')
    $lines.Add('  ETW capture (wpr) requires an elevated shell: ' + (-not $isAdmin))
    $lines.Add('  wpa/xperf/PerfView/samply/procdump are not installed on this machine.')
    Write-BenchTextFile -Path (Join-Path $outDir 'doctor.txt') -Text (($lines -join "`r`n") + "`r`n")
    if (-not $optionQuiet) { foreach ($line in $lines) { Write-Host $line } }
    if (-not $optionQuiet) { Write-Host ''; Write-Host "wrote $outDir\doctor.json and doctor.txt" }
}

# ---------------------------------------------------------------------------
# phase-audit / micro-probes (ignored cargo tests)
# ---------------------------------------------------------------------------
function Invoke-IgnoredTests {
    param([string]$Name, [string]$Filter, [string]$LogName)
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'cargo not found on PATH' }
    Write-ProfilingBanner $Name
    $arguments = @('test', '--release', '-p', 'bibites-core', '--lib', '--', '--ignored', '--nocapture', '--test-threads=1')
    if ($Filter) { $arguments += $Filter }
    $started = (Get-Date).ToUniversalTime()
    Write-Host ('running: cargo ' + ($arguments -join ' '))
    Write-Host 'release test binaries may need a one-time compile (this can take a minute); runtime is bounded by the test bodies.'
    $cargoCommand = Get-Command cargo -ErrorAction Stop
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    # Capture through the shared process helper instead of the PowerShell
    # pipeline: cargo writes progress to stderr, which would otherwise become a
    # terminating ErrorRecord under $ErrorActionPreference='Stop', and this also
    # gives us a hard timeout.
    $processResult = Invoke-BenchProcess -FilePath $cargoCommand.Source -ArgumentList $arguments -TimeoutSec $TestTimeoutSec -SampleMs 200 -WorkingDirectory $repo
    $watch.Stop()
    $exitCode = $processResult.ExitCode
    if ($processResult.TimedOut) {
        $exitCode = 'timeout'
    }
    $text = ($processResult.StdOut + "`r`n" + $processResult.StdErr).Trim()
    Write-BenchTextFile -Path (Join-Path $outDir $LogName) -Text ($text + "`r`n")
    $meta = [ordered]@{
        schema           = 'rustibites.profiling.audit/1'
        action           = $Name
        started_at_utc   = $started.ToString('o')
        duration_secs    = [math]::Round($watch.Elapsed.TotalSeconds, 3)
        exit_code        = $exitCode
        timed_out        = $processResult.TimedOut
        log              = $LogName
        command          = ('cargo ' + ($arguments -join ' '))
        test_filter      = $Filter
        git              = Get-BenchGitInfo -Repo $repo
        rust             = Get-BenchRustInfo
        load_at_run      = Get-BenchLoadSnapshot
    }
    Save-ProfilingJson -Object $meta -Path (Join-Path $outDir ($LogName -replace '\.log$', '.meta.json'))
    if (-not $optionQuiet) {
        Write-Host ''
        Write-Host "exit=$exitCode duration=$([math]::Round($watch.Elapsed.TotalSeconds,1))s log=$outDir\$LogName"
        foreach ($line in (($text -split "`r?`n") | Select-Object -Last 12)) { Write-Host $line }
    }
    if ($processResult.TimedOut) { throw "$Name timed out after $TestTimeoutSec s (see $outDir\$LogName)" }
    if ($exitCode -ne 0) { throw "$Name failed with exit code $exitCode (see $outDir\$LogName)" }
}

# ---------------------------------------------------------------------------
# bench-phases (delegate to tools/bench)
# ---------------------------------------------------------------------------
function Invoke-BenchPhases {
    Write-ProfilingBanner 'bench phase tables'
    $runner = Join-Path $benchDir 'run_phases.ps1'
    if (-not (Test-Path -LiteralPath $runner)) { throw "missing $runner (tools/bench harness)" }
    $phaseOut = Join-Path $outDir 'bench-phases'
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $runner, '-Pops', $Pops, '-Threads', $Threads, '-Ticks', [string]$Ticks, '-OutputDir', $phaseOut)
    if ($optionExe) { $arguments += @('-Exe', $optionExe) }
    if ($Scenario) { $arguments += @('-Scenario', $Scenario) }
    if ($optionQuiet) { $arguments += '-Quiet' }
    Write-Host ('running: powershell ' + ($arguments -join ' '))
    & powershell @arguments
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) { throw "bench phase capture failed with exit code $exitCode" }
    if (-not $optionQuiet) { Write-Host ''; Write-Host "wrote $phaseOut\phases.csv, phases.md, phases_raw.txt" }
}

# ---------------------------------------------------------------------------
# build-symbols (debug info without editing Cargo.toml)
# ---------------------------------------------------------------------------
function Invoke-BuildSymbols {
    Write-ProfilingBanner 'release build with debug symbols'
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'cargo not found on PATH' }
    $extension = if (Test-BenchWindows) { '.exe' } else { '' }
    $target = if ($TargetDir) {
        if ([System.IO.Path]::IsPathRooted($TargetDir)) { $TargetDir } else { Join-Path $repo $TargetDir }
    } else {
        Join-Path (Join-Path $repo 'target') 'profiling'
    }
    $exePath = Join-Path (Join-Path $target 'release') ('rustibites' + $extension)
    $pdbPath = [System.IO.Path]::ChangeExtension($exePath, '.pdb')
    if (-not $Force -and (Test-Path -LiteralPath $exePath)) {
        Write-Host "symbol build already present: $exePath (pass -Force to rebuild)"
    } else {
        $arguments = @(
            'build', '--release', '-p', 'bibites-cli',
            '--target-dir', $target,
            '--config', 'profile.release.debug=true',
            '--config', 'profile.release.strip=false'
        )
        Write-Host ('running: cargo ' + ($arguments -join ' '))
        Write-Host 'note: this is an isolated target dir (target/profiling), so target/release stays untouched; expect a full LTO build (~2-4 min).'
        # Capture through the shared process helper: cargo writes progress to
        # stderr, which would otherwise become a terminating ErrorRecord under
        # $ErrorActionPreference='Stop'.
        $cargoCommand = Get-Command cargo -ErrorAction Stop
        $processResult = Invoke-BenchProcess -FilePath $cargoCommand.Source -ArgumentList $arguments -TimeoutSec 1800 -SampleMs 500 -WorkingDirectory $repo
        $buildText = ($processResult.StdOut + "`r`n" + $processResult.StdErr).Trim()
        foreach ($line in (($buildText -split "`r?`n") | Select-Object -Last 8)) { Write-Host $line }
        if ($processResult.TimedOut) { throw 'cargo build timed out after 1800 s' }
        if ($processResult.ExitCode -ne 0) { throw "cargo build failed with exit code $($processResult.ExitCode)" }
    }
    $binary = Get-Item -LiteralPath $exePath -ErrorAction SilentlyContinue
    if ($null -eq $binary) { throw "symbol build did not produce $exePath" }
    $info = [ordered]@{
        schema          = 'rustibites.profiling.symbol-build/1'
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        exe_path        = $binary.FullName
        exe_bytes       = $binary.Length
        exe_mtime_utc   = $binary.LastWriteTimeUtc.ToString('o')
        exe_sha256      = (Get-FileHash -LiteralPath $binary.FullName -Algorithm SHA256).Hash
        pdb_path        = if (Test-Path -LiteralPath $pdbPath) { $pdbPath } else { $null }
        pdb_bytes       = if (Test-Path -LiteralPath $pdbPath) { (Get-Item -LiteralPath $pdbPath).Length } else { $null }
        target_dir      = $target
        overrides       = @('profile.release.debug=true', 'profile.release.strip=false')
        command         = ('cargo build --release -p bibites-cli --target-dir ' + $target + ' --config profile.release.debug=true --config profile.release.strip=false')
        note            = 'root Cargo.toml is unchanged; these are cargo --config overrides. A full symbolised symbol server is not set up: PDB next to the exe is what WPA/VS/Windbg consume.'
    }
    Save-ProfilingJson -Object $info -Path (Join-Path $outDir 'symbol-build.json')
    if (-not $optionQuiet) {
        Write-Host ''
        Write-Host "symbolised binary: $($info.exe_path)"
        Write-Host ('  pdb: ' + $(if ($info.pdb_path) { $info.pdb_path + ' (' + $info.pdb_bytes + ' bytes)' } else { 'NOT FOUND (MSVC usually emits one next to the exe; check target dir)' }))
        Write-Host ('  sha256: ' + $info.exe_sha256)
        Write-Host "wrote $outDir\symbol-build.json"
    }
}

# ---------------------------------------------------------------------------
# etw-capture (WPR; requires admin)
# ---------------------------------------------------------------------------
function Invoke-EtwCapture {
    Write-ProfilingBanner 'ETW CPU capture (Windows Performance Recorder)'
    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not (Get-Command wpr -ErrorAction SilentlyContinue)) {
        Write-Host 'wpr not found; install the Windows Performance Toolkit or use the phase-audit workflow.'
        exit 3
    }
    if (-not $isAdmin) {
        $message = @(
            'WPR CPU capture needs an elevated shell (kernel ETW session).',
            'This session is NOT elevated, so no trace can be started (error 0xc5585011).',
            '',
            'Run this action from an elevated PowerShell:',
            ('  powershell -NoProfile -ExecutionPolicy Bypass -File ' + $PSCommandPath + ' -Action etw-capture -Pops 2000 -Ticks 400'),
            '',
            'Then open the .etl in Windows Performance Analyzer (Windows Performance',
            'Toolkit from the Windows SDK, not installed here) or PerfView. The',
            'tracerpt CSV conversion is written by this action for grep-level inspection.'
        )
        Write-BenchTextFile -Path (Join-Path $outDir 'etw-capture-blocked.txt') -Text (($message -join "`r`n") + "`r`n")
        foreach ($line in $message) { Write-Host $line }
        exit 3
    }
    $extension = if (Test-BenchWindows) { '.exe' } else { '' }
    $exePath = if ($optionExe) { $optionExe } else { Join-Path (Join-Path (Join-Path $repo 'target') 'release') ('rustibites' + $extension) }
    if (-not (Test-Path -LiteralPath $exePath)) { throw "benchmark binary not found: $exePath (build it, or pass -Build)" }
    $etl = Join-Path $outDir ('rustibites-' + (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss') + '.etl')
    $csv = [System.IO.Path]::ChangeExtension($etl, '.csv')
    $arguments = @('bench', '--bibites', [string]$Pops, '--ticks', [string]$Ticks, '--threads', [string]$Threads, '--repeat', '1', '--json')
    Write-Host ("starting WPR profile '$WprProfile' -> $etl")
    & wpr -start $WprProfile -filemode 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'wpr -start failed (see output above; elevated shell required)' }
    try {
        Write-Host ('capturing up to ' + $CaptureSeconds + ' s while running: ' + $exePath + ' ' + ($arguments -join ' '))
        $processArgs = @{
            FilePath     = $exePath
            ArgumentList = $arguments
            TimeoutSec   = [Math]::Max($CaptureSeconds + 30, 60)
            SampleMs     = 50
        }
        $result = Invoke-BenchProcess @processArgs
        Start-Sleep -Milliseconds 500
        if ($result.StdOut) { Write-Host ($result.StdOut.Trim()) }
    } finally {
        Write-Host 'stopping WPR'
        & wpr -stop $etl 2>&1 | Out-Host
    }
    if (-not (Test-Path -LiteralPath $etl)) { throw "WPR did not produce $etl" }
    if ($ConvertWithTracerpt -or (Get-Command tracerpt -ErrorAction SilentlyContinue)) {
        try {
            Write-Host ('converting with tracerpt -> ' + $csv)
            & tracerpt $etl -o $csv -of CSV -y 2>&1 | Select-Object -Last 5 | Out-Host
        } catch {
            Write-Warning ('tracerpt conversion failed: ' + $_.Exception.Message)
        }
    }
    $meta = [ordered]@{
        schema        = 'rustibites.profiling.etw/1'
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        elevated      = $isAdmin
        wpr_profile   = $WprProfile
        etl_path      = $etl
        etl_bytes     = (Get-Item -LiteralPath $etl).Length
        csv_path      = if (Test-Path -LiteralPath $csv) { $csv } else { $null }
        bench_command = ($exePath + ' ' + ($arguments -join ' '))
        next_steps    = @(
            'Open the .etl in Windows Performance Analyzer (WPT) or PerfView and use the CPU Usage (Sampled) view.',
            'The tracerpt CSV is a flat event conversion for grep-level inspection; it is not a flamegraph.',
            'If WPA is unavailable, install Windows Performance Toolkit from the Windows SDK (requires approval), or use samply/cargo-flamegraph on Linux.'
        )
    }
    Save-ProfilingJson -Object $meta -Path (Join-Path $outDir 'etw-capture.json')
    if (-not $optionQuiet) {
        Write-Host ''
        Write-Host "etl: $etl ($($meta.etl_bytes) bytes)"
        if ($meta.csv_path) { Write-Host "csv: $($meta.csv_path)" }
        Write-Host "metadata: $outDir\etw-capture.json"
    }
}

# ---------------------------------------------------------------------------
# dispatch
# ---------------------------------------------------------------------------
switch ($Action) {
    'doctor' { Invoke-Doctor }
    'phase-audit' { Invoke-IgnoredTests -Name 'phase-audit' -Filter 'perf_' -LogName 'phase_audit.log' }
    'micro-probes' {
        Invoke-IgnoredTests -Name 'micro-probes' -Filter 'perf_probe_' -LogName 'micro_probes_body.log'
        Invoke-IgnoredTests -Name 'micro-probes-executor' -Filter 'perf_parallel_executor_overhead' -LogName 'micro_probes_executor.log'
        Invoke-IgnoredTests -Name 'micro-probes-sensing' -Filter 'perf_sensing_' -LogName 'micro_probes_sensing.log'
    }
    'bench-phases' { Invoke-BenchPhases }
    'build-symbols' { Invoke-BuildSymbols }
    'etw-capture' { Invoke-EtwCapture }
}
