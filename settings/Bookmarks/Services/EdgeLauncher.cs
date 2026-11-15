using System.IO;
using System.Diagnostics;
using Waypoint.Bookmarks.Core.Diagnostics;
using Microsoft.Win32;

namespace Waypoint.Settings.Bookmarks.Services;

internal sealed class EdgeLauncher : IEdgeLauncher
{
    private readonly ILog _log;

    public EdgeLauncher(ILog log) => _log = log;

    public EdgeLaunchResult Launch()
    {
        if (FindExtensionDirectory() is not { } extension) return EdgeLaunchResult.ExtensionNotFound;
        if (Process.GetProcessesByName("msedge").Length > 0) return EdgeLaunchResult.EdgeAlreadyRunning;

        Process.Start(new ProcessStartInfo(FindEdge(), $"--load-extension=\"{extension}\"") { UseShellExecute = false });
        _log.Info("launched Edge with extension: " + extension);
        return EdgeLaunchResult.Launched;
    }

    /// <summary>
    /// 同梱の拡張 (Waypoint Tab Bridge) のフォルダ。インストール先では exe の隣の
    /// browser-tabs\extension、開発中はリポジトリの browser_tabs\extension を、実行ファイルから上へたどって探す。
    /// </summary>
    private static string? FindExtensionDirectory()
    {
        for (var dir = new DirectoryInfo(AppContext.BaseDirectory); dir is not null; dir = dir.Parent)
        {
            foreach (var folder in new[] { "browser-tabs", "browser_tabs" })
            {
                var candidate = Path.Combine(dir.FullName, folder, "extension");
                if (File.Exists(Path.Combine(candidate, "manifest.json"))) return candidate;
            }
        }
        return null;
    }

    private static string FindEdge()
    {
        const string key = @"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe";
        foreach (var hive in new[] { Registry.LocalMachine, Registry.CurrentUser })
            if (hive.OpenSubKey(key)?.GetValue(null) is string path && File.Exists(path)) return path;

        var fallback = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86),
            @"Microsoft\Edge\Application\msedge.exe");
        return File.Exists(fallback) ? fallback : throw new FileNotFoundException("Microsoft Edge (msedge.exe) was not found.");
    }
}
