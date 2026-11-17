<#
.SYNOPSIS
    Captures reproducible environment metadata for RUSTibites benchmarks.

.DESCRIPTION
    Records everything needed to interpret (and reproduce) a benchmark run:
    OS, CPU, memory, rustc/cargo versions, git commit and dirty state, the
    workspace release profile, and the SHA-256 of the measured binary.

    Standalone use:

        pwsh -File tools/bench/env.ps1
        pwsh -File tools/bench/env.ps1 -OutputDir tools/bench/results/environment
        pwsh -File tools/bench/env.ps1 -Json

    It is also dot-sourced by tools/bench/run_matrix.ps1 and
    tools/bench/run_phases.ps1, which call the functions below directly.

    `-Build` rebuilds the release CLI first; by default an existing
    target/release binary is used (shared machines should not force rebuilds).

.PARAMETER Repo
    Repository root. Defaults to the repo containing this script.

.PARAMETER Exe
    Explicit CLI binary to fingerprint. Defaults to target/release/rustibites[.exe].

.PARAMETER Build
    Run `cargo build --release -p bibites-cli` before capturing.

.PARAMETER OutputDir
    Write environment.json + environment.txt here.

.PARAMETER Json
    Print the environment as JSON instead of human-readable text.
#>
[CmdletBinding()]
param(
    [string]$Repo = '',
    [string]$Exe = '',
    [switch]$Build,
    [string]$OutputDir = '',
    [switch]$Json,
    [switch]$Quiet
)

. (Join-Path $PSScriptRoot 'bench_common.ps1')

function Invoke-BenchEnvMain {
    $repoPath = if ($Repo) { (Resolve-Path -LiteralPath $Repo).Path } else { Get-BenchRepo -ScriptRoot $PSScriptRoot }

    $executable = $null
    $extension = if (Test-BenchWindows) { '.exe' } else { '' }
    $candidate = if ($Exe) { $Exe } else { Join-Path (Join-Path (Join-Path $repoPath 'target') 'release') ('rustibites' + $extension) }
    if ($Build -or (Test-Path -LiteralPath $candidate)) {
        # -NoCopy: a metadata-only capture must not leave temp binaries behind.
        $executable = Resolve-BenchExecutable -Repo $repoPath -Exe $candidate -Build:$Build -NoCopy
    } elseif (-not $Json -and -not $Quiet) {
        Write-Warning "benchmark binary not found: $candidate (run: cargo build --release -p bibites-cli; environment captured without binary metadata)"
    }

    $environment = Get-BenchEnvironment -Repo $repoPath -Executable $executable
    if ($OutputDir) {
        Write-BenchEnvironment -Environment $environment -OutputDir $OutputDir
        if (-not $Quiet) { Write-Host "wrote environment metadata to $OutputDir" }
    }
    if ($Json) {
        return ($environment | ConvertTo-Json -Depth 8)
    }
    return (Format-BenchEnvironmentText $environment)
}

# Dot-sourcing (`. tools/bench/env.ps1`) must define functions only; direct execution prints/writes.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-BenchEnvMain
}
