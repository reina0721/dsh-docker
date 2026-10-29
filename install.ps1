# DSH Docker installer for Windows.
#
# Usage (PowerShell 5.1+):
#   Interactive:
#     irm https://raw.githubusercontent.com/reina0721/dsh-docker/main/install.ps1 | iex
#
#   Options:
#     .\install.ps1 [-Dir C:\path] [-Port 3080] [-Bind 127.0.0.1] [-Variant dev|slim]
#                   [-Mirror global|cn|off] [-AdminUser admin] [-AdminPassword pass]
#                   [-Version v2.0.1] [-NoDeploy] [-Yes]
#
# Docker Desktop with WSL2 is required for deployment.

[CmdletBinding()]
param(
    [string]$Dir = (Join-Path $HOME 'dsh-docker'),
    [int]$Port = 3080,
    [string]$Bind = '127.0.0.1',
    [ValidateSet('dev', 'slim')]
    [string]$Variant = 'dev',
    [ValidateSet('global', 'cn', 'off')]
    [string]$Mirror = 'global',
    [string]$AdminUser = 'admin',
    [string]$AdminPassword = '',
    [string]$Version = 'latest',
    [switch]$NoDeploy,
    [switch]$Yes,
    [switch]$Insecure,
    [string]$InstallDir = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'reina0721/dsh-docker'
$FallbackVersion = 'v2.0.1'
$MirrorPrefixes = @('https://ghfast.top/', 'https://gh-proxy.com/')

function Write-Info($Message) { Write-Host $Message }
function Write-Warn($Message) { Write-Warning $Message }

function Generate-RandomPassword {
    $bytes = New-Object byte[] 12
    (New-Object Security.Cryptography.RNGCryptoServiceProvider).GetBytes($bytes)
    return [Convert]::ToBase64String($bytes).Replace('+', '!').Replace('/', '@').Substring(0, 16)
}

function Prompt-Value([string]$PromptText, [string]$DefaultValue) {
    Write-Host -NoNewline "$PromptText [默认: $DefaultValue]: "
    $val = Read-Host
    if ([string]::IsNullOrWhiteSpace($val)) { return $DefaultValue }
    return $val.Trim()
}

function Prompt-SecretValue([string]$PromptText) {
    Write-Host -NoNewline "$PromptText"
    $pass = Read-Host -AsSecureString
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($pass)
    $plain = [Runtime.InteropServices.Marshal]::PtrToStringAuto($bstr)
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    return $plain
}

# Interactive Wizard
$isInteractive = ($Host.UI.RawUI -ne $null) -and (-not $Yes) -and (-not [Console]::IsInputRedirected)
$passwordGenerated = $false

if ($isInteractive) {
    Write-Host "================================================================" -ForegroundColor Cyan
    Write-Host "            DSH Docker 交互式安装向导 (Windows)                 " -ForegroundColor Cyan
    Write-Host "================================================================" -ForegroundColor Cyan
    Write-Host "所有项目产物将独立集中存放在您指定的项目目录下。`n"

    $Dir = Prompt-Value "`n[1/5] 项目安装目录" $Dir
    if ($Dir.StartsWith('~')) {
        $Dir = Join-Path $HOME $Dir.Substring(1).TrimStart('\', '/')
    }

    $portInput = Prompt-Value "`n[2/5] 外部 Web 访问端口" "$Port"
    if ($portInput -match '^\d+$') {
        $Port = [int]$portInput
    }

    Write-Host "`n[3/5] 监听绑定地址:"
    Write-Host "  1) 127.0.0.1 (推荐: 仅本机访问) [默认]"
    Write-Host "  2) 0.0.0.0   (局域网或公网直接访问)"
    $bindChoice = Prompt-Value "请选择监听地址 [1/2]" "1"
    if ($bindChoice -eq '2' -or $bindChoice -eq '0.0.0.0') {
        $Bind = '0.0.0.0'
    } else {
        $Bind = '127.0.0.1'
    }

    Write-Host "`n[4/5] 网络加速策略:"
    Write-Host "  1) global (官方默认源) [默认]"
    Write-Host "  2) cn     (中国大陆加速镜像)"
    $mirrorChoice = Prompt-Value "请选择加速策略 [1/2]" "1"
    if ($mirrorChoice -eq '2' -or $mirrorChoice -eq 'cn') {
        $Mirror = 'cn'
    } else {
        $Mirror = 'global'
    }

    $AdminUser = Prompt-Value "`n[5/5] 管理员用户名" $AdminUser
    $typedPass = Prompt-SecretValue "请输入管理员密码 (至少12位，留空自动生成安全密码): "
    if ([string]::IsNullOrWhiteSpace($typedPass)) {
        $AdminPassword = Generate-RandomPassword
        $passwordGenerated = $true
    } else {
        $AdminPassword = $typedPass
    }

    Write-Host "`n----------------------------------------------------------------"
    Write-Host "安装配置摘要:"
    Write-Host "  项目目录 : $Dir"
    Write-Host "  访问端口 : $Port"
    Write-Host "  绑定地址 : $Bind"
    Write-Host "  加速策略 : $Mirror"
    Write-Host "  管理账号 : $AdminUser"
    if ($passwordGenerated) {
        Write-Host "  管理密码 : $AdminPassword (自动生成)"
    } else {
        Write-Host "  管理密码 : [已设置]"
    }
    Write-Host "----------------------------------------------------------------"
    $confirm = Prompt-Value "确认并开始安装? [Y/n]" "Y"
    if ($confirm -match '^[nN]') {
        Write-Info "已取消安装。"
        exit 0
    }
}

if ([string]::IsNullOrWhiteSpace($AdminPassword)) {
    $AdminPassword = Generate-RandomPassword
    $passwordGenerated = $true
}

function Get-TargetAsset {
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($arch -eq 'AMD64' -or $arch -eq 'ARM64') {
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
New-Item -ItemType Directory -Path $temp -Force | Out-Null

try {
    Write-Info "Downloading DSH package $tag for $target..."
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
    }

    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    Write-Info "Extracting files to $Dir..."
    Expand-Archive -Path (Join-Path $temp $asset) -DestinationPath $Dir -Force

    $binDir = Join-Path $Dir 'bin'
    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    $dshctlExe = Join-Path $binDir 'dshctl.exe'
    if (-not (Test-Path $dshctlExe) -and (Test-Path (Join-Path $Dir 'dshctl.exe'))) {
        Copy-Item (Join-Path $Dir 'dshctl.exe') $dshctlExe -Force
    }

    # Create root batch launcher in $Dir so user can run .\dshctl from project dir
    $cmdLauncher = Join-Path $Dir 'dshctl.cmd'
    "@echo off`r`n`"%~dp0bin\dshctl.exe`" %*" | Out-File -FilePath $cmdLauncher -Encoding ASCII -Force

    # Configure .env in $Dir
    $envFile = Join-Path $Dir '.env'
    $envExample = Join-Path $Dir '.env.example'
    if (-not (Test-Path $envFile)) {
        if (Test-Path $envExample) {
            Copy-Item $envExample $envFile -Force
        }
    }

    if (Test-Path $envFile) {
        $content = Get-Content $envFile -Raw
        $content = $content -replace '(?m)^DSH_PORT=.*', "DSH_PORT=$Port"
        $content = $content -replace '(?m)^BIND_ADDRESS=.*', "BIND_ADDRESS=$Bind"
        $content = $content -replace '(?m)^DSH_IMAGE_VARIANT=.*', "DSH_IMAGE_VARIANT=$Variant"
        $content = $content -replace '(?m)^DSH_IMAGE_MODE=.*', "DSH_IMAGE_MODE=build"
        $content = $content -replace '(?m)^DSH_IMAGE=.*', "DSH_IMAGE=dsh-docker:local"
        $content = $content -replace '(?m)^DSH_IMAGE_MIRROR=.*', "DSH_IMAGE_MIRROR=$Mirror"
        Set-Content -Path $envFile -Value $content -Encoding UTF8
    }

    # Initialize runtime
    Write-Info "Initializing runtime directories in $Dir..."
    & $dshctlExe --dir $Dir init

    # Create admin user
    Write-Info "Configuring admin account '$AdminUser'..."
    $AdminPassword | & $dshctlExe --dir $Dir user add $AdminUser --password-stdin

    $docker = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $docker) {
        Write-Warn 'Docker Desktop 未运行或未安装。所有文件已安装于项目目录。'
        Write-Warn "请启动 Docker Desktop 后，进入 $Dir 运行 .\dshctl.cmd deploy 完成部署。"
        $NoDeploy = $true
    }

    if (-not $NoDeploy) {
        Write-Info "`n正在本地极速构建 Docker 镜像并启动容器..."
        if ($Mirror -eq 'global') {
            & $dshctlExe --dir $Dir deploy --yes
        } else {
            & $dshctlExe --dir $Dir deploy --mirror $Mirror --yes
        }
    }

    Write-Host @"

================================================================
               🎉 DSH Docker 安装部署成功!
================================================================
服务地址 : http://${Bind}:${Port}
管理账号 : $AdminUser
管理密码 : $AdminPassword

项目目录 : $Dir
所有产物与数据已集中存放在上述项目目录下。

常用管理命令 (在项目目录下执行):
  .\dshctl.cmd status        - 查看运行状态与容器健康
  .\dshctl.cmd logs          - 实时跟踪日志
  .\dshctl.cmd stop / up     - 停止 / 启动服务
  .\dshctl.cmd restart       - 重启服务
  .\dshctl.cmd user add      - 添加新的登录用户
  .\dshctl.cmd doctor        - 诊断部署与环境
================================================================
"@ -ForegroundColor Green

} finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}
