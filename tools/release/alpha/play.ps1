# RUSTibites alpha launcher.
param([switch]$GameSaves, [switch]$Windowed, [string]$GameDir = '', [string[]]$ViewerArgs = @())
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
$stamp = Join-Path $PSScriptRoot 'assets\game\.art_extracted.json'
$logs = Join-Path $PSScriptRoot 'logs'
New-Item -ItemType Directory -Force $logs | Out-Null

function Pick-GameFolder {
    Add-Type -AssemblyName System.Windows.Forms
    $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
    $dialog.Description = "Select the folder of The Bibites (the one that contains 'The Bibites.exe')"
    if ($dialog.ShowDialog() -eq 'OK') { return $dialog.SelectedPath }
    return ''
}

# 1. The art (first run, or after the game was moved).
if (-not (Test-Path $stamp)) {
    Write-Host 'First run: building the game art from your copy of The Bibites...'
    $extra = @()
    if ($GameDir) { $extra = @('--game', $GameDir) }
    & "$PSScriptRoot\extract_art.exe" @extra
    if ($LASTEXITCODE -eq 2) {
        $picked = Pick-GameFolder
        if (-not $picked) { Write-Host 'No folder chosen. Nothing was changed.'; exit 1 }
        & "$PSScriptRoot\extract_art.exe" --game $picked
    }
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $stamp)) {
        Write-Host 'The art could not be built. Please send logs\ and a note to the testers channel (see README.txt).'
        exit 1
    }
}
$game = (Get-Content $stamp -Raw | ConvertFrom-Json).game
$env:RUSTIBITES_GAME_DIR = $game

# 2. Where the worlds go: a private folder by default.
if (-not $GameSaves) {
    $data = Join-Path $PSScriptRoot 'userdata'
    New-Item -ItemType Directory -Force $data | Out-Null
    $env:RUSTIBITES_DATA_DIR = $data
    $settings = Join-Path $data 'game_user_settings.json'
    if (-not (Test-Path $settings)) {
        # The game starts fullscreen; an alpha is easier to report from a window.
        Set-Content -Path $settings -Value '{"version":2,"values":{"Fullscreen":false}}' -Encoding ascii
    }
}

# 3. Run, keeping the log of this run and the one before.
$log = Join-Path $logs 'last_run.log'
if (Test-Path $log) { Move-Item -Force $log (Join-Path $logs 'previous_run.log') }
$version = if (Test-Path "$PSScriptRoot\VERSION.txt") { (Get-Content "$PSScriptRoot\VERSION.txt" -Raw).Trim() } else { 'unknown' }
"RUSTibites alpha $version`r`nstarted $(Get-Date -Format s)`r`ngame $game`r`n" | Set-Content $log
$err = Join-Path $logs 'stderr.tmp'
$startArgs = @{ FilePath = "$PSScriptRoot\RUSTibites.exe"; PassThru = $true; Wait = $true; RedirectStandardError = $err; WorkingDirectory = $PSScriptRoot }
# From a .bat, "-ViewerArgs --a,--b" and "-ViewerArgs '--a --b'" arrive as ONE string: a flag entry holds one flag or several, split at commas and spaces.
$ViewerArgs = @($ViewerArgs | ForEach-Object { if ($_ -like '--*') { $_ -split '[,\s]+' } else { $_ } } | Where-Object { $_ })
if ($ViewerArgs.Count -gt 0) { $startArgs.ArgumentList = $ViewerArgs }   # an empty ArgumentList is an error in Windows PowerShell
$p = Start-Process @startArgs
if (Test-Path $err) { Get-Content $err | Add-Content $log; Remove-Item $err }
$hex = if ($p.ExitCode -lt 0 -or $p.ExitCode -gt 255) { ' (0x{0:X8})' -f ([int64]$p.ExitCode -band 0xFFFFFFFFL) } else { '' }
"exit code $($p.ExitCode)$hex" | Add-Content $log
if ($p.ExitCode -ne 0) {
    # What Windows recorded about the crash (the module that failed tells a graphics-driver crash from ours); the record is written a moment after the exit.
    # Nothing here may stop the launcher from telling the user what happened.
    try {
        Start-Sleep -Seconds 2
        $crash = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; ProviderName = 'Application Error'; StartTime = (Get-Date).AddMinutes(-5) } -ErrorAction SilentlyContinue |
            Where-Object { $_.Properties.Count -ge 8 -and "$($_.Properties[0].Value)" -match 'RUSTibites' } | Select-Object -First 1
        if ($crash) {
            "windows crash record: faulting module $($crash.Properties[3].Value) $($crash.Properties[4].Value), exception $($crash.Properties[6].Value), offset $($crash.Properties[7].Value)" | Add-Content $log
        }
    } catch { }
    Write-Host "RUSTibites stopped with exit code $($p.ExitCode)$hex. The log is $log - please run Make-Bug-Report.bat and send the zip."
    exit 1
}
