namespace Waypoint.Bookmarks.Edge.Protocol;

public static class ProtocolConstants
{
    /// <summary>
    /// 編集画面 (サーバー) と Native Messaging host (`waypoint-bookmark-host.exe`、クライアント) を結ぶ
    /// 名前付きパイプの名前。Rust 側の `bookmark_editor::PIPE_PATH` と揃える。
    /// </summary>
    public const string PipeName = "waypoint-bookmarks";

    /// <summary>Edge に登録する Native Messaging host の名前。拡張の bookmarks.js と Rust 側の定数と揃える。</summary>
    public const string NativeHostName = "com.ntaksh42.waypoint.bookmarks";
}
