using System.IO;

namespace Waypoint.Settings.Bookmarks.Services;

internal static class AppPaths
{
    private static readonly string _dataDirectory = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "waypoint");

    /// <summary>お気に入り編集のログ (%LOCALAPPDATA%\waypoint\bookmark-editor.log)。</summary>
    public static string LogFile { get; } = Path.Combine(_dataDirectory, "bookmark-editor.log");

    /// <summary>お気に入りのバックアップの置き場 (FR-6.20)。</summary>
    public static string BackupDirectory { get; } = Path.Combine(_dataDirectory, "bookmark-backups");
}
