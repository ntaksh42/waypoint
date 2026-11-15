using System.IO;
using System.Diagnostics;
using System.Windows;
using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Settings.Bookmarks.Services;

namespace Waypoint.Settings.Bookmarks.Views;

internal sealed class WpfDialogService : IDialogService
{
    private readonly Func<Window> _ownerAccessor;

    /// <param name="ownerAccessor">ダイアログの親にするウィンドウ。画面より先にこのサービスを作るため、取得を遅らせる。</param>
    public WpfDialogService(Func<Window> ownerAccessor) => _ownerAccessor = ownerAccessor;

    private Window Owner => _ownerAccessor();

    public bool Confirm(string title, string message) =>
        MessageBox.Show(Owner, message, title, MessageBoxButton.YesNo, MessageBoxImage.Warning) == MessageBoxResult.Yes;

    public void ShowInfo(string title, string message) =>
        MessageBox.Show(Owner, message, title, MessageBoxButton.OK, MessageBoxImage.Information);

    public void ShowWarning(string title, string message) =>
        MessageBox.Show(Owner, message, title, MessageBoxButton.OK, MessageBoxImage.Warning);

    public string? Ask(string title, string label, string initial) => InputDialog.Show(Owner, title, label, initial);

    public BookmarkNode? PickFolder(string title, BookmarkTree tree) => FolderPickerDialog.Show(Owner, title, tree);

    public ReplaceSpec? AskReplace(IReadOnlyList<BookmarkNode> targets, string scopeLabel) =>
        ReplaceDialog.Show(Owner, targets, scopeLabel);

    public void OpenUrl(string url) => Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });

    public void OpenFolder(string path)
    {
        Directory.CreateDirectory(path);
        Process.Start("explorer.exe", path);
    }
}
