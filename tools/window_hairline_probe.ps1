# What the real window draws round a bibite that the headless software render does not (see docs/PARITY_REMAINING.md, "artifacts floating around the bibite"):
# one in-process framebuffer capture of the real window per view (RUSTIBITES_SELF_SHOT: no key, no mouse, no OS-level capture), next to the headless software
# render of the same view, then a comparison of each pair:
#   * "specks" views (a neutral bibite, tools/window_specks.py): faint dotted lines along the sprites' rectangles = pixels only the window has;
#   * "colour" views (the same bibite recoloured, tools/window_pair_diff.py): a coloured body that goes grey in the window (a finished picture drawn through
#     the colour material) shows as far fewer saturated pixels in the window than in the software render.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\window_hairline_probe.ps1 [-Exe <viewer exe>] [-Save <zip>] [-OutDir C:\Temp\hairline] [-Variants 0,0.5,0.75]
#
# Idle-gated: a window opens only after -IdleSeconds of user idle (90 by default), one at a time, and not at all while the user is back (the captures not yet
# taken wait for the next quiet spell, for up to -MaxWaitMinutes). Windowed 1280 x 720 with an isolated data directory (Fullscreen false); the bibite sits at the
# centre, clear of the HUD. Needs a save with a bibite at -Center (the game's default world rust_seed_600.zip has one at -5.4,831.0); the colour views use a copy
# of it with that bibite recoloured (tools/recolour_save.py, written to <OutDir> when missing). A window opens for a few seconds per view.
#
# -Variants runs every view once per value of RUSTIBITES_SPRITE_INSET_PIXELS (render.rs `SPRITE_INSET_PIXELS`, how far in from its cell's border the window
# samples a sprite, in screen pixels): 0 is what the window did before the fix (89 / 105 / 33 window-only specks round the bibite at plain 20.6, tail20 20.6,
# plain 87.8 on the NVIDIA GTX 1660 SUPER the marks were first seen on, 48 / 47 / 11 of them in empty space = the dotted lines), the exe's default (0.75) leaves
# none in empty space (the others, 7 to 34, are art pixels the supersampled software render draws differently). The captures are <OutDir>\win_<view>_<value>.png.
# Without -Variants one capture per view is taken with the exe's own default. The exit code is 1 while a window-only speck in empty space exists.
#
# The colour views are the check of the second defect (a coloured body drawn grey through the colour material): the window's mean |dRGB| against the software
# render of the recoloured bibite with a forced 20 degree tail was 11.8 at plain 20.6 before the fix (1.7 without the tail), 1.5 after.
param(
    [string]$Exe = 'Z:\RUSTibites\target\release\rustibites-view.exe',
    [string]$Save = 'Z:\RUSTibites\testdata\parity\default\rust_seed_600.zip',
    [string]$OutDir = 'C:\Temp\hairline',
    [string]$Center = '-5.4,831.0',
    [int]$IdleSeconds = 90,
    [int]$MaxWaitMinutes = 20,
    [int]$SelfShotFrame = 40,
    [string]$Region = '300,120,1000,640',
    [string[]]$Variants = @(),
    [switch]$NoColour,
    # keep the software twins that are already in <OutDir> (the headless render does not depend on the window code under test)
    [switch]$ReuseSoftware,
    # only these views (by name), in the order of the list below: e.g. -Only colour_tail20_o21,plain_o88
    [string[]]$Only = @()
)
$root = 'Z:\RUSTibites'
$py = 'python'
$pyenv = Join-Path $env:USERPROFILE '.pyenv\pyenv-win\versions\3.11.0\python.exe'
if (Test-Path $pyenv) { $py = $pyenv }
Set-Location $root
. (Join-Path $root 'tools\ui_drive.ps1')
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$colourSave = Join-Path $OutDir 'coloured.zip'
if (-not $NoColour -and -not (Test-Path $colourSave)) {
    $c = $Center -split ','
    & $py -I (Join-Path $root 'tools\recolour_save.py') $Save $colourSave $c[0] $c[1] 0.95 0.2 0.05 | Out-Null
}
# name, ortho size (half the window height in world units: 20.6 = 17.5 px per unit, 87.8 = 4.1 px per unit = the default zoom, 200 = 1.8 px per unit: about two
# pixels per texel), RUSTIBITES_FORCE_TAIL degrees ('' = none), the save, the kind of comparison
$views = @(
    @{ name = 'plain_o21';  ortho = '20.6'; tail = '';   save = $Save; kind = 'specks' },
    @{ name = 'tail20_o21'; ortho = '20.6'; tail = '20'; save = $Save; kind = 'specks' },
    @{ name = 'plain_o88';  ortho = '87.8'; tail = '';   save = $Save; kind = 'specks' },
    @{ name = 'tail20_o88'; ortho = '87.8'; tail = '20'; save = $Save; kind = 'specks' },
    @{ name = 'plain_o200'; ortho = '200';  tail = '';   save = $Save; kind = 'specks' }
)
if (-not $NoColour) {
    $views += @(
        @{ name = 'colour_plain_o21';  ortho = '20.6'; tail = '';   save = $colourSave; kind = 'colour' },
        @{ name = 'colour_tail20_o21'; ortho = '20.6'; tail = '20'; save = $colourSave; kind = 'colour' },
        @{ name = 'colour_tail20_o88'; ortho = '87.8'; tail = '20'; save = $colourSave; kind = 'colour' }
    )
}
if ($Only.Count -gt 0) { $views = @($views | Where-Object { $Only -contains $_.name }) }
$common = @('--paused', '--width', '1280', '--height', '720', '--center', $Center)

