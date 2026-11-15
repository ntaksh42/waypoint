namespace Waypoint.Settings.Bookmarks.Services;

internal interface IBackupService
{
    /// <summary>バックアップの置き場。</summary>
    public string Directory { get; }

    /// <summary>お気に入りのスナップショット (JSON) を保存する。</summary>
    public void Save(string json);
}
