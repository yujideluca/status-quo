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
