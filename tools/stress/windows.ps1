# Stress test of the Windows desktop panel (SQ-48, SQ-49, SQ-50): real clicks, real keys and
# real restarts against the INSTALLED Status Quo, on the visible desktop of a Windows 10 or 11
# machine. It moves the pointer, presses Win+D, restarts Windows Explorer and rewrites the
# layer and Floating window settings: run it on a test machine, not on a desktop in use.
#
#   powershell -ExecutionPolicy Bypass -File tools\stress\windows.ps1            every round
#   powershell -ExecutionPolicy Bypass -File tools\stress\windows.ps1 -Only matrix,place
#
# Rounds: matrix (every layer with and without Floating window: styles, owner, stacking after
# clicks, Show desktop twice), desktop (Show desktop 15 times), windows (twelve windows over the
# panel), floating (switch on and off six times), place (the window starts where it was left),
# explorer (Windows Explorer restarted three times), restart (launch while running, stop and
# start), clicks (300 fast clicks, then three idle minutes with memory compared).
# It must run without a console window in front of the panel, for example as a scheduled task
# through `conhost.exe --headless`. Every check prints PASS or FAIL; the exit code is the
# number of failures.
param([string]$Only = '')
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace L -Name W -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool IsIconic(System.IntPtr h);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr h, out Rect r);
[DllImport("user32.dll")] public static extern System.IntPtr GetWindow(System.IntPtr h, uint which);
[DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern System.IntPtr GetLong(System.IntPtr h, int index);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(System.IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern System.IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, System.UIntPtr extra);
[DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, System.UIntPtr extra);
public static string Class(System.IntPtr h) { var c = new System.Text.StringBuilder(100); GetClassNameW(h, c, 100); return c.ToString(); }
public static void Key(byte k) { keybd_event(k, 0, 0, System.UIntPtr.Zero); keybd_event(k, 0, 2, System.UIntPtr.Zero); }
public static void WinD() { keybd_event(0x5B, 0, 0, System.UIntPtr.Zero); keybd_event(0x44, 0, 0, System.UIntPtr.Zero); keybd_event(0x44, 0, 2, System.UIntPtr.Zero); keybd_event(0x5B, 0, 2, System.UIntPtr.Zero); }
// Windows from top to bottom: is `a` above `b`?
public static bool Above(System.IntPtr a, System.IntPtr b) { for (var h = GetWindow(a, 2); h != System.IntPtr.Zero; h = GetWindow(h, 2)) if (h == b) return true; return false; }
'@
[void][L.W]::SetProcessDPIAware()
function Pump($n) { foreach ($i in 1..$n) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 } }
function Fg { [L.W]::Class([L.W]::GetForegroundWindow()) }
function PanelWindow { [L.W]::FindWindowW('Window Class', 'Status Quo') }
function Point($x, $y) { [void][L.W]::SetCursorPos($x - 2, $y); Start-Sleep -Milliseconds 60; [L.W]::mouse_event(1, 1, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 60; [void][L.W]::SetCursorPos($x, $y); [L.W]::mouse_event(1, 0, 0, 0, [UIntPtr]::Zero); Pump 2 }
function Click($x, $y) { Point $x $y; [L.W]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 90; [L.W]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Pump 6 }
function Wheel($x, $y, $notches) { Point $x $y; $d = if ($notches -lt 0) { -120 } else { 120 }; foreach ($i in 1..[Math]::Abs($notches)) { [L.W]::mouse_event(0x0800, 0, 0, $d, [UIntPtr]::Zero); Start-Sleep -Milliseconds 60 }; Pump 5 }
function Keys($text) { foreach ($ch in $text.ToCharArray()) { [L.W]::Key([byte][char]::ToUpper($ch)); Start-Sleep -Milliseconds 60 }; Pump 4 }
function Shot($name) { $b = [Windows.Forms.Screen]::PrimaryScreen.Bounds; $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height; $g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen(0, 0, 0, 0, $bmp.Size); $bmp.Save("$env:TEMP\status-quo-stress\$name.png"); $g.Dispose(); $bmp.Dispose() }
function Pixel($x, $y) { $bmp = New-Object Drawing.Bitmap 1, 1; $g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size); $c = $bmp.GetPixel(0, 0); $g.Dispose(); $bmp.Dispose(); "{0:x2}{1:x2}{2:x2}" -f $c.R, $c.G, $c.B }
function Facts { $p = PanelWindow; if ($p -eq [IntPtr]::Zero) { return 'no panel window' }; $r = New-Object L.W+Rect; [void][L.W]::GetWindowRect($p, [ref]$r); 'style {0:x} ex {1:x} owner {2} rect {3},{4},{5},{6} visible {7} iconic {8}' -f [L.W]::GetLong($p, -16).ToInt64(), [L.W]::GetLong($p, -20).ToInt64(), [L.W]::Class([L.W]::GetWindow($p, 4)), $r.Left, $r.Top, $r.Right, $r.Bottom, [L.W]::IsWindowVisible($p), [L.W]::IsIconic($p) }
function Other($x = 40, $y = 120, $w = 380, $h = 300) { $f = New-Object Windows.Forms.Form; $f.Text = 'Another window'; $f.StartPosition = 'Manual'; $f.SetBounds($x, $y, $w, $h); $f.BackColor = 'Orange'; $f.Show(); $f.TopMost = $true; $f.TopMost = $false; $f.Activate(); Pump 8; $f }
New-Item -ItemType Directory -Force $env:TEMP\status-quo-stress | Out-Null
$exe = "$env:LOCALAPPDATA\status-quo\bin\status-quo.exe"
$cfg = "$env:LOCALAPPDATA\status-quo\config.json"
$script:fails = 0
function Check($ok, $name, $detail = '') { if ($ok) { "PASS $name $detail" } else { $script:fails++; "FAIL $name $detail" } }
function Procs { @(Get-CimInstance Win32_Process -Filter "Name = 'status-quo.exe'") }
function Panels { @(Procs | ? { $_.CommandLine -notmatch ' (collect|launch|stop|floating|doctor)' }) }
function WaitPanel($seconds = 30) { foreach ($i in 1..($seconds * 4)) { if ((PanelWindow) -ne [IntPtr]::Zero -and [L.W]::IsWindowVisible((PanelWindow))) { Pump 15; return $true }; Start-Sleep -Milliseconds 250 }; $false }
function StopAll { & $exe stop | Out-Null; Start-Sleep 1; Get-Process status-quo -ea 0 | Stop-Process -Force -ea 0; Start-Sleep 1 }
function Launch { $p = Start-Process $exe -ArgumentList 'launch','--verify' -PassThru -WindowStyle Hidden; $p.WaitForExit(60000) | Out-Null; [void](WaitPanel) }
function SetConfig($layer, $floating) {
  StopAll
  $t = Get-Content $cfg -Raw
  $t = $t -replace '"layer":\s*"[^"]*"', "`"layer`":`"$layer`""
  $t = $t -replace '"floating":\s*(true|false)', "`"floating`":$floating"
  [IO.File]::WriteAllText($cfg, $t)
  Launch
}
function Rect { $r = New-Object L.W+Rect; [void][L.W]::GetWindowRect((PanelWindow), [ref]$r); $r }
function Style { [L.W]::GetLong((PanelWindow), -16).ToInt64() }
function Ex { [L.W]::GetLong((PanelWindow), -20).ToInt64() }
function Owner { [L.W]::Class([L.W]::GetWindow((PanelWindow), 4)) }
# Show desktop twice with another window open: it must go and come back; returns a text.
function DesktopTwice($other, $panelStays) {
  [L.W]::WinD(); Pump 25
  $gone = [L.W]::IsIconic($other.Handle)
  $panelThere = [L.W]::IsWindowVisible((PanelWindow)) -and -not [L.W]::IsIconic((PanelWindow))
  [L.W]::WinD(); Pump 30
  $back = -not [L.W]::IsIconic($other.Handle)
  $panelBack = [L.W]::IsWindowVisible((PanelWindow)) -and -not [L.W]::IsIconic((PanelWindow))
  $ok = $gone -and $back -and $panelBack -and ($panelThere -eq $panelStays)
  @($ok, "gone=$gone panelDuring=$panelThere back=$back panelAfter=$panelBack")
}
function Res { $p = Get-Process -Id @(Panels)[0].ProcessId; 'handles {0} memory {1:n0} MB threads {2}' -f $p.HandleCount, ($p.WorkingSet64 / 1MB), $p.Threads.Count }
function Run($name) { -not $Only -or $Only -split ',' -contains $name }

if (Run 'matrix') {
"######## Every layer, with and without Floating window"
foreach ($floating in 'false', 'true') { foreach ($layer in 'behind', 'normal', 'above') {
  SetConfig $layer $floating
  $tag = "layer=$layer floating=$floating"
  Check (@(Panels).Count -eq 1) "$tag one panel" "$(Facts)"
  $style = Style; $ex = Ex; $owner = Owner; $r = Rect
  $desk = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
  Check ($r.Left -ge $desk.Left -and $r.Top -ge $desk.Top -and $r.Right -le $desk.Right + 8 -and $r.Bottom -le $desk.Bottom + 8) "$tag inside the free screen area"
  if ($floating -eq 'true') {
    Check (($style -band 0xC00000) -eq 0xC00000 -and -not ($ex -band 0x80)) "$tag ordinary window with a title bar"
    Check ($owner -eq '') "$tag no owner" $owner
  } else {
    Check (-not ($style -band 0xC70000) -and ($ex -band 0x80) -and -not ($ex -band 0x40000)) "$tag no title bar, not in Alt+Tab"
    Check ($owner -eq $(if ($layer -eq 'behind') { 'Progman' } else { '' })) "$tag owner" "'$owner'"
  }
  Check ([bool]($ex -band 8) -eq ($layer -eq 'above')) "$tag always-on-top only in Above"
  # Another window over the left half of the panel; click the panel's free corner three times.
  $other = Other $r.Left ($r.Top + 60) 300 300
  $x = $r.Right - 12; $y = $r.Bottom - 12
  foreach ($i in 1..3) { Click $x $y }
  $front = [L.W]::Above((PanelWindow), $other.Handle)
  $want = -not ($floating -eq 'false' -and $layer -eq 'behind')
  Check ($front -eq $want) "$tag after clicks the panel is $(if ($want) { 'in front of' } else { 'behind' }) another window" "panelInFront=$front fg=$(Fg)"
  if ($layer -ne 'above') { $other.Activate(); $other.TopMost = $true; $other.TopMost = $false; Pump 5; Check (-not [L.W]::Above((PanelWindow), $other.Handle)) "$tag another window can come in front again" }
  $res = DesktopTwice $other ($floating -eq 'false')
  Check $res[0] "$tag show desktop twice" $res[1]
  Click $x $y
  $res = DesktopTwice $other ($floating -eq 'false')
  Check $res[0] "$tag show desktop twice after a click on the panel" $res[1]
  $other.Close(); $other.Dispose()
} }
SetConfig 'behind' 'false'
}

if (Run 'desktop') {
"######## Show desktop 15 times in a row, clicking the panel in between"
$r = Rect; $other = Other 60 80 320 240; $bad = 0; $why = ''
"before: $(Res)"
foreach ($i in 1..15) { if ($i % 2) { Click ($r.Right - 12) ($r.Bottom - 12) } else { $other.Activate(); Pump 3 }; $res = DesktopTwice $other $true; if (-not $res[0]) { $bad++; $why = "round $i $($res[1])" } }
Check ($bad -eq 0) 'show desktop 15 rounds' "failures=$bad $why"
$other.Close()
"after: $(Res)"
}

if (Run 'windows') {
"######## Twelve windows over the panel"
$r = Rect; $all = @()
foreach ($i in 1..12) { $all += Other ($r.Left - 100 + 30 * $i) ($r.Top + 20 * $i) 320 240 }
foreach ($i in 1..10) { Click ($r.Right - 12) ($r.Bottom - 12) }
$above = @($all | ? { [L.W]::Above($_.Handle, (PanelWindow)) }).Count
Check ($above -eq 12) 'panel behind all twelve after ten clicks' "windows above the panel: $above"
$mid = Pixel ($r.Left + 60) ($r.Top + 200)
Check ($mid -match '^f[c-f]a[3-6]0[0-4]$') 'the screen shows the other window where they overlap' "pixel $mid"
$all | % { $_.Close() }
Pump 5
}

if (Run 'floating') {
"######## Floating window switched on and off six times"
"before: $(Res)"
foreach ($i in 1..6) {
  & $exe floating on | Out-Null; $ok = $false
  foreach ($t in 1..60) { Start-Sleep -Milliseconds 250; if ((PanelWindow) -ne [IntPtr]::Zero -and ((Style) -band 0xC00000) -eq 0xC00000) { $ok = $true; break } }
  Check $ok "round $i floating on gives a title bar" "$(Facts)"
  & $exe floating off | Out-Null; $ok = $false
  foreach ($t in 1..60) { Start-Sleep -Milliseconds 250; if ((PanelWindow) -ne [IntPtr]::Zero -and -not ((Style) -band 0xC00000) -and (Owner) -eq 'Progman') { $ok = $true; break } }
  Check $ok "round $i floating off is back on the desktop" "$(Facts)"
  Start-Sleep 1
  Check (@(Panels).Count -eq 1 -and @(Procs).Count -eq 2) "round $i one panel and one collector" "processes: $((Procs | % { $_.ProcessId }) -join ',')"
}
"after: $(Res)"
}

if (Run 'place') {
"######## The window starts where it was left (SQ-50)"
function Place { $r = Rect; "$($r.Left),$($r.Top),$($r.Right),$($r.Bottom)" }
foreach ($floating in 'true', 'false') {
  SetConfig 'behind' $floating; Start-Sleep 8; $first = Place
  foreach ($i in 1..3) { StopAll; Launch; Start-Sleep 8; $now = Place; Check ($now -and $now -eq $first) "floating=$floating restart $i keeps the place" "$first then $now" }
}
}

if (Run 'explorer') {
"######## Windows Explorer restarted three times"
foreach ($i in 1..3) {
  $old = [L.W]::FindWindowW('Progman', 'Program Manager')
  Get-Process explorer | Stop-Process -Force; Start-Sleep 3
  if (-not (Get-Process explorer -ea 0)) { Start-Process explorer.exe }
  $ok = $false
  foreach ($t in 1..80) { Start-Sleep -Milliseconds 500; $new = [L.W]::FindWindowW('Progman', 'Program Manager'); if ($new -ne [IntPtr]::Zero -and $new -ne $old -and [L.W]::GetWindow((PanelWindow), 4) -eq $new) { $ok = $true; break } }
  Pump 30
  Check $ok "round $i the panel belongs to the new desktop window" "$(Facts) old=$old new=$([L.W]::FindWindowW('Progman', 'Program Manager')) owner=$([L.W]::GetWindow((PanelWindow), 4)) explorer=$((Get-Process explorer -ea 0 | % Id) -join ',')"
  $other = Other 60 80 320 240
  $res = DesktopTwice $other $true
  Check $res[0] "round $i show desktop twice after the restart" $res[1]
  $other.Close()
}
}

if (Run 'restart') {
"######## Started again while running, stopped and started five times"
foreach ($i in 1..5) { Launch; Check (@(Panels).Count -eq 1) "launch $i while running keeps one panel" "$(@(Panels).Count)" }
foreach ($i in 1..5) {
  & $exe stop | Out-Null; Start-Sleep 2
  Check (@(Panels).Count -eq 0 -and (PanelWindow) -eq [IntPtr]::Zero) "stop $i closes the panel"
  Launch
  Check (@(Panels).Count -eq 1 -and (Owner) -eq 'Progman') "start $i brings it back on the desktop" "$(Facts)"
}
}

if (Run 'clicks') {
"######## 300 fast clicks all over the panel, then 3 minutes idle"
"before: $(Res)"
$r = Rect; $rnd = New-Object Random 7
foreach ($i in 1..300) { [void][L.W]::SetCursorPos($rnd.Next($r.Left + 10, $r.Right - 10), $rnd.Next($r.Top + 60, $r.Bottom - 10)); [L.W]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); [L.W]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 15 }
Pump 20
Check (@(Panels).Count -eq 1 -and [L.W]::IsWindowVisible((PanelWindow))) 'panel alive after 300 clicks' "$(Facts)"
"after clicks: $(Res)"
$first = (Get-Process -Id @(Panels)[0].ProcessId).WorkingSet64
Start-Sleep 180
$p = Get-Process -Id @(Panels)[0].ProcessId
"after 3 minutes idle: $(Res) cpu seconds $([int]$p.CPU)"
Check ($p.WorkingSet64 -lt $first * 1.2 + 20MB) 'memory does not grow while idle' ('{0:n0} MB then {1:n0} MB' -f ($first / 1MB), ($p.WorkingSet64 / 1MB))
$other = Other 60 80 320 240; $res = DesktopTwice $other $true; Check $res[0] 'show desktop twice at the end' $res[1]; $other.Close()
}
"######## failures: $script:fails"
Get-Content "$env:LOCALAPPDATA\status-quo\panel.log" -Tail 3
exit $script:fails
