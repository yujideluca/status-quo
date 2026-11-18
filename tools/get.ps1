# Status Quo, one-paste installer for Windows 10 and 11 (SQ-46, SQ-47). In PowerShell:
#
#   irm https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.ps1 | iex
#
# Downloads status-quo-setup.exe from the latest release, checks its SHA-256 checksum and runs it.
# It works in the Windows PowerShell 5.1 that every Windows 10 and 11 has, and in PowerShell 7.
# Everything runs inside one block and never calls `exit`, so a failure leaves the window open
# with the reason on screen and changes nothing in the PowerShell session that pasted it.
# For tests: SQ_RELEASE_BASE replaces the download address, SQ_SETUP_ARGS adds installer options.
& {
    $ErrorActionPreference = 'Stop'
    # The progress bar of Windows PowerShell makes a download many times slower.
    $ProgressPreference = 'SilentlyContinue'
    # Older Windows 10 builds do not offer TLS 1.2 by default, and GitHub accepts nothing older.
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }

    $base = if ($env:SQ_RELEASE_BASE) { $env:SQ_RELEASE_BASE } else { 'https://github.com/yujideluca/status-quo/releases/latest/download' }
    $dir = Join-Path ([IO.Path]::GetTempPath()) ('status-quo-' + [guid]::NewGuid().ToString('N'))
    $exe = Join-Path $dir 'status-quo-setup.exe'
    $sum = Join-Path $dir 'status-quo-setup.exe.sha256'

    # Three attempts, so one dropped connection does not end the installation.
    function Get-Download($url, $file) {
        foreach ($attempt in 1..3) {
            try { Invoke-WebRequest $url -OutFile $file -UseBasicParsing; return }
            catch { if ($attempt -eq 3) { throw } else { Start-Sleep -Seconds 2 } }
        }
    }

    $step = 'start'
    try {
        if (-not [Environment]::Is64BitOperatingSystem) { throw 'Status Quo needs a 64-bit Windows 10 or 11; this Windows is 32-bit.' }
        New-Item -ItemType Directory -Path $dir | Out-Null
        Write-Host '==> Status Quo: download'
        $step = 'download'
        Get-Download "$base/status-quo-setup.exe" $exe
        Get-Download "$base/status-quo-setup.exe.sha256" $sum
        Write-Host '  [ok]  downloaded status-quo-setup.exe'
        $step = 'checksum'
        # The checksum is read from the saved file: read straight from the web it arrives as raw
        # bytes, not text, and never compares equal.
        $expected = ((Get-Content -LiteralPath $sum -Raw) -split '\s+' | Where-Object { $_ })[0].ToLower()
        $actual = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLower()
        if ($expected -ne $actual) { throw 'checksum does not match: the download is damaged or was changed. Nothing was installed. Paste the line again.' }
        Write-Host '  [ok]  checksum ok'
        $step = 'setup'
        Unblock-File -LiteralPath $exe -ErrorAction SilentlyContinue
        $setupArgs = @('--yes')
        if ($env:SQ_SETUP_ARGS) { $setupArgs += $env:SQ_SETUP_ARGS -split '\s+' | Where-Object { $_ } }
        & $exe @setupArgs
        if ($LASTEXITCODE -ne 0) { throw "the installer stopped with an error (code $LASTEXITCODE); the lines above say why." }
    } catch {
        Write-Host ''
        Write-Host "  [x]   $($_.Exception.Message)" -ForegroundColor Red
        if ($step -eq 'download') {
            Write-Host '        The download did not arrive. Check the internet connection and paste the line again.'
            Write-Host '        Or download status-quo-setup.exe by hand: https://github.com/yujideluca/status-quo/releases/latest'
        }
        $global:LASTEXITCODE = 1
    } finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}
