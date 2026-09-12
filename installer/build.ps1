<#
.SYNOPSIS
  waypoint の MSI をビルドする。

.DESCRIPTION
  Cargo.toml の version を読み、x64 リリースビルド → WiX で MSI を作る。
  バージョンは Cargo.toml が唯一の出どころ。wxs 側には -d Version で渡すので
  二重管理にならない。

.EXAMPLE
  .\installer\build.ps1
  .\installer\build.ps1 -SkipBuild     # ビルド済みバイナリを使う
#>
[CmdletBinding()]
param(
    # cargo build を省略して既存のバイナリを使う
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$target = 'x86_64-pc-windows-msvc'
$relDir = Join-Path $root "target\$target\release"
$distDir = Join-Path $root 'dist'

# --- バージョンは Cargo.toml から取る ---
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"(.+?)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw 'Cargo.toml から version を読めませんでした' }
Write-Host "version = $version"

# --- 常駐中だとリンクに失敗するので落とす ---
Get-Process waypoint, waypoint-settings -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "  実行中の $($_.ProcessName) (PID $($_.Id)) を終了します"
    Stop-Process -Id $_.Id -Force
}

if (-not $SkipBuild) {
    # 設定画面は build.rs も同じ内容で publish するが、あちらは失敗しても
    # 警告どまり (設定画面が無くても常駐部は動くため)。配布物では欠落を
    # 許さないので、ここは明示的に publish して失敗を throw する。
    # 同じ publish を 2 度走らせないよう cargo 側は環境変数で抑止する。
    Write-Host "cargo build --release --target $target"
    $env:WAYPOINT_SKIP_SETTINGS_BUILD = '1'
    try {
        cargo build --release --target $target
    } finally {
        Remove-Item Env:\WAYPOINT_SKIP_SETTINGS_BUILD -ErrorAction SilentlyContinue
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build に失敗しました" }

    Write-Host "dotnet publish settings (self-contained x64)"
    dotnet publish .\settings\Waypoint.Settings.csproj -c Release -r win-x64 --self-contained true `
        -p:PublishSingleFile=true -p:DebugType=None -o $relDir
    if ($LASTEXITCODE -ne 0) { throw "Settings の .NET publish に失敗しました" }
}

foreach ($exe in @('waypoint.exe', 'waypoint-settings.exe', 'waypoint-tab-host.exe')) {
    $p = Join-Path $relDir $exe
    if (-not (Test-Path $p)) { throw "$exe が見つかりません: $p" }
}

# --- WiX の Util 拡張 (util:CloseApplication に必要) ---
if (-not (wix extension list --global 2>&1 | Select-String 'WixToolset.Util.wixext')) {
    Write-Host 'WiX Util 拡張を追加します'
    wix extension add --global WixToolset.Util.wixext/5.0.2
}

New-Item -ItemType Directory -Force $distDir | Out-Null
$msi = Join-Path $distDir "waypoint-$version-x64.msi"

Write-Host "wix build -> $msi"
wix build (Join-Path $PSScriptRoot 'waypoint.wxs') `
    -ext WixToolset.Util.wixext `
    -arch x64 `
    -bindpath $relDir `
    -d Version=$version `
    -o $msi
if ($LASTEXITCODE -ne 0) { throw 'wix build に失敗しました' }

# wixpdb は配布物ではないので消す
Remove-Item (Join-Path $distDir "waypoint-$version-x64.wixpdb") -ErrorAction SilentlyContinue

$size = [math]::Round((Get-Item $msi).Length / 1KB)
Write-Host ""
Write-Host "完成: $msi  ($size KB)" -ForegroundColor Green
Write-Host ""
Write-Host "インストール : msiexec /i `"$msi`""
Write-Host "サイレント   : msiexec /i `"$msi`" /qn"
Write-Host "アンインストール: 設定 > アプリ、または msiexec /x `"$msi`""
