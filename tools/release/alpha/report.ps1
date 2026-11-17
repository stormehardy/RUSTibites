# Collects what is needed to look at a problem: the logs, the settings, the version, the machine (memory, graphics card and driver), what Windows recorded about a
# crash of this program, and the newest screenshots. -OutDir puts the zip somewhere other than the Desktop.
param([string]$OutDir = '')
Set-Location -LiteralPath $PSScriptRoot
$stage = Join-Path ([IO.Path]::GetTempPath()) ("rustibites-report-" + (Get-Date -Format yyyyMMdd-HHmmss))
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($item in @('logs', 'VERSION.txt', 'userdata\game_user_settings.json')) {
    if (Test-Path $item) { Copy-Item -Recurse -Force $item (Join-Path $stage (Split-Path $item -Leaf)) }
}

# The newest pictures of the last 12 hours. The game's art under assets\ is not looked into (thousands of pictures, none of them a screenshot).
$folders = @(Get-ChildItem -LiteralPath $PSScriptRoot -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'assets' })
$shots = @(Get-ChildItem -LiteralPath $PSScriptRoot -File -Filter '*.png' -ErrorAction SilentlyContinue) +
    @($folders | ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Recurse -File -Filter '*.png' -ErrorAction SilentlyContinue })
$shots = @($shots | Where-Object { $_.LastWriteTime -gt (Get-Date).AddHours(-12) } | Sort-Object LastWriteTime -Descending | Select-Object -First 10)
foreach ($shot in $shots) { Copy-Item -LiteralPath $shot.FullName $stage }

# The machine: Windows, memory (what is free matters for a program that stops by itself), processor, graphics card(s) and their driver.
$os = Get-CimInstance Win32_OperatingSystem
$lines = @(
    "Windows: $([Environment]::OSVersion.VersionString) ($($os.OSArchitecture), language $((Get-Culture).Name))",
    ("RAM: {0:N1} GB, free {1:N1} GB; commit limit {2:N1} GB, free {3:N1} GB" -f ($os.TotalVisibleMemorySize / 1MB), ($os.FreePhysicalMemory / 1MB), ($os.TotalVirtualMemorySize / 1MB), ($os.FreeVirtualMemory / 1MB)),
    "CPU: $((Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim())"
)
foreach ($gpu in @(Get-CimInstance Win32_VideoController)) {
    $date = if ($gpu.DriverDate) { $gpu.DriverDate.ToString('yyyy-MM-dd') } else { 'no date' }
    $lines += "GPU: $($gpu.Name), driver $($gpu.DriverVersion) ($date)"
}
$lines | Set-Content (Join-Path $stage 'machine.txt')

# What Windows recorded when the program crashed (Event Viewer > Windows Logs > Application > "Application Error"): the module that failed tells a crash in the
# graphics driver (nvoglv64.dll, atioglxx.dll, ig*icd64.dll ...) from one in this program. The fields are read by position, so the language of Windows does not matter.
try {
    $crashes = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; ProviderName = 'Application Error'; StartTime = (Get-Date).AddDays(-7) } -ErrorAction SilentlyContinue |
        Where-Object { $_.Properties.Count -ge 8 -and "$($_.Properties[0].Value)" -match 'RUSTibites' } | Select-Object -First 6)
    if ($crashes.Count -gt 0) {
        $text = foreach ($e in $crashes) {
            "{0:s}  {1} {2}: faulting module {3} {4}, exception {5}, offset {6}" -f $e.TimeCreated, $e.Properties[0].Value, $e.Properties[1].Value, $e.Properties[3].Value, $e.Properties[4].Value, $e.Properties[6].Value, $e.Properties[7].Value
            $e.Message
            ''
        }
        $text | Set-Content (Join-Path $stage 'crash_events.txt')
    }
} catch { }

$target = if ($OutDir) { $OutDir } else { [Environment]::GetFolderPath('Desktop') }
New-Item -ItemType Directory -Force $target | Out-Null
$zip = Join-Path $target ("RUSTibites-report-" + (Get-Date -Format yyyyMMdd-HHmmss) + '.zip')
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
Write-Host "Report written to $zip"
Write-Host 'Attach it with a screenshot of the game next to a screenshot of this port, and say what you did.'
