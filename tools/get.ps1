# Status Quo, one-paste installer for Windows 10 and 11 (SQ-46). In PowerShell:
#
#   irm https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.ps1 | iex
#
# Downloads status-quo-setup.exe from the latest release, checks its SHA-256 checksum and runs it.
$ErrorActionPreference = 'Stop'
$base = if ($env:SQ_RELEASE_BASE) { $env:SQ_RELEASE_BASE } else { 'https://github.com/yujideluca/status-quo/releases/latest/download' }
$dir = Join-Path $env:TEMP ('status-quo-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
$exe = Join-Path $dir 'status-quo-setup.exe'
try {
    Write-Host '==> Status Quo: download'
    Invoke-WebRequest "$base/status-quo-setup.exe" -OutFile $exe -UseBasicParsing
    $expected = ((Invoke-WebRequest "$base/status-quo-setup.exe.sha256" -UseBasicParsing).Content -split '\s+')[0].Trim().ToLower()
    $actual = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { throw 'checksum does not match: the download is damaged or was changed. Nothing was installed.' }
    Write-Host '  [ok]  checksum ok'
    & $exe
} catch {
    Write-Host "  [x]   $($_.Exception.Message)"
    Write-Host '        No release is published yet, or the network failed. See the README to build it from source.'
    exit 1
} finally {
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
}
