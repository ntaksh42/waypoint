<#
.SYNOPSIS
  GitHub Releases から waypoint の最新 MSI を取得してインストールする。

.DESCRIPTION
  ntaksh42/waypoint の最新リリースから *-x64.msi アセットを一時フォルダへ
  ダウンロードし、msiexec でインストールする。MSI を手動で GitHub から
  落とす手間を省くためのもの。

.PARAMETER Silent
  msiexec を /qn (無人・UI 非表示) で実行する。

.EXAMPLE
  irm https://raw.githubusercontent.com/ntaksh42/waypoint/main/installer/install.ps1 | iex
  .\installer\install.ps1
  .\installer\install.ps1 -Silent
#>
[CmdletBinding()]
param(
    [switch]$Silent
)

$ErrorActionPreference = 'Stop'
$repo = 'ntaksh42/waypoint'

Write-Host "最新リリースを確認しています ($repo) ..."
$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'waypoint-install-script' }

$asset = $release.assets | Where-Object { $_.name -like '*-x64.msi' } | Select-Object -First 1
if (-not $asset) { throw "リリース $($release.tag_name) に MSI アセットが見つかりませんでした" }

Write-Host "$($release.tag_name) : $($asset.name) をダウンロードします"

$tempDir = Join-Path $env:TEMP 'waypoint-install'
New-Item -ItemType Directory -Force $tempDir | Out-Null
$msiPath = Join-Path $tempDir $asset.name

Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $msiPath

# 常駐中だと上書きに失敗するので落とす
Get-Process waypoint, waypoint-settings -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "  実行中の $($_.ProcessName) (PID $($_.Id)) を終了します"
    Stop-Process -Id $_.Id -Force
}

$msiArgs = @('/i', "`"$msiPath`"")
if ($Silent) { $msiArgs += '/qn' }

Write-Host "msiexec $($msiArgs -join ' ')"
$proc = Start-Process msiexec.exe -ArgumentList $msiArgs -Wait -PassThru
if ($proc.ExitCode -ne 0) { throw "msiexec がエラーで終了しました (exit code $($proc.ExitCode))" }

Write-Host ""
Write-Host "インストール完了: $($release.tag_name)" -ForegroundColor Green