# 1) the software twins (headless; no window)
foreach ($v in $views) {
    $png = Join-Path $OutDir ("soft_{0}.png" -f $v.name)
    if ($ReuseSoftware -and (Test-Path $png)) { Write-Host "software $($v.name): kept"; continue }
    $data = Join-Path $OutDir 'data_soft'
    New-Item -ItemType Directory -Force -Path $data | Out-Null
    $env:RUSTIBITES_DATA_DIR = $data
    if ($v.tail -ne '') { $env:RUSTIBITES_FORCE_TAIL = $v.tail } else { Remove-Item Env:RUSTIBITES_FORCE_TAIL -ErrorAction SilentlyContinue }
    $args2 = @($v.save, '--screenshot', $png) + $common + @('--ortho-size', $v.ortho)
    $p = Start-Process -FilePath $Exe -ArgumentList $args2 -WorkingDirectory $root -Wait -PassThru -NoNewWindow -RedirectStandardError (Join-Path $OutDir ("soft_{0}.err" -f $v.name)) -RedirectStandardOutput (Join-Path $OutDir ("soft_{0}.out" -f $v.name))
    Remove-Item Env:RUSTIBITES_DATA_DIR
    Remove-Item Env:RUSTIBITES_FORCE_TAIL -ErrorAction SilentlyContinue
    Write-Host ("software {0}: exit {1}" -f $v.name, $p.ExitCode)
}

