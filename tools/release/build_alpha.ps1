# Builds the tester package: a zip people unzip and run (Play.bat). It holds NO game art: the first run builds it from the player's own copy of
# The Bibites with extract_art.exe.
#
#   pwsh -File tools/release/build_alpha.ps1 [-Exe target_perf\perf\rustibites-view.exe] [-Name RUSTibites-alpha-1]
#
# Builds the viewer unless -Exe is given. Needs `pip install pyinstaller UnityPy pillow` (once; the extractor tool is frozen with it).
[CmdletBinding()]
param(
    [string]$Exe = '',
    [string]$Name = '',
    [string]$OutputRoot = 'target\alpha-package',
    [switch]$SkipFreeze
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Push-Location $repo
try {
    $commit = (git rev-parse --short HEAD 2>$null); if (-not $commit) { $commit = 'unknown' }
    $date = Get-Date -Format 'yyyy-MM-dd'
    if (-not $Name) { $Name = "RUSTibites-Improved-alpha-$date-$commit" }   # Faithful (main): "RUSTibites-alpha-..."
    if (-not $Exe) {
        # Optimised, and with the C runtime inside the exe so a clean machine needs no Visual C++ redistributable.
        Write-Host '==> building the viewer (perf profile, static CRT) ...'
        $env:RUSTFLAGS = '-C target-feature=+crt-static'
        $env:CARGO_INCREMENTAL = '0'
        cargo build --profile perf -p bibites-view --target-dir target_alpha
        if ($LASTEXITCODE -ne 0) { throw 'viewer build failed' }
        Remove-Item Env:RUSTFLAGS
        $Exe = 'target_alpha\perf\rustibites-view.exe'
    }
    if (-not (Test-Path $Exe)) { throw "missing $Exe" }

    $stage = Join-Path $repo "$OutputRoot\$Name"
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $stage | Out-Null

    Copy-Item $Exe (Join-Path $stage 'RUSTibites.exe')
    foreach ($f in 'Play.bat', 'Play-with-my-game-saves.bat', 'Make-Bug-Report.bat', 'play.ps1', 'report.ps1', 'README.txt') {
        Copy-Item (Join-Path $PSScriptRoot "alpha\$f") $stage
    }
    # The Improved edition's own files for testers (Faithful has none): the note on what is new and where its files are, and the check that ffmpeg is there.
    foreach ($f in 'IMPROVED.txt', 'ensure_ffmpeg.ps1') {
        $improved = Join-Path $PSScriptRoot "alpha\$f"
        if (Test-Path $improved) { Copy-Item $improved $stage }
    }
    foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE') { Copy-Item $f $stage }
    Copy-Item (Join-Path $PSScriptRoot 'art_map.json') $stage
    "$Name`r`ncommit $commit`r`nbuilt $date" | Set-Content (Join-Path $stage 'VERSION.txt')

    # The extractor: the repo's own extract.py plus the wrapper, frozen with Python and UnityPy inside.
    if (-not $SkipFreeze) {
        $work = Join-Path $repo 'target\pyi'
        Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force $work | Out-Null
        Push-Location $work
        # A clean virtual environment keeps the frozen tool small (a system Python drags everything installed into it along).
        $venv = Join-Path $repo 'target\venv'
        if (-not (Test-Path (Join-Path $venv 'Scripts\python.exe'))) {
            python -m venv $venv
            & (Join-Path $venv 'Scripts\python.exe') -m pip install -q UnityPy==1.25.4 pillow pyinstaller
        }
        & (Join-Path $venv 'Scripts\python.exe') -m PyInstaller --onefile --name extract_art --paths (Join-Path $repo 'tools\assets') --collect-all UnityPy --collect-all texture2ddecoder --collect-all etcpak --collect-all astc_encoder --collect-all brotli --collect-all fmod_toolkit --collect-all pyfmodex --collect-all archspec --exclude-module tkinter --exclude-module matplotlib --hidden-import extract (Join-Path $repo 'tools\release\extract_art.py')
        Pop-Location
    }
    Copy-Item (Join-Path $repo 'target\pyi\dist\extract_art.exe') $stage

    # The data the viewer reads that is ours (layouts, metadata); pictures are built on the first run.
    $src = Join-Path $repo 'assets\game'
    $dst = Join-Path $stage 'assets\game'
    New-Item -ItemType Directory -Force $dst | Out-Null
    foreach ($f in 'LICENSE-ASSETS.txt', 'README.md', 'manifest.json', 'prefabs.json', 'coverage.json', 'coverage_sprite_cells.json', 'ui_runtime.index.json', 'ui_runtime_native.index.json', 'ui_runtime_native.json.gz') {
        Copy-Item (Join-Path $src $f) $dst
    }
    foreach ($d in 'materials', 'tail', 'text', 'ui_runtime_native') { Copy-Item -Recurse (Join-Path $src $d) $dst }
    New-Item -ItemType Directory -Force (Join-Path $dst 'ui_fonts') | Out-Null
    Copy-Item (Join-Path $src 'ui_fonts\*.json') (Join-Path $dst 'ui_fonts')
    $ref = Join-Path $dst 'reference\functional_ui_native'
    New-Item -ItemType Directory -Force $ref | Out-Null
    Copy-Item (Join-Path $src 'reference\functional_ui_native\*.json') $ref
    New-Item -ItemType Directory -Force (Join-Path $dst 'reference\default_save') | Out-Null
    Copy-Item (Join-Path $src 'reference\default_save\ui_save_capture.json') (Join-Path $dst 'reference\default_save')

    # The UI atlas index (the viewer looks for tools/ui/ui_atlas.json next to the exe; without it the title menu will not start).
    New-Item -ItemType Directory -Force (Join-Path $stage 'tools\ui') | Out-Null
    Copy-Item (Join-Path $repo 'tools\ui\ui_atlas.json') (Join-Path $stage 'tools\ui')

    New-Item -ItemType Directory -Force (Join-Path $stage 'assets\icon') | Out-Null
    Copy-Item (Join-Path $repo 'assets\icon\rustibites_256.png') (Join-Path $stage 'assets\icon')

    # What is known not to match yet.
    $gaps = Get-Content (Join-Path $repo 'docs\PARITY_REMAINING.md') -Raw
    $m = [regex]::Match($gaps, '(?s)## Open items as of.*?(?=\r?\n## )')
    if ($m.Success) { $m.Value | Set-Content (Join-Path $stage 'KNOWN_GAPS.txt') }

    $zip = Join-Path $repo "$OutputRoot\$Name.zip"
    Remove-Item -Force $zip -ErrorAction SilentlyContinue
    Compress-Archive -Path $stage -DestinationPath $zip
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "Package: $zip ($mb MB)"
    (Get-FileHash $zip -Algorithm SHA256).Hash | Set-Content (Join-Path $repo "$OutputRoot\$Name.sha256")
}
finally { Pop-Location }
