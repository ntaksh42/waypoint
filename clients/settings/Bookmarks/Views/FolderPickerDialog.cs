using System.Windows;
using System.Windows.Controls;
using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Settings.Bookmarks.Views;

/// <summary>フォルダを一覧から選ぶダイアログ。語で絞り込める。</summary>
internal sealed class FolderPickerDialog : Window
{
    private readonly List<(BookmarkNode Folder, string Path)> _folders;
    private readonly ListBox _list = new();
    private readonly TextBox _filter = new() { Margin = new Thickness(0, 0, 0, 8) };

    private FolderPickerDialog(Window owner, string title, BookmarkTree tree)
    {
        DialogLayout.Configure(this, owner, title, 520, 480);
        _folders = tree.Folders
            .Select(f => (f, string.IsNullOrEmpty(f.FolderPath) ? f.Title : f.FolderPath + " / " + f.Title))
            .ToList();
        _list.ItemsSource = _folders.Select(f => f.Path).ToList();

        var ok = new Button { Content = "Move" };
        var buttons = DialogLayout.Buttons(this, ok);
        buttons.Margin = new Thickness(0, 8, 0, 0);
        DockPanel.SetDock(buttons, Dock.Bottom);
        DockPanel.SetDock(_filter, Dock.Top);
        Content = new DockPanel { Margin = new Thickness(12), Children = { buttons, _filter, _list } };

        _filter.TextChanged += (_, _) => _list.ItemsSource = _folders
            .Where(f => f.Path.Contains(_filter.Text.Trim(), StringComparison.CurrentCultureIgnoreCase))
            .Select(f => f.Path)
            .ToList();
        _list.MouseDoubleClick += (_, _) => { if (_list.SelectedItem is not null) DialogResult = true; };
        Loaded += (_, _) => _filter.Focus();
    }

    /// <returns>選ばれたフォルダ。キャンセルなら null。</returns>
    public static BookmarkNode? Show(Window owner, string title, BookmarkTree tree)
    {
        var dialog = new FolderPickerDialog(owner, title, tree);
        if (dialog.ShowDialog() != true || dialog._list.SelectedItem is not string selected) return null;
        return dialog._folders.First(f => f.Path == selected).Folder;
    }
}
