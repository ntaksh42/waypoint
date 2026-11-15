namespace Waypoint.Settings.Bookmarks.Services;

internal enum EdgeLaunchResult
{
    Launched,

    /// <summary>Edge が起動中で、起動オプション (拡張機能の読み込み) が効かないため起動しなかった。</summary>
    EdgeAlreadyRunning,

    ExtensionNotFound,
}

internal interface IEdgeLauncher
{
    /// <summary>拡張機能を読み込んだ状態で Edge を起動する。</summary>
    /// <exception cref="FileNotFoundException">Edge (msedge.exe) が見つからないとき。</exception>
    public EdgeLaunchResult Launch();
}
