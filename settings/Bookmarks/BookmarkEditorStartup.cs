using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows;
using Waypoint.Bookmarks.Core.Diagnostics;
using Waypoint.Bookmarks.Edge;
using Waypoint.Settings.Bookmarks.Services;
using Waypoint.Settings.Bookmarks.ViewModels;
using Waypoint.Settings.Bookmarks.Views;

namespace Waypoint.Settings.Bookmarks;

/// <summary>
/// `waypoint-settings.exe --bookmarks` で開くお気に入り編集画面 (FR-6.16) の起動処理。
/// 部品の組み立てはここだけで行う。
/// </summary>
internal static class BookmarkEditorStartup
{
    /// <summary>この画面を開く起動引数。Rust 側の `bookmark_editor::SETTINGS_ARGUMENT` と揃える。</summary>
    public const string Argument = "--bookmarks";

    private const string MutexName = "waypoint-bookmark-editor";
    private const string WindowTitle = "Edit Bookmarks - Waypoint";
    private const int SwRestore = 9;

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern nint FindWindow(string? className, string windowName);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ShowWindow(nint hwnd, int command);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetForegroundWindow(nint hwnd);

    public static bool IsRequested(string[] args) => args.Contains(Argument, StringComparer.OrdinalIgnoreCase);

    public static void Run(Application app)
    {
        // パイプのサーバーは 1 つしか立てられないため、画面も 1 つに限る。2 回目は既存の画面を前に出す
        var singleInstance = new Mutex(true, MutexName, out var created);
        if (!created)
        {
            singleInstance.Dispose();
            BringExistingWindowToFront();
            app.Shutdown();
            return;
        }

        var log = new FileLog(AppPaths.LogFile);
        var bridge = new EdgeBridge(log);
        var viewModel = new BookmarkEditorViewModel(
            bridge,
            new WpfDialogService(() => app.MainWindow),
            new FileBackupService(AppPaths.BackupDirectory, log),
            new EdgeLauncher(log),
            new WpfUiDispatcher(app.Dispatcher),
            log);

        var window = new BookmarkEditorWindow(viewModel) { Title = WindowTitle };
        app.MainWindow = window;
        window.Show();

        var shutdown = new CancellationTokenSource();
        _ = RunBridgeAsync(bridge, shutdown.Token, log);
        app.Exit += (_, _) =>
        {
            shutdown.Cancel();
            singleInstance.Dispose();
        };
    }

    private static async Task RunBridgeAsync(EdgeBridge bridge, CancellationToken ct, ILog log)
    {
        try { await bridge.RunAsync(ct); }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            log.Error("bridge stopped: " + e);
        }
    }

    private static void BringExistingWindowToFront()
    {
        var hwnd = FindWindow(null, WindowTitle);
        if (hwnd == 0) return;
        ShowWindow(hwnd, SwRestore);
        SetForegroundWindow(hwnd);
    }
}
