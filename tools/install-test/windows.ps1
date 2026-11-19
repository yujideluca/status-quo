# Real install test on Windows (SQ-47). Nothing is simulated: the one-paste command from the
# README is run as written, in the PowerShell named by -Shell, and the real installer installs
# and starts the real panel.
#
#   powershell -File windows.ps1 -Shell powershell -Line 'irm http://127.0.0.1:8017/get.ps1 | iex' -Base http://127.0.0.1:8017
#
# -Line is the install line of the README (tools/install-test/readme-command.sh prints it). With -Base, the installer is taken from a local server (a
# build under test); without it, from the published release. The last line of a passing test
# is PROVEN; any failure ends with exit code 1.
param(
    [Parameter(Mandatory = $true)][string]$Line,
    [string]$Shell = 'powershell',
    [string]$Base = ''
)
$ErrorActionPreference = 'Stop'
function Fail($why) { Write-Host "`nFAILED: $why"; exit 1 }
function Panel { @(Get-Process -Name 'status-quo' -ErrorAction SilentlyContinue) }

$installed = Join-Path $env:LOCALAPPDATA 'status-quo\bin\status-quo.exe'
Write-Host "######## Windows: $((Get-CimInstance Win32_OperatingSystem).Caption), shell: $Shell"
if ($Base) { $env:SQ_RELEASE_BASE = $Base } else { Remove-Item Env:SQ_RELEASE_BASE -ErrorAction SilentlyContinue }

Write-Host "`n######## The one-paste command, as written in the README"
# The command runs in a new window-less shell, exactly as pasted; the shell must survive it.
$run = "$Line; Write-Host SHELL-STILL-OPEN; exit `$LASTEXITCODE"
# The output goes through a pipe on purpose: an installer that leaves the panel holding the
# pipe would make this wait forever (it did, before the panel was given its own streams).
$ErrorActionPreference = 'Continue'
$log = & $Shell -NoProfile -ExecutionPolicy Bypass -Command $run 2>&1 | Out-String
$code = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
Write-Host $log
if ($code -ne 0) { Fail "the command ended with code $code" }
if ($log -notmatch 'SHELL-STILL-OPEN') { Fail 'the command closed the PowerShell that ran it' }
if ($log -notmatch 'checksum ok') { Fail 'the checksum was not verified' }
if (-not (Test-Path $installed)) {
    $found = Get-ChildItem -Path $env:LOCALAPPDATA -Recurse -Filter 'status-quo.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $found) { Fail 'status-quo.exe was not installed' }
    $installed = $found.FullName
}
Write-Host "installed: $installed"

Write-Host "`n######## The panel must be running"
Start-Sleep -Seconds 5
if ((Panel).Count -eq 0) { Fail 'no Status Quo process is running after the install' }
Write-Host "running: $((Panel | ForEach-Object { $_.Id }) -join ', ')"

Write-Host "`n######## Compatibility check"
# The program has no console window: without a pipe PowerShell would not wait for it and
# would leave the exit code of an earlier command in $LASTEXITCODE.
$report = & $installed doctor | Out-String
$code = $LASTEXITCODE
Write-Host $report
if ($code -ne 0) { Fail 'the compatibility check reports a blocking problem' }
if ($report -notmatch 'Ready to run|Everything is ready') { Fail 'the compatibility check printed no verdict' }