# 2) the window captures, only while the user is away
function Capture-Window($v, $suffix, $edges) {
    $data = Join-Path $OutDir ("data_win_{0}{1}" -f $v.name, $suffix)
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $data | Out-Null
    '{"version":2,"values":{"Fullscreen":false}}' | Set-Content (Join-Path $data 'game_user_settings.json') -Encoding ascii
    $png = Join-Path $OutDir ("win_{0}{1}.png" -f $v.name, $suffix)
    Remove-Item $png -ErrorAction SilentlyContinue
    $env:RUSTIBITES_DATA_DIR = $data
    $env:RUSTIBITES_SELF_SHOT = ("{0}:{1}" -f $SelfShotFrame, ($png -replace '\\', '/'))
    if ($v.tail -ne '') { $env:RUSTIBITES_FORCE_TAIL = $v.tail } else { Remove-Item Env:RUSTIBITES_FORCE_TAIL -ErrorAction SilentlyContinue }
    if ($edges -ne '') { $env:RUSTIBITES_SPRITE_INSET_PIXELS = $edges } else { Remove-Item Env:RUSTIBITES_SPRITE_INSET_PIXELS -ErrorAction SilentlyContinue }
    $log = Join-Path $OutDir ("win_{0}{1}.log" -f $v.name, $suffix)
    $p = Start-Process -FilePath $Exe -ArgumentList (@($v.save) + $common + @('--ortho-size', $v.ortho)) -WorkingDirectory $root -PassThru -RedirectStandardError $log -RedirectStandardOutput ($log + '.out')
    Remove-Item Env:RUSTIBITES_DATA_DIR
    Remove-Item Env:RUSTIBITES_SELF_SHOT
    Remove-Item Env:RUSTIBITES_FORCE_TAIL -ErrorAction SilentlyContinue
    Remove-Item Env:RUSTIBITES_SPRITE_INSET_PIXELS -ErrorAction SilentlyContinue
    if (-not $p.WaitForExit(60000)) { $p.Kill(); Write-Host "$($v.name)${suffix}: killed after 60 s" }
    Start-Sleep -Milliseconds 800
    if (Test-Path $png) { Write-Host "window $($v.name)${suffix}: capture saved"; return $true }
    Write-Host "window $($v.name)${suffix}: no capture"
    return $false
}
# what is still to capture: every view, once per variant
$suffixes = @('')
if ($Variants.Count -gt 0) { $suffixes = @($Variants | ForEach-Object { "_$_" }) }
$pending = New-Object System.Collections.ArrayList
foreach ($v in $views) {
    foreach ($s in $suffixes) {
        $e = ''
        if ($s -ne '') { $e = $s.Substring(1) }
        if (-not (Test-Path (Join-Path $OutDir ("win_{0}{1}.png" -f $v.name, $s)))) { [void]$pending.Add(@{ view = $v; suffix = $s; edges = $e }) }
    }
}
$deadline = (Get-Date).AddMinutes($MaxWaitMinutes)
while ($pending.Count -gt 0) {
    while ([IdleInput]::Seconds() -lt $IdleSeconds) {
        if ((Get-Date) -gt $deadline) { break }
        Start-Sleep -Seconds 3
    }
    if ((Get-Date) -gt $deadline -and [IdleInput]::Seconds() -lt $IdleSeconds) { Write-Host ("the user did not stay away long enough: {0} capture(s) not taken" -f $pending.Count); break }
    Write-Host ("idle {0:N0} s: opening the windows ({1} to take)" -f [IdleInput]::Seconds(), $pending.Count)
    foreach ($item in @($pending)) {
        if ([IdleInput]::Seconds() -lt 10) { Write-Host 'the user is back: the rest waits for the next quiet spell'; break }
        if (Capture-Window $item.view $item.suffix $item.edges) { $pending.Remove($item) }
    }
}

# 3) the comparison
$exit = 0
foreach ($v in $views) {
    $soft = Join-Path $OutDir ("soft_{0}.png" -f $v.name)
    foreach ($s in $suffixes) {
        $win = Join-Path $OutDir ("win_{0}{1}.png" -f $v.name, $s)
        if (-not ((Test-Path $win) -and (Test-Path $soft))) { continue }
        Write-Host ("== {0}{1}" -f $v.name, $s)
        if ($v.kind -eq 'specks') {
            & $py -I (Join-Path $root 'tools\window_specks.py') $win $soft $Region (Join-Path $OutDir ("marked_{0}{1}.png" -f $v.name, $s))
            if ($LASTEXITCODE -ne 0) { $exit = 1 }
        } else {
            & $py -I (Join-Path $root 'tools\window_pair_diff.py') $win $soft $Region (Join-Path $OutDir ("diff_{0}{1}.png" -f $v.name, $s))
        }
    }
}
Write-Host 'done'
exit $exit
