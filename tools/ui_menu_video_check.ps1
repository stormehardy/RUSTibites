# Checks the animated title background: starts the viewer's title menu, takes window screenshots a
# couple of seconds apart and reports whether they differ (the background video plays), plus the
# ffmpeg decoder process and the viewer's CPU / memory.
# Usage: powershell -File tools\ui_menu_video_check.ps1 -Exe target_light\release\rustibites-view.exe -Out $env:TEMP\menu_video
# It makes the window topmost; do not use the mouse while it runs.
param(
    [string]$Exe = 'target_light\release\rustibites-view.exe',
    [string]$Out = "$env:TEMP\menu_video"
)
. "$PSScriptRoot\ui_drive.ps1"
New-Item -ItemType Directory -Force $Out | Out-Null
$exe = (Resolve-Path $Exe).Path
$p = Start-Process $exe -ArgumentList '--menu' -PassThru
for ($i = 0; $i -lt 120; $i++) {
    Start-Sleep -Milliseconds 500
    $p.Refresh()
    if ($p.MainWindowHandle -ne 0) { break }
}
Start-Sleep 8
try {
    Focus-Window $p
    for ($i = 0; $i -lt 4; $i++) {
        Shot-Window $p (Join-Path $Out "frame_$i.png")
        Start-Sleep -Milliseconds 1700
    }
    $ffmpeg = Get-Process ffmpeg -ErrorAction SilentlyContinue
    $p.Refresh()
    "viewer alive=$(-not $p.HasExited) mem=$([int]($p.WorkingSet64/1MB))MB cpu=$([int]$p.CPU)s; ffmpeg processes: $(@($ffmpeg).Count)"
}
finally { Stop-Viewer $p }
