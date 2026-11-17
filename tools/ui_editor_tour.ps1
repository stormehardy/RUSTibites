# Walks the scenario editor with real mouse/keyboard input and saves a screenshot per step.
# Usage: powershell -File tools\ui_editor_tour.ps1 -Exe target_light\release\rustibites-view.exe -Out $env:TEMP\editor_tour
# Coordinates are reference px (1920x1080) scaled to the 1280x720 client area (x 2/3). It moves the
# real cursor: do not use the mouse while it runs.
param(
    [string]$Exe = 'target_light\release\rustibites-view.exe',
    [string]$Out = "$env:TEMP\editor_tour",
    [string]$Steps = 'open,toggle,tabs,zones,slider,save,start'
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
function Shot($name) { Shot-Window $p (Join-Path $Out "$name.png") }
# reference px -> client px
function Rx([double]$v) { [int]($v * 2 / 3) }
function Click-Ref([double]$x, [double]$y) { Click-Window $p (Rx $x) (Rx $y) }
# The viewer scrolls one step per wheel event: send one event per notch.
function Scroll-Notches([int]$n) { for ($i = 0; $i -lt [math]::Abs($n); $i++) { Wheel-Window $p (Rx 560) (Rx 600) ([math]::Sign($n)) } }
try {
    foreach ($step in $Steps.Split(',')) {
        switch ($step) {
            'open' {
                Click-Ref 1622 362; Start-Sleep -Milliseconds 800; Shot 'new_game'
                Click-Ref 635 65; Start-Sleep -Milliseconds 1500; Shot 'editor_open'
            }
            'toggle' {
                Click-Ref 913 195; Shot 'toggle_corpses_off'
                Click-Ref 1062 320; Shot 'pellet_options_open'
                Click-Ref 1062 320
                Scroll-Notches -6; Shot 'scrolled'
                Scroll-Notches 10
            }
            'tabs' {
                Click-Ref 231 45; Shot 'tab_energy'
                Click-Ref 317 45; Shot 'tab_bibites'
                Click-Ref 403 45; Shot 'tab_anims'
                Click-Ref 1080 45; Shot 'tab_challenge'
                Click-Ref 60 45; Shot 'tab_sim'
            }
            'zones' {
                Click-Ref 145 45; Shot 'tab_zones'
                Click-Ref 61 638; Shot 'zone_added'
                Click-Ref 1080 638; Shot 'zone_clear_dialog'
                Send-Keys $p '{ESC}'
            }
            'slider' {
                Click-Ref 60 45
                Scroll-Notches -6
                # the first slider row of the performance group (Ticks Per Second) is at the bottom
                Drag-Window $p (Rx 820) (Rx 1005) (Rx 960) (Rx 1005); Shot 'slider_dragged'
                Scroll-Notches 10
            }
            'save' {
                Click-Ref 1665 45; Start-Sleep -Milliseconds 500; Shot 'save_form'
                Send-Keys $p 'ZED'; Shot 'save_typed'
                Send-Keys $p '{ESC}'
            }
            'zoneedit' {
                Click-Ref 145 45
                Click-Ref 500 712; Shot 'zone_open'
                Click-Ref 850 810; Click-Ref 850 810; Click-Ref 850 810; Shot 'zone_ring'
                Drag-Window $p (Rx 888) (Rx 860) (Rx 760) (Rx 860); Shot 'zone_fertility'
                Scroll-Notches -4; Shot 'zone_scrolled'
            }
            'materials' {
                Click-Ref 231 45
                Scroll-Notches -12; Shot 'materials_scrolled'
                Click-Ref 1068 615; Shot 'material_open'
                Scroll-Notches -3; Shot 'material_open_scrolled'
            }
            'saveflow' {
                Click-Ref 1665 45; Start-Sleep -Milliseconds 500
                Send-Keys $p '{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}{BACKSPACE}'
                Send-Keys $p 'ZED'; Shot 'saveflow_typed'
                Click-Ref 445 274; Start-Sleep -Milliseconds 800; Shot 'saveflow_saved'
            }
            'start' {
                Click-Ref 1860 45; Start-Sleep -Milliseconds 500; Shot 'start_dialog'
                Click-Ref 1162 665; Start-Sleep -Seconds 8; Shot 'world_started'
            }
        }
    }
    $p.Refresh(); "alive=$(-not $p.HasExited) mem=$([int]($p.WorkingSet64/1MB))MB"
}
finally { Stop-Viewer $p }
