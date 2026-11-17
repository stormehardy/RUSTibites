# Walks the title menu with real mouse/keyboard input and saves a screenshot per step.
# Usage: powershell -File tools\ui_menu_tour.ps1 -Exe target_light\release\rustibites-view.exe -Out $env:TEMP\tour
# Coordinates are for a 1280x800 client area (the viewer's default window size).
param(
    [string]$Exe = 'target_light\release\rustibites-view.exe',
    [string]$Out = "$env:TEMP\tour",
    [string]$Steps = 'load,settings,library,tools,credits,achievements,new'
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
Start-Sleep 2
function Shot($name) { Shot-Window $p (Join-Path $Out "$name.png") }
# main-menu button centres at 1280x800
$menu = @{ continue = 192; new = 242; load = 291; library = 340; settings = 390; achievements = 443; tools = 492; credits = 541; quit = 594 }
function Open-Menu($name) { Click-Window $p 1080 $menu[$name]; Start-Sleep -Milliseconds 800 }
try {
    Shot 'main'
    foreach ($step in $Steps.Split(',')) {
        switch ($step) {
            'load' {
                Open-Menu 'load'; Shot 'load_0'
                Click-Window $p 250 100; Shot 'load_select'
                Send-Keys $p '{ESC}'; Start-Sleep -Milliseconds 500; Shot 'load_back'
            }
            'settings' {
                Open-Menu 'settings'; Shot 'settings_graphics'
                Wheel-Window $p 640 400 -5; Shot 'settings_scrolled'
                Send-Keys $p '{ESC}'; Start-Sleep -Milliseconds 500
            }
            'library' {
                Open-Menu 'library'; Shot 'library_0'
                Click-Window $p 200 200; Shot 'library_select'
                Send-Keys $p '{ESC}'; Start-Sleep -Milliseconds 500
            }
            'tools' { Open-Menu 'tools'; Shot 'tools'; Send-Keys $p '{ESC}'; Start-Sleep -Milliseconds 500 }
            'credits' { Open-Menu 'credits'; Shot 'credits'; Send-Keys $p '{ESC}'; Start-Sleep -Milliseconds 500 }
            'achievements' { Open-Menu 'achievements'; Shot 'achievements'; Send-Keys $p '{ESC}'; Start-Sleep -Milliseconds 500 }
            'new' {
                Open-Menu 'new'; Shot 'new_0'
                Click-Window $p 1190 500; Shot 'new_dropdown'
            }
        }
    }
    Shot 'end'
    $p.Refresh(); "alive=$(-not $p.HasExited) mem=$([int]($p.WorkingSet64/1MB))MB"
}
finally { Stop-Viewer $p }
