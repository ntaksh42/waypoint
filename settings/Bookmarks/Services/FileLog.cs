using System.IO;
using Waypoint.Bookmarks.Core.Diagnostics;

namespace Waypoint.Settings.Bookmarks.Services;

/// <summary>ログファイルに追記する。書き込みに失敗しても、アプリの動作は止めない。</summary>
internal sealed class FileLog : ILog
{
    private readonly object _gate = new();
    private readonly string _path;

    public FileLog(string path) => _path = path;

    public void Info(string message) => Write("INFO ", message);
    public void Error(string message) => Write("ERROR", message);

    private void Write(string level, string message)
    {
        try
        {
            lock (_gate)
            {
                Directory.CreateDirectory(Path.GetDirectoryName(_path)!);
                File.AppendAllText(_path, $"{DateTime.Now:s} {level} {message}{Environment.NewLine}");
            }
        }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
    }
}
