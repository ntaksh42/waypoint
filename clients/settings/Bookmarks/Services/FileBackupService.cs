using System.IO;
using Waypoint.Bookmarks.Core.Diagnostics;

namespace Waypoint.Settings.Bookmarks.Services;

/// <summary>スナップショットを時刻付きの JSON ファイルとして保存し、新しい <see cref="KeepCount"/> 件だけ残す。</summary>
internal sealed class FileBackupService : IBackupService
{
    private const int KeepCount = 30;

    private readonly ILog _log;

    public FileBackupService(string directory, ILog log)
    {
        Directory = directory;
        _log = log;
    }

    public string Directory { get; }

    public void Save(string json)
    {
        try
        {
            System.IO.Directory.CreateDirectory(Directory);
            File.WriteAllText(Path.Combine(Directory, $"{DateTime.Now:yyyyMMdd-HHmmss-fff}.json"), json);
            foreach (var old in new DirectoryInfo(Directory).GetFiles("*.json").OrderByDescending(f => f.Name).Skip(KeepCount))
                old.Delete();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            _log.Error("backup failed: " + e.Message);
        }
    }
}
