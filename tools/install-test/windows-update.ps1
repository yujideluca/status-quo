# Real self-update test on Windows (SQ-54). The previous release is installed with the README
# line (its installer starts the panel), and the test waits until that panel has found the
# newest published release on GitHub, checked it and installed it by itself. Nothing is
# simulated: only the wait before the first check is shortened (STATUS_QUO_UPDATE_DELAY).
#
#   powershell -File windows-update.ps1 -Line '<README line>' -From v0.1.6 -To v0.1.7
#
# A test that passes prints "PROVEN" as its last line; any failure ends with exit code 1.
param(
    [Parameter(Mandatory = $true)][string]$Line,
    [Parameter(Mandatory = $true)][string]$From,
    [Parameter(Mandatory = $true)][string]$To
)
$ErrorActionPreference = 'Stop'
$manifest = Join-Path $env:LOCALAPPDATA 'status-quo\install.json'
$program = Join-Path $env:LOCALAPPDATA 'status-quo\bin\status-quo.exe'
function Installed {
    if (Test-Path $manifest) { (Get-Content $manifest -Raw | ConvertFrom-Json).version } else { '' }
}
function Fail($message) {
    Write-Host "`nFAILED: $message"
    Get-ChildItem -Path $env:LOCALAPPDATA, $env:APPDATA -Recurse -Filter 'update.log' -ErrorAction SilentlyContinue |
        ForEach-Object { Write-Host "--- $($_.FullName)"; Get-Content $_.FullName -Tail 40 }
    exit 1
}

Write-Host "######## Install $From with the README line"
$env:SQ_RELEASE_BASE = "https://github.com/yujideluca/status-quo/releases/download/$From"
$env:STATUS_QUO_UPDATE_DELAY = '20'
$log = & powershell -NoProfile -ExecutionPolicy Bypass -Command "$Line; exit `$LASTEXITCODE" 2>&1 | Out-String
Write-Host $log
Remove-Item Env:SQ_RELEASE_BASE
if ((Installed) -ne $From.TrimStart('v')) { Fail "install.json names '$(Installed)', not $($From.TrimStart('v'))" }
Write-Host "installed: $(Installed)"

Write-Host "######## Wait for the running panel to update itself to $To"
$deadline = (Get-Date).AddMinutes(10)
while ((Installed) -ne $To.TrimStart('v')) {
    if ((Get-Date) -gt $deadline) { Fail "after 10 minutes install.json still names '$(Installed)', not $($To.TrimStart('v'))" }
    Start-Sleep -Seconds 5
}
Write-Host "install.json names $(Installed)"

Write-Host "######## The new version runs"
Start-Sleep -Seconds 15
$running = Get-Process -Name 'status-quo' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $program }
if (-not $running) { Fail 'no Status Quo process runs after the update' }
$check = & $program update --check 2>&1 | Out-String
Write-Host $check
if ($check -notmatch 'is the newest version') { Fail 'the updated program does not see itself as the newest' }
Get-ChildItem -Path $env:LOCALAPPDATA -Recurse -Filter 'update.log' -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Host "--- $($_.FullName)"; Get-Content $_.FullName }
Write-Host ''
Write-Host 'PROVEN'
