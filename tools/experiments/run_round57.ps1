# run_round57.ps1 - bounded ecology validation batch (task-57).
# Runs the release engine through tools/experiments/run_ecology.py on:
#   * Ring scenario fresh start (20k ticks)
#   * the Ring parity base save (5k ticks)
#   * the real lol save (5k ticks)
#   * dense founder populations 1000/2000/5000 at 10k ticks (brain-dev collapse question)
# Outputs under testdata/experiments/<name>/ (series.csv, summary.json, plots/, final.zip).
# The engine path can be overridden: -Engine C:\...\rustibites.exe
param(
    [string]$Engine = "Z:\RUSTibites\target\eco-target\release\rustibites.exe",
    [int]$Threads = 8,
    [int]$Seed = 12345
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Push-Location $root
try {
    if (-not (Test-Path $Engine)) { throw "engine not found: $Engine" }
    $runs = @(
        @{ input = "testdata\real\Ring.zip"; name = "ring_scenario_20k"; ticks = 20000 },
        @{ input = "testdata\parity\bases\ring_parity.zip"; name = "ring_base_5k"; ticks = 5000 },
        @{ input = "testdata\real\saves\lol.zip"; name = "lol_save_5k"; ticks = 5000 },
        @{ input = "testdata\experiments\dense\founders_1000.zip"; name = "dense_1000_10k"; ticks = 10000 },
        @{ input = "testdata\experiments\dense\founders_2000.zip"; name = "dense_2000_10k"; ticks = 10000 },
        @{ input = "testdata\experiments\dense\founders_5000.zip"; name = "dense_5000_10k"; ticks = 10000 }
    )
    foreach ($r in $runs) {
        if (-not (Test-Path $r.input)) { Write-Warning "skip $($r.name): missing $($r.input)"; continue }
        python tools\experiments\run_ecology.py --input $r.input --name $r.name --ticks $r.ticks `
            --seed $Seed --threads $Threads --engine $Engine
        if ($LASTEXITCODE -ne 0) { throw "run_ecology failed for $($r.name) (exit $LASTEXITCODE)" }
    }
} finally {
    Pop-Location
}
