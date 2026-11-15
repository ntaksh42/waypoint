# お気に入り編集 (FR-6.16, FR-6.17) を実際の Edge で確かめる結合試験。
# 使い捨てのプロファイルで Edge を起動し、Waypoint.Bookmarks.SmokeTest を流す。
#
# 事前に cargo build で target\debug\waypoint-bookmark-host.exe を作っておく。
# Native Messaging host の登録は試験の間だけこのビルドへ向け、終わったら元に戻す。
# 起動中の Edge があると、そちらの拡張につながって普段のお気に入りを書き換えてしまうため、
# Edge が 1 つでも起動していたら実行しない。
param([string]$Configuration = 'debug')

$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (Get-Process msedge -ErrorAction SilentlyContinue) {
    Write-Error 'Edge is running. Close every Edge window so the test cannot touch your real bookmarks.'
    exit 2
}

$hostExe = Join-Path $root "target\$Configuration\waypoint-bookmark-host.exe"
if (-not (Test-Path $hostExe)) {
    Write-Error "$hostExe was not found. Run cargo build first."
    exit 2
}

$hostName = 'com.ntaksh42.waypoint.bookmarks'
$key = "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\$hostName"
$previous = (Get-Item $key -ErrorAction SilentlyContinue)?.GetValue('')
$work = Join-Path $env:TEMP "waypoint-bookmarks-smoke-$([guid]::NewGuid().ToString('N'))"
$profileDir = Join-Path $work 'profile'
$manifest = Join-Path $work "$hostName.json"
New-Item -ItemType Directory -Force $profileDir | Out-Null
@{
    name            = $hostName
    description     = 'Waypoint bookmark editor bridge (smoke test)'
    path            = (Resolve-Path $hostExe).Path
    type            = 'stdio'
    allowed_origins = @('chrome-extension://fllmalpfkkdholloicheiplekihkkjeo/')
} | ConvertTo-Json | Set-Content $manifest -Encoding utf8
New-Item $key -Force | Out-Null
Set-ItemProperty $key '(default)' $manifest

$edge = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe' -ErrorAction SilentlyContinue).'(default)'
if (-not $edge) { $edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" }

try {
    $test = Start-Process dotnet -ArgumentList "run --project `"$PSScriptRoot\Waypoint.Bookmarks.SmokeTest`" -c Release" -PassThru -NoNewWindow
    Start-Sleep 5
    Start-Process $edge -ArgumentList "--user-data-dir=`"$profileDir`"", "--load-extension=`"$root\clients\browser_tabs\extension`"", '--no-first-run', 'about:blank'
    $test.WaitForExit()
    $code = $test.ExitCode
} finally {
    Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" |
        Where-Object { $_.CommandLine -like "*$profileDir*" } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    if ($previous) { Set-ItemProperty $key '(default)' $previous } else { Remove-Item $key -ErrorAction SilentlyContinue }
    Start-Sleep 2
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
exit $code
