# DSH Docker installer for Windows.
#
# Usage (PowerShell 5.1+):
#   irm https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.ps1 | iex
#
# Or:
#   .\install.ps1 [-Dir C:\path] [-Version v2.0.0] [-Mirror global|cn|off] [-NoDeploy]
#
# Docker Desktop with WSL2 is required for deployment. WSL2 is also recommended for
# the best file permission and symlink behavior.

[CmdletBinding()]
param(
    [string]$Dir = (Join-Path $HOME 'dsh-docker'),
    [string]$Version = 'latest',
    [ValidateSet('global', 'cn', 'off')]
    [string]$Mirror = 'global',
    [switch]$NoDeploy,
    [switch]$Insecure,
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'dsh-docker\bin')
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'reina0721/dsh-docker'
$FallbackVersion = 'v2.0.0'
$MirrorPrefixes = @('https://ghfast.top/', 'https://gh-proxy.com/')

function Write-Info($Message) { Write-Host $Message }
function Write-Warn($Message) { Write-Warning $Message }

function Get-TargetAsset {
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($arch -eq 'AMD64') {
        return 'x86_64-pc-windows-msvc'
    }
    if ($arch -eq 'ARM64') {
        # x64 emulation is available on Windows 11 ARM; use the x64 build.
        return 'x86_64-pc-windows-msvc'
    }
    throw "Unsupported Windows architecture: $arch"
}

function Get-ReleaseTag {
    if ($Version -ne 'latest') { return $Version }
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
            -Headers @{ 'User-Agent' = 'dshctl-installer' } -TimeoutSec 20
        if ($release.tag_name) { return $release.tag_name }
    } catch {
        Write-Warn "Could not resolve the latest release; falling back to $FallbackVersion"
    }
    return $FallbackVersion
}

function Invoke-Download($Url, $Destination) {
    $candidates = @()
    if ($Mirror -eq 'cn') {
        foreach ($prefix in $MirrorPrefixes) { $candidates += "$prefix$Url" }
        $candidates += $Url
    } else {
        $candidates += $Url
        foreach ($prefix in $MirrorPrefixes) { $candidates += "$prefix$Url" }
    }
    foreach ($candidate in $candidates) {
        try {
            Write-Info "Downloading $candidate"
            Invoke-WebRequest -Uri $candidate -OutFile $Destination -UseBasicParsing -TimeoutSec 300
            return
        } catch {
            Write-Warn "Download failed: $candidate"
        }
    }
    throw "Could not download $Url"
}

$target = Get-TargetAsset
$tag = Get-ReleaseTag
$asset = "dshctl-$target.zip"
$base = "https://github.com/$Repo/releases/download/$tag"
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("dshctl-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp | Out-Null

try {
    Write-Info "Installing dshctl $tag for $target"
    Invoke-Download "$base/$asset" (Join-Path $temp $asset)
    Invoke-Download "$base/SHA256SUMS" (Join-Path $temp 'SHA256SUMS')

    if (-not $Insecure) {
        $sums = Get-Content (Join-Path $temp 'SHA256SUMS')
        $line = $sums | Where-Object { ($_ -split '\s+')[1] -eq $asset } | Select-Object -First 1
        if (-not $line) { throw "SHA256SUMS does not contain $asset" }
        $expected = ($line -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash (Join-Path $temp $asset) -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { throw "Checksum mismatch for $asset" }
        Write-Info 'Checksum verified.'
    } else {
        Write-Warn 'Skipping checksum verification (-Insecure).'
    }

    Expand-Archive -Path (Join-Path $temp $asset) -DestinationPath (Join-Path $temp 'unpack') -Force
    $binary = Get-ChildItem (Join-Path $temp 'unpack') -Recurse -Filter 'dshctl.exe' | Select-Object -First 1
    if (-not $binary) { throw 'Archive did not contain dshctl.exe' }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $destination = Join-Path $InstallDir 'dshctl.exe'
    Copy-Item $binary.FullName $destination -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $InstallDir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$InstallDir", 'User')
        Write-Info "Added $InstallDir to the user PATH; open a new terminal to use it."
    }
    $env:Path = "$env:Path;$InstallDir"

    & $destination --dir $Dir init
    if ($LASTEXITCODE -ne 0) { throw 'dshctl init failed' }

    # Upgrade an existing v1 directory in place (auth file/image tag).
    & $destination --dir $Dir migrate | Out-Null
    $global:LASTEXITCODE = 0

    $docker = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $docker) {
        Write-Warn 'Docker Desktop was not found. Install it before deploying.'
        $NoDeploy = $true
    }

    if (-not $NoDeploy) {
        Write-Info ''
        Write-Info 'Starting deployment...'
        if ($Mirror -eq 'global') {
            & $destination --dir $Dir deploy
        } else {
            & $destination --dir $Dir deploy --mirror $Mirror
        }
    } else {
        Write-Info ''
        Write-Info 'Next steps:'
        Write-Info "  dshctl --dir `"$Dir`" user add <name>"
        Write-Info "  dshctl --dir `"$Dir`" deploy"
    }
} finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}
