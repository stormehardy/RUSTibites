# usage: ui_sweep.ps1 <exe> <tag>   -> renders every menu_ui reference session, prints mean diff and text-ink ratio per session
param([string]$Exe, [string]$Tag)
$py = "C:\Users\Storme\.pyenv\pyenv-win\versions\3.11.0\python.exe"
$black = "C:\Users\Storme\AppData\Local\Temp\claude\03cb0fd17c9cd590\8d2193bd-3754-4825-9996-dc16f1b3141d\scratchpad\black.png"
$refdir = "Z:\RUSTibites\assets\game\reference\functional_ui_native\menu_ui"
New-Item -ItemType Directory -Force "C:\Temp\sweep_$Tag" | Out-Null
Set-Location Z:\RUSTibites
$names = Get-ChildItem $refdir -Filter "*_ui_on_black_1920x1080.png" | ForEach-Object { $_.Name -replace '_ui_on_black_1920x1080.png$', '' }
foreach ($s in $names) {
    $out = "C:\Temp\sweep_$Tag\$s.png"
    & $Exe --ui-session $s --ui-background $black --width 1920 --height 1080 --screenshot $out 2>&1 | Out-Null
    if (-not (Test-Path $out)) { "$s : no render"; continue }
    $ref = "$refdir\${s}_ui_on_black_1920x1080.png"
    $r = & $py -X utf8 C:\Temp\sweep_one.py $out $ref
    "$s : $r"
}
