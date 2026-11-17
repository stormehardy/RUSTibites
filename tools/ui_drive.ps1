# Drives the real Windows viewer window with real mouse/keyboard input and captures it.
# Usage (dot-source, then call):  . tools\ui_drive.ps1
#   $p = Start-Viewer 'target_light\release\rustibites-view.exe' @('--menu')
#   Click-Window $p 960 540          # window-client pixels
#   Send-Keys $p '{DOWN}{ENTER}'     # System.Windows.Forms.SendKeys syntax
#   Wheel-Window $p 960 540 -3       # notches (negative = down)
#   Shot-Window $p out.png
#   Stop-Viewer $p
# Needs an interactive desktop session; it moves the real cursor while it runs, so call
# `Assert-UserIdle` first: it throws when the keyboard or mouse was used in the last N seconds.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class IdleInput {
  [StructLayout(LayoutKind.Sequential)] struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }
  [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LASTINPUTINFO info);
  public static double Seconds() {
    LASTINPUTINFO info = new LASTINPUTINFO(); info.cbSize = (uint)Marshal.SizeOf(info);
    GetLastInputInfo(ref info);
    return unchecked((uint)Environment.TickCount - info.dwTime) / 1000.0;
  }
}
'@
function Assert-UserIdle([int]$seconds = 120) {
    $idle = [IdleInput]::Seconds()
    if ($idle -lt $seconds) { throw ("The user was active {0:N0} s ago (needs {1} s idle): not driving the real cursor." -f $idle, $seconds) }
}
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct PT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref PT p);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, int d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
}
'@
[void][W]::SetProcessDPIAware()

function Start-Viewer([string]$exe, [string[]]$arguments = @(), [int]$waitMs = 4000) {
    $p = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
    Start-Sleep -Milliseconds $waitMs
    $p.Refresh()
    return $p
}
function Stop-Viewer($p) { if ($p -and -not $p.HasExited) { $p.Kill() } }
# The window is made topmost (HWND_TOPMOST, no move/size) so another app cannot cover it: screenshots
# copy the screen rectangle of the client area.
function Focus-Window($p) {
    $p.Refresh()
    [void][W]::SetWindowPos($p.MainWindowHandle, [IntPtr](-1), 0, 0, 0, 0, 0x0003)
    [void][W]::SetForegroundWindow($p.MainWindowHandle); Start-Sleep -Milliseconds 150
}
function Client-Origin($p) {
    $pt = New-Object W+PT; [void][W]::ClientToScreen($p.MainWindowHandle, [ref]$pt); return $pt
}
function Move-Window($p, [int]$x, [int]$y) {
    $o = Client-Origin $p; [void][W]::SetCursorPos($o.X + $x, $o.Y + $y); Start-Sleep -Milliseconds 80
}
function Click-Window($p, [int]$x, [int]$y, [switch]$Right) {
    Focus-Window $p; Move-Window $p $x $y
    $down, $up = if ($Right) { 0x08, 0x10 } else { 0x02, 0x04 }
    [W]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 60
    [W]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 250
}
# Press at (x0,y0), move in steps to (x1,y1), release (left, or right with -Right).
function Drag-Window($p, [int]$x0, [int]$y0, [int]$x1, [int]$y1, [switch]$Right, [int]$steps = 12) {
    Focus-Window $p; Move-Window $p $x0 $y0
    $down, $up = if ($Right) { 0x08, 0x10 } else { 0x02, 0x04 }
    [W]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80
    for ($i = 1; $i -le $steps; $i++) {
        Move-Window $p ([int]($x0 + ($x1 - $x0) * $i / $steps)) ([int]($y0 + ($y1 - $y0) * $i / $steps))
        Start-Sleep -Milliseconds 30
    }
    [W]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 250
}
function Wheel-Window($p, [int]$x, [int]$y, [int]$notches) {
    Focus-Window $p; Move-Window $p $x $y
    [W]::mouse_event(0x0800, 0, 0, 120 * $notches, [UIntPtr]::Zero); Start-Sleep -Milliseconds 250
}
# miniquad (macroquad's windowing) maps keys by hardware scancode, so SendKeys (scancode 0) is ignored:
# send real scancodes with keybd_event instead.
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class K {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
}
'@
$script:VK = @{ ESC = 0x1B; ENTER = 0x0D; TAB = 0x09; SPACE = 0x20; BACKSPACE = 0x08; DELETE = 0x2E; HOME = 0x24; END = 0x23
    PGUP = 0x21; PGDN = 0x22; LEFT = 0x25; UP = 0x26; RIGHT = 0x27; DOWN = 0x28; PLUS = 0xBB; MINUS = 0xBD
    F1 = 0x70; F2 = 0x71; F3 = 0x72; F4 = 0x73; F5 = 0x74; F6 = 0x75; F7 = 0x76; F8 = 0x77; F9 = 0x78; F10 = 0x79; F11 = 0x7A; F12 = 0x7B }
function Press-Vk([int]$vk, [string[]]$mods = @()) {
    $extended = if ($vk -in 0x21..0x28 + 0x2E) { 1 } else { 0 }
    $modVk = @{ CTRL = 0x11; SHIFT = 0x10; ALT = 0x12 }
    foreach ($m in $mods) { $v = $modVk[$m]; [K]::keybd_event($v, [byte][K]::MapVirtualKey($v, 0), 0, [UIntPtr]::Zero) }
    [K]::keybd_event([byte]$vk, [byte][K]::MapVirtualKey($vk, 0), $extended, [UIntPtr]::Zero); Start-Sleep -Milliseconds 70
    [K]::keybd_event([byte]$vk, [byte][K]::MapVirtualKey($vk, 0), $extended -bor 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 70
    foreach ($m in $mods) { $v = $modVk[$m]; [K]::keybd_event($v, [byte][K]::MapVirtualKey($v, 0), 2, [UIntPtr]::Zero) }
}
# Syntax: '{DOWN}{ENTER}' named keys, plain characters (letters/digits), prefix ^ (ctrl), + (shift) for the next key.
function Send-Keys($p, [string]$keys) {
    Focus-Window $p
    $i = 0
    while ($i -lt $keys.Length) {
        $mods = @()
        while ($keys[$i] -in '^', '+', '%') { $mods += @{ '^' = 'CTRL'; '+' = 'SHIFT'; '%' = 'ALT' }[[string]$keys[$i]]; $i++ }
        if ($keys[$i] -eq '{') {
            $end = $keys.IndexOf('}', $i); $name = $keys.Substring($i + 1, $end - $i - 1).ToUpper(); $i = $end + 1
            Press-Vk $script:VK[$name] $mods
        } else {
            $c = [char]::ToUpper($keys[$i]); $i++
            Press-Vk ([int]$c) $mods
        }
    }
    Start-Sleep -Milliseconds 250
}
function Shot-Window($p, [string]$path) {
    $r = New-Object W+RECT; [void][W]::GetClientRect($p.MainWindowHandle, [ref]$r)
    $o = Client-Origin $p
    $bmp = New-Object System.Drawing.Bitmap $r.R, $r.B
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($o.X, $o.Y, 0, 0, $bmp.Size)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
}
