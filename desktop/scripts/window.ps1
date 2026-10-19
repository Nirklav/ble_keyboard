# Finds a WSLg window by its title on the Windows side and screenshots it or sends input to it.
#   -Action find                 exits 0 when the window is visible, prints its size
#   -Action shot  -Out <png>     saves the window area as PNG (the window is brought to front)
#   -Action click -X <x> -Y <y>  left click at window coordinates (pixels from the top-left corner)
#   -Action type  -Text <text>   types text (ASCII, mapped through the US layout)
#   -Action resize -X <w> -Y <h> [-Text <x,y>] drags the bottom-right corner (or the point x,y) to w,h
#   -Action key   -Text <combo>  presses a key combo: enter, esc, tab, backspace, delete, up, f2, ctrl+a, ctrl+shift+s, ...
param(
    [Parameter(Mandatory)][ValidateSet("find", "shot", "click", "type", "key", "resize")][string]$Action,
    [string]$Title = "Keyboard config",
    [string]$Out,
    [int]$X,
    [int]$Y,
    [string]$Text
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class Win {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int ht, bool repaint);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, UIntPtr e);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern short VkKeyScanEx(char c, IntPtr layout);
    [DllImport("user32.dll")] public static extern uint MapVirtualKeyEx(uint code, uint type, IntPtr layout);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr LoadKeyboardLayout(string id, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetKeyboardLayout(uint thread);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);

    // WSLg maps scan codes to characters with the window's Windows input language, so it is switched
    // to English while typing and restored afterwards.
    public static IntPtr Layout(IntPtr window) { return GetKeyboardLayout(GetWindowThreadProcessId(window, IntPtr.Zero)); }
    public static void SetLayout(IntPtr window, IntPtr layout) {
        PostMessage(window, 0x50, IntPtr.Zero, layout); // WM_INPUTLANGCHANGEREQUEST
        System.Threading.Thread.Sleep(300);
    }

    // Characters are mapped to keys through the US layout.
    public static readonly IntPtr US = LoadKeyboardLayout("00000409", 0x80); // KLF_NOTELLSHELL

    // The RDP client behind WSLg windows only forwards real scan codes, not SendKeys' unicode input.
    public static void Key(byte vk, bool up) {
        uint f = 0x8 | (up ? 0x2u : 0u); // KEYEVENTF_SCANCODE, KEYEVENTF_KEYUP
        if (vk >= 0x21 && vk <= 0x2E) f |= 0x1; // KEYEVENTF_EXTENDEDKEY: arrows, Home, End, PgUp, PgDn, Ins, Del
        keybd_event(vk, (byte)MapVirtualKeyEx(vk, 0, US), f, UIntPtr.Zero);
        System.Threading.Thread.Sleep(10);
    }
    public static void Press(byte[] mods, byte vk) {
        foreach (var m in mods) Key(m, false);
        Key(vk, false); Key(vk, true);
        for (int i = mods.Length - 1; i >= 0; i--) Key(mods[i], true);
    }
}
"@
[Win]::SetProcessDPIAware() | Out-Null

$script:window = [IntPtr]::Zero
$enum = {
    param($h, $l)
    $sb = New-Object Text.StringBuilder 256
    [Win]::GetWindowText($h, $sb, 256) | Out-Null
    if ([Win]::IsWindowVisible($h) -and $sb.ToString().Contains($Title)) { $script:window = $h; return $false }
    return $true
}
[Win]::EnumWindows($enum, [IntPtr]::Zero) | Out-Null
if ($script:window -eq [IntPtr]::Zero) { [Console]::Error.WriteLine("window '$Title' not found"); exit 1 }

$rect = New-Object Win+RECT
[Win]::GetWindowRect($script:window, [ref]$rect) | Out-Null
$width = $rect.R - $rect.L
$height = $rect.B - $rect.T

if ($Action -ne "find") {
    [Win]::SetForegroundWindow($script:window) | Out-Null
    Start-Sleep -Milliseconds 300
}

switch ($Action) {
    "find" { "${width}x${height}" }
    "resize" {
        # Drags the bottom-right corner like a user would: WSLg doesn't pass MoveWindow to the app.
        # -Text "x,y": where to grab, window coordinates (the frame corner, not its shadow).
        if ($Text) { $from = $Text.Split(","); $fromX = $rect.L + [int]$from[0]; $fromY = $rect.T + [int]$from[1] }
        else { $fromX = $rect.R - 2; $fromY = $rect.B - 2 }
        $toX = $rect.L + $X; $toY = $rect.T + $Y
        [Win]::SetCursorPos($fromX, $fromY) | Out-Null
        Start-Sleep -Milliseconds 100
        [Win]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero) # left down
        for ($i = 1; $i -le 20; $i++) {
            [Win]::SetCursorPos($fromX + ($toX - $fromX) * $i / 20, $fromY + ($toY - $fromY) * $i / 20) | Out-Null
            Start-Sleep -Milliseconds 20
        }
        [Win]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero) # left up
        Start-Sleep -Milliseconds 500
        [Win]::GetWindowRect($script:window, [ref]$rect) | Out-Null
        "$($rect.R - $rect.L)x$($rect.B - $rect.T)"
    }
    "shot" {
        $bmp = New-Object Drawing.Bitmap $width, $height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($rect.L, $rect.T, 0, 0, $bmp.Size)
        $bmp.Save($Out, [Drawing.Imaging.ImageFormat]::Png)
        "${width}x${height}"
    }
    "click" {
        [Win]::SetCursorPos($rect.L + $X, $rect.T + $Y) | Out-Null
        Start-Sleep -Milliseconds 50
        [Win]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero) # left down
        Start-Sleep -Milliseconds 50
        [Win]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero) # left up
    }
    "type" {
        $previous = [Win]::Layout($script:window)
        [Win]::SetLayout($script:window, [Win]::US)
        foreach ($c in $Text.ToCharArray()) {
            $scan = [Win]::VkKeyScanEx($c, [Win]::US)
            if ($scan -eq -1) { [Console]::Error.WriteLine("no US-layout key for '$c'"); exit 1 }
            $mods = @()
            if ($scan -band 0x100) { $mods += [byte]0x10 } # shift
            if ($scan -band 0x200) { $mods += [byte]0x11 } # ctrl
            if ($scan -band 0x400) { $mods += [byte]0x12 } # alt
            [Win]::Press([byte[]]$mods, [byte]($scan -band 0xFF))
        }
        Start-Sleep -Milliseconds 200
        [Win]::SetLayout($script:window, $previous)
    }
    "key" {
        $named = @{
            enter = 0x0D; esc = 0x1B; escape = 0x1B; tab = 0x09; backspace = 0x08; delete = 0x2E; del = 0x2E; space = 0x20
            insert = 0x2D; home = 0x24; end = 0x23; pageup = 0x21; pagedown = 0x22; left = 0x25; up = 0x26; right = 0x27; down = 0x28
            ctrl = 0x11; shift = 0x10; alt = 0x12
        }
        $parts = $Text.ToLower().Split("+")
        $codes = foreach ($p in $parts) {
            if ($named.ContainsKey($p)) { $named[$p] }
            elseif ($p -match '^f([1-9]|1[0-2])$') { 0x6F + [int]$Matches[1] }
            elseif ($p -match '^[a-z0-9]$') { [int][char]$p.ToUpper() }
            else { [Console]::Error.WriteLine("unknown key '$p'"); exit 1 }
        }
        $mods = [byte[]]@($codes | Select-Object -SkipLast 1)
        [Win]::Press($mods, [byte]($codes | Select-Object -Last 1))
    }
}
