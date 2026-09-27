<#
.SYNOPSIS
    下载 LGPL 版 FFmpeg / FFprobe 并放置为 Tauri sidecar。

.DESCRIPTION
    设计方案 §11：Tuneforge 以子进程方式调用 FFmpeg，并把可执行文件随包分发。
    本脚本从 BtbN 的 Windows 构建中取出 ffmpeg.exe / ffprobe.exe，重命名为
    Tauri `externalBin` 约定的 `ffmpeg-<target-triple>.exe` 形式。

    随包分发请保持默认 Flavor=lgpl。义务与源码获取方式见 docs/third-party.md。

.PARAMETER Triple
    目标 triple，默认 x86_64-pc-windows-msvc。

.PARAMETER Flavor
    lgpl（默认，推荐）或 gpl。

.PARAMETER Version
    FFmpeg 主版本号，默认 8.1（资产名形如 ffmpeg-n8.1-latest-win64-lgpl-8.1.zip）。

.EXAMPLE
    pwsh -File scripts/fetch-ffmpeg.ps1
    pwsh -File scripts/fetch-ffmpeg.ps1 -Version 7.1 -Flavor lgpl
#>
[CmdletBinding()]
param(
    [string]$Triple = "x86_64-pc-windows-msvc",
    [ValidateSet("lgpl", "gpl")]
    [string]$Flavor = "lgpl",
    [string]$Version = "8.1"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$targetDir = Join-Path $repoRoot "app/src-tauri/binaries"
New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

# 资产名形如 ffmpeg-n8.1-latest-win64-lgpl-8.1.zip；优先通过 GitHub API 解析，
# 避免版本号变化导致 404。
$prefix = "ffmpeg-n$Version-latest-win64-$Flavor"
$url = $null
try {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/latest" `
        -Headers @{ "User-Agent" = "tuneforge-fetch" } -UseBasicParsing
    $match = $release.assets |
        Where-Object { $_.name -like "$prefix*.zip" -and $_.name -notlike "*shared*" } |
        Select-Object -First 1
    if ($match) { $url = $match.browser_download_url }
} catch {
    Write-Warning "查询 GitHub API 失败，回退到固定资产名：$_"
}
if (-not $url) {
    $url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/$prefix-$Version.zip"
}

$asset = Split-Path -Leaf $url
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) $asset

Write-Host "下载：$url"
Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing

$extractRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("tuneforge-ffmpeg-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
Expand-Archive -Path $tmp -DestinationPath $extractRoot -Force

$binDir = Get-ChildItem -Path $extractRoot -Recurse -Directory -Filter bin | Select-Object -First 1
if (-not $binDir) { throw "压缩包里找不到 bin 目录：$asset" }

foreach ($name in @("ffmpeg", "ffprobe")) {
    $source = Join-Path $binDir.FullName "$name.exe"
    if (-not (Test-Path $source)) { throw "缺少 $name.exe" }
    $dest = Join-Path $targetDir "$name-$Triple.exe"
    Copy-Item -Path $source -Destination $dest -Force
    Write-Host ("已写入 {0}（{1:N1} MB）" -f $dest, ((Get-Item $dest).Length / 1MB))
}

Remove-Item -Recurse -Force $extractRoot
Remove-Item -Force $tmp

Write-Host ""
Write-Host "完成。请确认 licenses/FFMPEG-LICENSE.txt 存在，并复核许可条款（LGPL 构建）。"
