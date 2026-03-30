#
# Install browseros-cli for Windows — downloads the latest release binary.
#
# Usage (PowerShell — save and run):
#   Invoke-WebRequest -Uri "https://cdn.browseros.com/cli/install.ps1" -OutFile install.ps1
#   .\install.ps1
#   .\install.ps1 -Version "0.1.0" -Dir "C:\tools\browseros"
#
# Usage (one-liner, uses env vars for options):
#   & { $env:BROWSEROS_VERSION="0.1.0"; irm https://cdn.browseros.com/cli/install.ps1 | iex }
#

param(
    [string]$Version = "",
    [string]$Dir = ""
)

$ErrorActionPreference = "Stop"

# TLS 1.2 — older PS 5.1 defaults to TLS 1.0
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$CdnBase = "https://cdn.browseros.com/cli"
$Binary = "browseros-cli"

# When piped via irm | iex, param() is ignored — fall back to env vars
if (-not $Version) { $Version = $env:BROWSEROS_VERSION }
if (-not $Dir) { $Dir = if ($env:BROWSEROS_DIR) { $env:BROWSEROS_DIR } else { "$env:LOCALAPPDATA\browseros-cli\bin" } }

# ── Resolve latest version ───────────────────────────────────────────────────

if (-not $Version) {
    Write-Host "Fetching latest version..."
    $Version = (Invoke-WebRequest -Uri "$CdnBase/latest/version.txt" -UseBasicParsing).Content.Trim()
    if (-not $Version) {
        Write-Error "Could not determine latest version. Try: -Version 0.1.0"
        exit 1
    }
}

if ($Version -notmatch '^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$') {
    Write-Error "Unexpected version format: '$Version'"
    exit 1
}

Write-Host "Installing browseros-cli v$Version..."

# ── Detect architecture ──────────────────────────────────────────────────────

# $env:PROCESSOR_ARCHITECTURE lies under x64 emulation on ARM64 Windows.
# Use .NET RuntimeInformation when available, fall back to PROCESSOR_ARCHITEW6432.
$Arch = "amd64"
try {
    $osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    if ($osArch -eq [System.Runtime.InteropServices.Architecture]::Arm64) { $Arch = "arm64" }
} catch {
    if ($env:PROCESSOR_ARCHITEW6432 -eq "ARM64" -or $env:PROCESSOR_ARCHITECTURE -eq "ARM64") {
        $Arch = "arm64"
    }
}

if (-not [Environment]::Is64BitOperatingSystem) {
    Write-Error "32-bit Windows is not supported."
    exit 1
}

# ── Download and extract ─────────────────────────────────────────────────────

$Filename = "${Binary}_${Version}_windows_${Arch}.zip"
$Url = "$CdnBase/v$Version/$Filename"
$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("browseros-cli-install-" + [System.IO.Path]::GetRandomFileName())

try {
    New-Item -ItemType Directory -Path $TmpDir | Out-Null

    $ZipPath = Join-Path $TmpDir $Filename

    Write-Host "Downloading $Url..."
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing

    Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

    $Exe = Get-ChildItem -Path $TmpDir -Filter "$Binary.exe" -File -Recurse | Select-Object -First 1
    if (-not $Exe) {
        Write-Error "Binary not found in archive."
        exit 1
    }

    # ── Install ──────────────────────────────────────────────────────────────

    if (-not (Test-Path $Dir)) {
        New-Item -ItemType Directory -Path $Dir -Force | Out-Null
    }

    Move-Item -Force $Exe.FullName (Join-Path $Dir "$Binary.exe")

    Write-Host "Installed $Binary.exe to $Dir"
} finally {
    if (Test-Path $TmpDir) { Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue }
}

# ── PATH ─────────────────────────────────────────────────────────────────────

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$PathEntries = $UserPath -split ";" | Where-Object { $_ -ne "" }
if ($Dir -notin $PathEntries) {
    Write-Host ""
    Write-Host "Adding $Dir to your user PATH..."
    [Environment]::SetEnvironmentVariable("Path", "$Dir;$UserPath", "User")
    $env:Path = "$Dir;$env:Path"
    Write-Host "Done. Restart your terminal for PATH changes to take effect."
}

Write-Host ""
Write-Host "Run 'browseros-cli --help' to get started."