Write-Host "`n######## The panel sits on the desktop background; the floating window is a setting (SQ-48)"
if ($report -notmatch 'windows-desktop') { Fail 'the compatibility check does not report the desktop placement' }
Add-Type -AssemblyName System.Windows.Forms
Add-Type -Namespace SqTest -Name Win -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)] public struct Spot { public int X; public int Y; }
public delegate bool EnumProc(System.IntPtr hwnd, System.IntPtr lParam);
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, System.IntPtr lParam);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr hwnd, out uint pid);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr hwnd);
[StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
[DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr hwnd, out Rect rect);
[DllImport("user32.dll")] public static extern bool GetClientRect(System.IntPtr hwnd, out Rect rect);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(System.IntPtr hwnd, System.Text.StringBuilder text, int max);
[DllImport("user32.dll")] public static extern System.IntPtr WindowFromPoint(Spot point);
[DllImport("user32.dll")] public static extern System.IntPtr GetAncestor(System.IntPtr hwnd, uint flags);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, System.UIntPtr extra);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr hwnd);
[DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern System.IntPtr GetWindowLongPtr(System.IntPtr hwnd, int index);
[DllImport("user32.dll")] public static extern System.IntPtr GetWindow(System.IntPtr hwnd, uint which);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(System.IntPtr hwnd, System.Text.StringBuilder text, int max);
[DllImport("user32.dll")] public static extern bool IsIconic(System.IntPtr hwnd);
[DllImport("user32.dll")] public static extern bool PostMessageW(System.IntPtr hwnd, uint message, System.IntPtr w, System.IntPtr l);
// The visible console window whose title contains `part` (zero when there is none).
public static System.IntPtr ConsoleWindow(string part) {
    var found = System.IntPtr.Zero;
    EnumWindows((hwnd, l) => {
        var cls = new System.Text.StringBuilder(64); GetClassName(hwnd, cls, 64);
        var title = new System.Text.StringBuilder(512); GetWindowText(hwnd, title, 512);
        if (IsWindowVisible(hwnd) && cls.ToString() == "ConsoleWindowClass" && title.ToString().Contains(part)) { found = hwnd; return false; }
        return true;
    }, System.IntPtr.Zero);
    return found;
}
// The class name of a window.
public static string ClassOf(System.IntPtr hwnd) {
    var text = new System.Text.StringBuilder(256);
    GetClassName(hwnd, text, 256);
    return text.ToString();
}
// The class name of the window that owns `hwnd` ("" when it has no owner).
public static string OwnerClass(System.IntPtr hwnd) {
    var owner = GetWindow(hwnd, 4);
    if (owner == System.IntPtr.Zero) return "";
    var text = new System.Text.StringBuilder(256);
    GetClassName(owner, text, 256);
    return text.ToString();
}
// The visible "Status Quo" window of one process, and where it is on the screen.
public static System.IntPtr PanelWindow(uint wanted, out Rect place) {
    var result = System.IntPtr.Zero; var where = new Rect();
    EnumWindows(delegate(System.IntPtr hwnd, System.IntPtr l) {
        var text = new System.Text.StringBuilder(256);
        GetWindowText(hwnd, text, 256);
        uint pid; GetWindowThreadProcessId(hwnd, out pid);
        if (IsWindowVisible(hwnd) && pid == wanted && text.ToString() == "Status Quo") { result = hwnd; GetWindowRect(hwnd, out where); return false; }
        return true;
    }, System.IntPtr.Zero);
    place = where; return result;
}
// Windows lists its windows from the front to the back: true when `front` comes before `back`.
public static bool InFrontOf(System.IntPtr front, System.IntPtr back) {
    int index = 0, frontAt = -1, backAt = -1;
    EnumWindows(delegate(System.IntPtr hwnd, System.IntPtr l) {
        if (hwnd == front) frontAt = index;
        if (hwnd == back) backAt = index;
        index++; return true;
    }, System.IntPtr.Zero);
    return frontAt >= 0 && backAt >= 0 && frontAt < backAt;
}
// A real left click at a point of the screen.
public static void Click(int x, int y) {
    SetCursorPos(x, y); System.Threading.Thread.Sleep(200);
    mouse_event(2, 0, 0, 0, System.UIntPtr.Zero); System.Threading.Thread.Sleep(80);
    mouse_event(4, 0, 0, 0, System.UIntPtr.Zero);
}
public static string Panels() {
    var found = new System.Collections.Generic.List<string>();
    EnumWindows(delegate(System.IntPtr hwnd, System.IntPtr l) {
        var text = new System.Text.StringBuilder(256);
        GetWindowText(hwnd, text, 256);
        if (IsWindowVisible(hwnd) && text.ToString() == "Status Quo") {
            uint pid; GetWindowThreadProcessId(hwnd, out pid);
            Rect outer, inner; GetWindowRect(hwnd, out outer); GetClientRect(hwnd, out inner);
            found.Add(pid + ":" + ((outer.Bottom - outer.Top) - (inner.Bottom - inner.Top)));
        }
        return true;
    }, System.IntPtr.Zero);
    return string.Join(",", found.ToArray());
}
'@
# An ordinary window is taller than its content by its title bar and borders; a frameless
# window is exactly as tall as its content. (The window style cannot tell them apart.)
function TitleBar {
    $ids = @(Panel | ForEach-Object { $_.Id })
    foreach ($entry in ([SqTest.Win]::Panels() -split ',' | Where-Object { $_ })) {
        $processId, $frame = $entry -split ':'
        if ($ids -contains [int]$processId) { return ([int]$frame -ge 8) }
    }
    return $null
}
function WaitTitleBar($wanted) {
    foreach ($second in 1..30) { if ((TitleBar) -eq $wanted) { return $true }; Start-Sleep -Seconds 1 }
    return $false
}
if (-not (WaitTitleBar $false)) { Fail "after the install the panel is not a frameless window on the desktop (title bar: $(TitleBar))" }
Write-Host 'default: no title bar, on the desktop background'
# Behind the other windows: another window is opened over the left half of the panel, then
# the panel is clicked for real (a click is what makes Windows bring a window to the front).
# The other window must stay in front, and the screen must show it where the two overlap.
# The frameless panel lies inside the free part of the screen (never under the taskbar) and
# at its right edge, where a first start puts it.
function InFreeArea($label) {
    $area = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    $place = New-Object SqTest.Win+Rect
    $problem = 'the panel window was not found'
    foreach ($second in 1..20) {
        foreach ($process in @(Panel)) {
            if ([SqTest.Win]::PanelWindow([uint32]$process.Id, [ref]$place) -ne [IntPtr]::Zero) {
                $where = "panel $($place.Left),$($place.Top) to $($place.Right),$($place.Bottom); free area $($area.Left),$($area.Top) to $($area.Right),$($area.Bottom)"
                if ($place.Top -lt $area.Top -or $place.Left -lt $area.Left -or $place.Right -gt $area.Right -or $place.Bottom -gt $area.Bottom) { $problem = "the panel is not inside the free part of the screen ($where)" }
                elseif ($place.Right -lt $area.Right - 48) { $problem = "the panel is not at the right edge of the screen ($where)" }
                else { Write-Host "$label`: the panel is inside the free part of the screen, at its right edge ($where)"; return }
            }
        }
        Start-Sleep -Seconds 1
    }
    Fail "$label`: $problem"
}
function BehindOtherWindows($label) {
    $place = New-Object SqTest.Win+Rect
    $panel = [IntPtr]::Zero
    foreach ($process in @(Panel)) {
        $panel = [SqTest.Win]::PanelWindow([uint32]$process.Id, [ref]$place)
        if ($panel -ne [IntPtr]::Zero) { break }
    }
    if ($panel -eq [IntPtr]::Zero) { Fail "$label`: the panel window was not found" }
    $width = $place.Right - $place.Left; $height = $place.Bottom - $place.Top
    $other = New-Object Windows.Forms.Form
    $other.Text = 'Another window'; $other.StartPosition = 'Manual'
    $other.SetBounds($place.Left, $place.Top, [int]($width / 2), $height)
    # Raised above every ordinary window (the test's own console may lie there), then ordinary again.
    $other.Show(); $other.TopMost = $true; $other.TopMost = $false; $other.Activate(); [Windows.Forms.Application]::DoEvents()
    Start-Sleep -Seconds 1
    try {
        foreach ($round in 1..2) {
            [SqTest.Win]::Click($place.Right - 8, $place.Bottom - 8)
            [void][SqTest.Win]::SetForegroundWindow($panel)
            foreach ($tick in 1..10) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 150 }
            if (-not [SqTest.Win]::InFrontOf($other.Handle, $panel)) { Fail "$label`: after a click on the panel it is in front of another window (click $round)" }
            $point = New-Object SqTest.Win+Spot
            $point.X = [int]($place.Left + $width / 4)
            $point.Y = [int]($place.Top + $height / 2)
            $seen = [SqTest.Win]::GetAncestor([SqTest.Win]::WindowFromPoint($point), 2)
            if ($seen -ne $other.Handle) { Fail "$label`: where the two windows overlap, the screen does not show the other window (click $round)" }
        }
    } finally { $other.Close(); $other.Dispose() }
    Write-Host "$label`: the panel stays behind another window, also after being clicked"
}
# Part of the desktop, not an application window. To Windows an application window is one with
# a title bar or a resizing edge that is listed in Alt+Tab; the panel has neither, is a "tool
# window" (never listed) and is owned by the desktop window (class Progman). And the real
# thing: "Show desktop" (what Win+D does) is switched on with another window open. The other
# window must go away and the panel must stay; switched again, the other window must be back.
function PartOfDesktop($label) {
    $place = New-Object SqTest.Win+Rect
    $panel = [IntPtr]::Zero
    foreach ($process in @(Panel)) {
        $panel = [SqTest.Win]::PanelWindow([uint32]$process.Id, [ref]$place)
        if ($panel -ne [IntPtr]::Zero) { break }
    }
    if ($panel -eq [IntPtr]::Zero) { Fail "$label`: the panel window was not found" }
    $style = [SqTest.Win]::GetWindowLongPtr($panel, -16).ToInt64()
    $extended = [SqTest.Win]::GetWindowLongPtr($panel, -20).ToInt64()
    $owner = [SqTest.Win]::OwnerClass($panel)
    $read = 'style {0:x}, extended style {1:x}, owner "{2}"' -f $style, $extended, $owner
    # 0xC00000 title bar, 0x40000 resizing edge, 0x20000 and 0x10000 minimize and maximize.
    if ($style -band 0xC70000) { Fail "$label`: the panel still has application window styles ($read)" }
    # 0x80 tool window (not in Alt+Tab, not on the taskbar), 0x40000 forces a taskbar button.
    if (-not ($extended -band 0x80) -or ($extended -band 0x40000)) { Fail "$label`: the panel would be listed in Alt+Tab ($read)" }
    if ($owner -ne 'Progman') { Fail "$label`: the panel is not owned by the desktop window ($read)" }
    $other = New-Object Windows.Forms.Form
    $other.Text = 'Another window'; $other.StartPosition = 'Manual'
    $other.SetBounds(40, 40, 320, 240)
    $other.Show(); $other.Activate()
    foreach ($tick in 1..10) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 }
    $shell = New-Object -ComObject Shell.Application
    $middle = New-Object SqTest.Win+Spot
    $middle.X = [int](($place.Left + $place.Right) / 2); $middle.Y = [int](($place.Top + $place.Bottom) / 2)
    try {
        $shell.ToggleDesktop()
        $gone = $false
        foreach ($tick in 1..50) {
            [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100
            if ([SqTest.Win]::IsIconic($other.Handle)) { $gone = $true; break }
        }
        if (-not $gone) { Fail "$label`: Show desktop did not put the other window away, so the check cannot run" }
        foreach ($tick in 1..10) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 }
        if (-not [SqTest.Win]::IsWindowVisible($panel) -or [SqTest.Win]::IsIconic($panel)) { Fail "$label`: Show desktop hid the panel" }
        if ([SqTest.Win]::GetAncestor([SqTest.Win]::WindowFromPoint($middle), 2) -ne $panel) {
            # What covers the panel, and whether that is still so three seconds later.
            $seen = { $top = [SqTest.Win]::GetAncestor([SqTest.Win]::WindowFromPoint($middle), 2)
                $owner = [uint32]0; [void][SqTest.Win]::GetWindowThreadProcessId($top, [ref]$owner)
                $title = New-Object Text.StringBuilder 256; [void][SqTest.Win]::GetWindowText($top, $title, 256)
                $program = (Get-Process -Id $owner -ErrorAction SilentlyContinue).ProcessName
                'window there: class "{0}", title "{1}", program "{2}", owner "{3}"; panel owner "{4}"; panel shown: {5}' -f [SqTest.Win]::ClassOf($top), $title, $program, [SqTest.Win]::OwnerClass($top), [SqTest.Win]::OwnerClass($panel), ($top -eq $panel) }
            $first = & $seen
            foreach ($tick in 1..30) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 }
            Fail "$label`: after Show desktop the screen does not show the panel ($first; three seconds later: $(& $seen))"
        }
        $shell.ToggleDesktop()
        $back = $false
        foreach ($tick in 1..50) {
            [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100
            if (-not [SqTest.Win]::IsIconic($other.Handle)) { $back = $true; break }
        }
        if (-not $back) { Fail "$label`: Show desktop a second time did not bring the other window back" }
    } finally { $other.Close(); $other.Dispose() }
    Write-Host "$label`: part of the desktop ($read); Show desktop leaves it and a second one brings the other windows back"
}
InFreeArea 'default'
BehindOtherWindows 'default'
PartOfDesktop 'default'
$said = & $installed floating on | Out-String
Write-Host $said
if (-not (WaitTitleBar $true)) { Fail 'with the Floating window setting on, the panel did not become an ordinary window' }
Write-Host 'Floating window on: an ordinary window with a title bar'
$said = & $installed floating off | Out-String
Write-Host $said
if (-not (WaitTitleBar $false)) { Fail 'with the Floating window setting off, the panel did not go back to the desktop background' }
Write-Host 'Floating window off: back on the desktop background'
InFreeArea 'after Floating window off'
BehindOtherWindows 'after Floating window off'
PartOfDesktop 'after Floating window off'
if ((Panel).Count -eq 0) { Fail 'the panel is not running after switching the setting' }

