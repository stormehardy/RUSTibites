# Opens the REAL viewer window (one process after the other), waits, captures its client area to a PNG and closes it.
#
# Unlike tools/ui_drive.ps1 it never moves the cursor, but it does take the keyboard focus (a topmost window) for a few seconds
# per capture, so it WAITS until the user has been idle for -IdleSeconds first (GetLastInputInfo) and skips the remaining
# captures if the user comes back. Run it detached and wait for its output, e.g.
#
#   Start-Process powershell -ArgumentList '-NoProfile','-File','tools\window_capture.ps1' -WindowStyle Hidden `
#       -RedirectStandardOutput out.txt   # variants below are the defaults; or call it from PowerShell with -Variants:
#   & tools\window_capture.ps1 -Exe target_light\release\rustibites-view.exe -OutDir $env:TEMP -Variants @(
#       @{ name = 'basic'; args = @('testdata\real\Default.zip', '--ortho-size', '90', '--center', '70,754.7') },
#       @{ name = 'blue';  args = @('testdata\real\Default.zip', '--ortho-size', '90', '--center', '70,754.7', '--no-gpu-hue-shader') })
#
# Output: <OutDir>\window_<name>.png and window_<name>.log (the viewer's stderr), and "done" as the last line of stdout.
# Camera: the window starts at the origin; a bibite moves a few tens of units in the first seconds, so use a wide view
# (`--ortho-size 90`) with `--center X,Y` of its start (the headless `--screenshot` prints its camera centre).
# Selecting a bibite (`--select-first`) draws its field-of-view cone over the sprites and tints their colours: keep it
# unselected for colour checks.
param(
    [string]$Exe = 'target_light\release\rustibites-view.exe',
    [string]$OutDir = (Join-Path $env:TEMP 'rustibites_window_capture'),
    [object[]]$Variants = @(
        @{ name = 'default'; args = @('testdata\real\Default.zip', '--ortho-size', '90', '--center', '70,754.7') }
    ),
    [int]$WaitMs = 4500,
    [int]$IdleSeconds = 90,
    [int]$MaxWaitMinutes = 40
)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
. (Join-Path $PSScriptRoot 'ui_drive.ps1')
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$deadline = (Get-Date).AddMinutes($MaxWaitMinutes)
while ([IdleInput]::Seconds() -lt $IdleSeconds) {
    if ((Get-Date) -gt $deadline) { Write-Host "user stayed active for $MaxWaitMinutes minutes: no window opened"; exit 2 }
    Start-Sleep -Seconds 5
}
Write-Host ("idle {0:N0} s: opening the windows" -f [IdleInput]::Seconds())
foreach ($v in $Variants) {
    if ([IdleInput]::Seconds() -lt 12) { Write-Host "user is back: skipping $($v.name)"; continue }
    $out = Join-Path $OutDir ("window_{0}.png" -f $v.name)
    $log = [System.IO.Path]::ChangeExtension($out, 'log')
    $p = Start-Process -FilePath $Exe -ArgumentList $v.args -PassThru -RedirectStandardError $log
    Start-Sleep -Milliseconds $WaitMs
    $p.Refresh()
    if ($p.HasExited) { Write-Host "$($v.name): viewer exited early: $($p.ExitCode)"; continue }
    Focus-Window $p
    Start-Sleep -Milliseconds 700
    Shot-Window $p $out
    Stop-Viewer $p
    Write-Host "saved $out"
    Start-Sleep -Milliseconds 800
}
Write-Host 'done'
