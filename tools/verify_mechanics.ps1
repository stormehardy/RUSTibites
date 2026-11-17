# Drives the real viewer window through the in-world mechanics that were ported from the game's C#
# (inspector panel toggling, the Escape stack, toolbar hover, the Kill Half dialog) and saves a
# screenshot after each step, so they can be looked at. It moves the real cursor and sends real keys:
# it refuses to run unless the keyboard and mouse have been idle (Assert-UserIdle).
#
# Usage:  powershell -File tools\verify_mechanics.ps1 [-Exe target\release\rustibites-view.exe] [-Out target\mechanics]
# Output: <Out>\NN_step.png plus a one-line-per-step log on the console.
param(
    [string]$Exe = 'target\release\rustibites-view.exe',
    [string]$World = 'testdata\real\Default.zip',
    [string]$Out = 'target\mechanics',
    [int]$IdleSeconds = 120
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ui_drive.ps1')
Assert-UserIdle $IdleSeconds
New-Item -ItemType Directory -Force $Out | Out-Null
$W = 1280; $H = 720
$scale = $H / 1080.0   # the UI is laid out for 1920x1080 and scaled by height
$n = 0
function Step($p, [string]$name) {
    $script:n++
    $file = Join-Path $Out ('{0:D2}_{1}.png' -f $script:n, $name)
    Shot-Window $p $file
    Write-Host ('{0:D2} {1}' -f $script:n, $name)
}
$p = Start-Viewer $Exe @($World, '--width', $W, '--height', $H) 6000
try {
    Focus-Window $p
    Step $p 'start'
    # The only bibite is near the centre: a click on it selects it. No panel opens, only the tab buttons.
    Click-Window $p ($W / 2) ($H / 2)
    Step $p 'selected_no_panel'
    # Digit 1 opens Stats, the same digit closes it again; 2 opens Genes.
    Send-Keys $p '1'; Step $p 'key1_stats_open'
    Send-Keys $p '1'; Step $p 'key1_again_closed'
    Send-Keys $p '2'; Step $p 'key2_genes_open'
    # Escape peels off the panel first (the bibite stays selected), then the selection, then opens the pause menu.
    Send-Keys $p '{ESC}'; Step $p 'esc1_panel_closed_still_selected'
    Send-Keys $p '{ESC}'; Step $p 'esc2_deselected'
    Send-Keys $p '{ESC}'; Step $p 'esc3_pause_menu'
    Send-Keys $p '{ESC}'; Step $p 'esc4_menu_closed'
    # Toolbar: resting on the Placer button for 0.8 s opens its panel, leaving closes it 0.5 s later.
    Move-Window $p ([int](35 * $scale)) ([int]($H - 35 * $scale))
    Start-Sleep -Milliseconds 800
    Step $p 'hover_placer_open'
    Move-Window $p ([int]($W / 2)) 200
    Start-Sleep -Milliseconds 800
    Step $p 'moved_away_closed'
    # A press opens the User Actions panel at once; Kill Half asks with the game's own wording.
    Click-Window $p ([int](95 * $scale)) ([int]($H - 35 * $scale))
    Step $p 'user_actions_pressed'
    Click-Window $p ([int](95 * $scale)) ([int]($H - 85 * $scale))
    Step $p 'kill_half_dialog'
    Send-Keys $p '{ESC}'
}
finally {
    Stop-Viewer $p
}
Write-Host "done: $n screenshots in $Out"
