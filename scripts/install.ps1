[CmdletBinding()]
param(
  [string]$Version = "latest",
  [switch]$Help
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = "srosthai/ServerDesk"
$BaseUrl = if ($env:SERVERDESK_BASE_URL) {
  $env:SERVERDESK_BASE_URL
} else {
  "https://github.com/$Repo/releases/download"
}

function Write-Usage {
  @"
Install ServerDesk from a GitHub Release.

Usage:
  powershell -ExecutionPolicy Bypass -File install.ps1 [-Version vX.Y.Z]

Options:
  -Version VERSION  Release tag to install. Defaults to the latest release.
  -Help             Show this help.

The script downloads the NSIS installer and SHA256SUMS, verifies the checksum,
and runs the per-user installer silently.
"@
}

if ($Help) {
  Write-Usage
  exit 0
}

if ($Version -eq "latest") {
  $release = Invoke-RestMethod `
    -Uri "https://api.github.com/repos/$Repo/releases/latest" `
    -Headers @{ "User-Agent" = "ServerDesk-Installer" }
  $Version = $release.tag_name
}

if (-not $Version.StartsWith("v")) {
  $Version = "v$Version"
}

if ($Version -eq "v" -or $Version -eq "latest") {
  throw "Could not determine the ServerDesk release. Pass -Version v0.1.0."
}

$versionNumber = $Version.Substring(1)
$asset = "ServerDesk_${versionNumber}_x64-setup.exe"
$base = "$BaseUrl/$Version"
$tempDir = Join-Path $env:TEMP ("serverdesk-install-" + [guid]::NewGuid().ToString("N"))

try {
  New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
  $installer = Join-Path $tempDir $asset
  $sums = Join-Path $tempDir "SHA256SUMS"

  Write-Host "Downloading ServerDesk $Version..."
  Invoke-WebRequest -Uri "$base/$asset" -OutFile $installer -UseBasicParsing
  Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile $sums -UseBasicParsing

  $line = Get-Content -Path $sums |
    Where-Object { ($_ -split '\s+')[1] -eq $asset } |
    Select-Object -First 1
  if (-not $line) {
    throw "SHA256SUMS does not contain $asset"
  }

  $expected = ($line -split '\s+')[0].ToLowerInvariant()
  $actual = (Get-FileHash -Path $installer -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expected) {
    throw "Checksum mismatch for $asset"
  }

  Write-Host "Installing ServerDesk per-user..."
  $process = Start-Process -FilePath $installer -ArgumentList "/S" -Wait -PassThru
  if ($process.ExitCode -ne 0) {
    throw "Installer exited with code $($process.ExitCode)"
  }

  Write-Host "ServerDesk $Version installed."
  Write-Host "If Windows SmartScreen blocked the unsigned installer, use a signed release or allow it in Windows Security."
} finally {
  Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