Write-Host "`n######## The panel outlives the terminal that started it (SQ-49)"
# Windows ends every program attached to a console when that console's window is closed. The
# panel is started here from a real console window, then that window is closed the way its X
# button does it: the console and its command must be gone and the panel must still run.
& $installed stop | Out-Null
Start-Sleep -Seconds 2
if ((Panel).Count -ne 0) { Fail 'the panel did not stop' }
$console = Start-Process conhost.exe -ArgumentList "cmd /c `"title sq-console-test & `"$installed`" launch --verify & ping -n 600 127.0.0.1 >nul`"" -PassThru
# The ping starts when `launch --verify` has returned: from then on only the panel and its
# collector carry the program's name.
$waiting = $null
foreach ($tick in 1..80) {
    $waiting = Get-CimInstance Win32_Process -Filter "Name = 'PING.EXE'" | Where-Object { $_.CommandLine -match '-n 600' }
    if ($waiting) { break }
    Start-Sleep -Milliseconds 500
}
if (-not $waiting) { Fail 'launch --verify did not return in the console' }
$before = @(Panel | ForEach-Object { $_.Id })
if ($before.Count -eq 0) { Fail 'the panel did not start from a console' }
# The console window is found by the title set above (Windows reports it under the process
# running in it, not under the console host) and gets the message its X button sends (0x10).
$window = [IntPtr]::Zero
foreach ($tick in 1..20) {
    $window = [SqTest.Win]::ConsoleWindow('sq-console-test')
    if ($window -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 250
}
if ($window -eq [IntPtr]::Zero) { Fail 'the console window was not found' }
[void][SqTest.Win]::PostMessageW($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
foreach ($tick in 1..40) { if ($console.HasExited) { break }; Start-Sleep -Milliseconds 500 }
if (-not $console.HasExited) { Fail 'the console did not close' }
Start-Sleep -Seconds 3
if (Get-CimInstance Win32_Process -Filter "Name = 'PING.EXE'" | Where-Object { $_.CommandLine -match '-n 600' }) { Fail 'closing the console did not end the command running in it: the test proves nothing' }
$after = @(Panel | ForEach-Object { $_.Id })
foreach ($id in $before) { if ($after -notcontains $id) { Fail "closing the terminal that started the panel ended it (process $id)" } }
Write-Host "the console window is closed, its command is gone, the panel still runs: $($after -join ', ')"

Write-Host "`n######## Uninstall, with the line of the README"
$readme = Join-Path $PSScriptRoot '..\..\README.md'
$remove = @(Get-Content $readme | Where-Object { $_ -match '^& "\$env:LOCALAPPDATA.*status-quo\.exe" uninstall' })[0]
if (-not $remove) { Fail 'the README has no uninstall line for Windows' }
Write-Host "> $remove"
$ErrorActionPreference = 'Continue'
$log = & $Shell -NoProfile -ExecutionPolicy Bypass -Command "$remove; exit `$LASTEXITCODE" 2>&1 | Out-String
$code = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
Write-Host $log
if ($code -ne 0) { Fail "uninstall ended with code $code" }
if ($log -notmatch 'Uninstalled Status Quo') { Fail 'uninstall did not say it uninstalled' }
$startup = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Startup\status-quo.vbs'
if (Test-Path $installed) { Fail "the program is still at $installed after uninstall" }
if (Test-Path $startup) { Fail 'the sign-in start entry is still there after uninstall' }
if ((Panel).Count -ne 0) { Fail 'the panel is still running after uninstall' }
# The moved-aside program file and its folder are deleted a moment after uninstall ends.
$bin = Split-Path $installed
foreach ($second in 1..20) { if (-not (Test-Path $bin)) { break }; Start-Sleep -Seconds 1 }
if (Test-Path $bin) { Fail "leftovers after uninstall: $((Get-ChildItem $bin -Force | ForEach-Object Name) -join ', ')" }

Write-Host "`nPROVEN on $((Get-CimInstance Win32_OperatingSystem).Caption) with $Shell"
