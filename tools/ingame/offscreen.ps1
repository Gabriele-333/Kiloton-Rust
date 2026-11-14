# Moves the development game's window off the desktop and gives the focus back to the window that had
# it before the launch, so that tests run without covering what Gabriele is doing and without his
# keys and clicks landing in the game (the game keeps drawing; horizon-ctl screenshots read its frames).
#   powershell -ExecutionPolicy Bypass -File offscreen.ps1 -ProcessId 1234 [-Restore HWND] [-Back]
#   powershell -ExecutionPolicy Bypass -File offscreen.ps1 -Foreground     (prints the focused window)
# The window is not minimized: minimizing it while it joins the world was the suspect of two
# disconnections ("End of stream") on 11-11.
param([int]$ProcessId, [long]$Restore = 0, [switch]$Back, [switch]$Foreground)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
}
"@
if ($Foreground) { Write-Output ([Win]::GetForegroundWindow().ToInt64()); exit 0 }
$p = Get-Process -Id $ProcessId -ErrorAction Stop
for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 200; $p.Refresh() }
if ($p.MainWindowHandle -eq [IntPtr]::Zero) { Write-Output "no window"; exit 1 }
$h = $p.MainWindowHandle
$x = if ($Back) { 100 } else { -4000 }
# SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
[Win]::SetWindowPos($h, [IntPtr]::Zero, $x, 100, 0, 0, 0x0001 -bor 0x0004 -bor 0x0010) | Out-Null
if (-not $Back) {
    # HWND_BOTTOM, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
    [Win]::SetWindowPos($h, [IntPtr]1, 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0010) | Out-Null
    if ($Restore -ne 0) {
        $prev = [IntPtr]$Restore
        $fg = [Win]::GetForegroundWindow()
        $a = [Win]::GetWindowThreadProcessId($fg, [IntPtr]::Zero)
        $me = [Win]::GetCurrentThreadId()
        [Win]::AttachThreadInput($me, $a, $true) | Out-Null
        [Win]::SetForegroundWindow($prev) | Out-Null
        [Win]::AttachThreadInput($me, $a, $false) | Out-Null
    }
}
Write-Output "moved"
