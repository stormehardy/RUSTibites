# Creates Windows shortcuts for RUSTibites with the red-tinted project icon.
#
#   powershell -ExecutionPolicy Bypass -File scripts\make_shortcut.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\make_shortcut.ps1 -DesktopOnly
#
# The main game shortcut launches the viewer with no path, which opens the
# in-game main menu (New Game / Continue / Load / Settings / Quit). A separate
# console shortcut opens a cmd window with the headless CLI help. The folder
# shortcut opens the project. Re-run after moving the checkout or rebuilding.

param(
    [switch]$DesktopOnly
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$icon = Join-Path $root "assets\icon\rustibites.ico"
$viewer = Join-Path $root "target\release\rustibites-view.exe"
$exe = Join-Path $root "target\release\rustibites.exe"
$desktop = [Environment]::GetFolderPath("Desktop")

if (-not (Test-Path $icon)) {
    throw "icon not found: $icon (run scripts\make_project_icon.py first)"
}
if (-not (Test-Path $viewer)) {
    Write-Host "viewer binary not found at $viewer; building it now..."
    Push-Location $root
    try {
        cargo build --release -p bibites-view
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        Pop-Location
    }
}

$shell = New-Object -ComObject WScript.Shell

# Main game shortcut: launches the viewer with no path, showing the in-game
# main menu instead of a generated world.
$gameShortcut = Join-Path $desktop "RUSTibites.lnk"
$shortcut = $shell.CreateShortcut($gameShortcut)
$shortcut.TargetPath = $viewer
$shortcut.Arguments = ""
$shortcut.WorkingDirectory = $root
$shortcut.IconLocation = "$icon,0"
$shortcut.Description = "RUSTibites - play The Bibites, rebuilt in Rust"
$shortcut.Save()
Write-Host "created $gameShortcut -> $viewer"

# Console shortcut: cmd in the workspace with the headless CLI help shown.
$consoleShortcut = Join-Path $desktop "RUSTibites Console.lnk"
$consoleArgs = '/k cd /d "' + $root + '" && target\release\rustibites.exe --help'
$shortcut = $shell.CreateShortcut($consoleShortcut)
$shortcut.TargetPath = "$env:ComSpec"
$shortcut.Arguments = $consoleArgs
$shortcut.WorkingDirectory = $root
$shortcut.IconLocation = "$icon,0"
$shortcut.Description = "RUSTibites - headless high-speed Bibites simulation CLI"
$shortcut.Save()
Write-Host "created $consoleShortcut"

if (-not $DesktopOnly) {
    $folderShortcut = Join-Path $desktop "RUSTibites Project.lnk"
    $shortcut = $shell.CreateShortcut($folderShortcut)
    $shortcut.TargetPath = $root
    $shortcut.IconLocation = "$icon,0"
    $shortcut.Description = "RUSTibites project workspace"
    $shortcut.Save()
    Write-Host "created $folderShortcut"
}
