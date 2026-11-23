# RUSTibites Improved: makes sure ffmpeg is there for the title menu's moving background (the original game's own video, played through ffmpeg).
# play.ps1 runs this before it starts the game. When ffmpeg is already there it does nothing at all. When it is not, it installs ffmpeg ONCE with winget
# (Windows' own package manager; the "essentials" build of ffmpeg, a small download, installed for the current user: no administrator rights) and says so on
# the screen. Whatever happens, the game starts afterwards: without ffmpeg the main screen is just a plain dark backdrop.
#
# It does not try again on every start: what it did is written to ffmpeg_install.txt next to this script (a failed try is repeated after a day, a missing winget
# after a month). To stop it altogether: start Play.bat with -NoFfmpegInstall, or make an empty file called no_ffmpeg_install.txt next to Play.bat, or set the
# environment variable RUSTIBITES_NO_FFMPEG_INSTALL to anything.
param([string]$PackageDir = $PSScriptRoot)
$ErrorActionPreference = 'Continue'

# The places the viewer looks for ffmpeg (crates/bibites-view/src/menu_video.rs): RUSTIBITES_FFMPEG, the PATH, C:\ffmpeg\bin, Program Files, and winget's packages.
function Find-Ffmpeg {
    if ($env:RUSTIBITES_FFMPEG -and (Test-Path -LiteralPath $env:RUSTIBITES_FFMPEG -PathType Leaf)) { return $env:RUSTIBITES_FFMPEG }
    $onPath = Get-Command ffmpeg.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($onPath) { return $onPath.Source }
    foreach ($dir in 'C:\ffmpeg\bin', 'C:\Program Files\ffmpeg\bin') {
        if (Test-Path -LiteralPath (Join-Path $dir 'ffmpeg.exe')) { return (Join-Path $dir 'ffmpeg.exe') }
    }
    if ($env:LOCALAPPDATA) {
        $packages = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages'
        if (Test-Path -LiteralPath $packages) {
            foreach ($package in Get-ChildItem -LiteralPath $packages -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -like '*FFmpeg*' }) {
                foreach ($build in Get-ChildItem -LiteralPath $package.FullName -Directory -ErrorAction SilentlyContinue) {
                    $candidate = Join-Path $build.FullName 'bin\ffmpeg.exe'
                    if (Test-Path -LiteralPath $candidate) { return $candidate }
                }
            }
        }
    }
    return $null
}

if (Find-Ffmpeg) { return }
if ($env:RUSTIBITES_NO_FFMPEG_INSTALL -or (Test-Path -LiteralPath (Join-Path $PackageDir 'no_ffmpeg_install.txt'))) { return }

# What happened last time: "<when> <outcome>" (installed | failed | no-winget).
$note = Join-Path $PackageDir 'ffmpeg_install.txt'
if (Test-Path -LiteralPath $note) {
    $last = (Get-Content -LiteralPath $note -Raw -ErrorAction SilentlyContinue)
    if ($last -match '^(?<when>\S+)\s+(?<what>\S+)') {
        $when = [datetime]::MinValue
        if ([datetime]::TryParse($Matches['when'], [ref]$when)) {
            $wait = if ($Matches['what'] -eq 'no-winget') { [timespan]::FromDays(30) } else { [timespan]::FromDays(1) }
            if (((Get-Date) - $when) -lt $wait) { return }
        }
    }
}
function Write-Note($what) { try { "$(Get-Date -Format s) $what" | Set-Content -LiteralPath $note -Encoding ascii } catch { } }

$winget = Get-Command winget -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $winget) {
    Write-Host 'Note: ffmpeg is not installed, so the main screen has a plain dark background instead of the game''s moving one.'
    Write-Host '      winget (Windows'' package manager) was not found to install it for you; see "A DARK MAIN SCREEN?" in IMPROVED.txt to do it by hand.'
    Write-Note 'no-winget'
    return
}

Write-Host 'ffmpeg (a free video tool) is not installed. The main screen''s moving background needs it, so it is being installed now, once, with winget:'
Write-Host '  winget install Gyan.FFmpeg.Essentials   (a small download, installed for you only, no administrator rights; to stop this, see IMPROVED.txt)'
$arguments = @('install', '--id', 'Gyan.FFmpeg.Essentials', '--exact', '--silent', '--accept-source-agreements', '--accept-package-agreements')
try {
    $process = Start-Process -FilePath $winget.Source -ArgumentList $arguments -NoNewWindow -PassThru
    $null = $process.Handle
    if (-not $process.WaitForExit(600000)) {
        try { $process.Kill() } catch { }
        Write-Host 'The installation took more than ten minutes and was stopped.'
    }
} catch {
    Write-Host "The installation could not be started: $($_.Exception.Message)"
}

if (Find-Ffmpeg) {
    Write-Host 'ffmpeg is installed: the main screen will have its moving background.'
    Write-Note 'installed'
} else {
    Write-Host 'ffmpeg could not be installed, so the main screen has a plain dark background (the game works the same). It will be tried again in a day.'
    Write-Host 'To do it by hand, see "A DARK MAIN SCREEN?" in IMPROVED.txt.'
    Write-Note 'failed'
}
