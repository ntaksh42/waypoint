using System.Windows;
using System.Windows.Controls;
using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;

namespace Waypoint.Settings.Bookmarks.Views;

/// <summary>一括置換の条件を決めるダイアログ。変更内容をプレビューしながら入力できる。</summary>
internal sealed class ReplaceDialog : Window
{
    private const int PreviewLimit = 200;

    private readonly IReadOnlyList<BookmarkNode> _targets;
    private readonly string _scopeLabel;
    private readonly TextBox _find = new() { Margin = new Thickness(0, 2, 0, 6) };
    private readonly TextBox _replacement = new() { Margin = new Thickness(0, 2, 0, 6) };
    private readonly CheckBox _inTitle = new() { Content = "Name", IsChecked = true, Margin = new Thickness(0, 0, 12, 0) };
    private readonly CheckBox _inUrl = new() { Content = "URL", IsChecked = true, Margin = new Thickness(0, 0, 12, 0) };
    private readonly CheckBox _useRegex = new() { Content = "Regular expression", Margin = new Thickness(0, 0, 12, 0) };
    private readonly CheckBox _ignoreCase = new() { Content = "Ignore case", IsChecked = true };
    private readonly TextBlock _summary = new() { Margin = new Thickness(0, 8, 0, 4), FontWeight = FontWeights.Bold };
    private readonly ListBox _preview = new();
    private readonly Button _ok = new() { Content = "Replace" };

    private ReplaceDialog(Window owner, IReadOnlyList<BookmarkNode> targets, string scopeLabel)
    {
        _targets = targets;
        _scopeLabel = scopeLabel;
        DialogLayout.Configure(this, owner, "Replace", 640, 560);

        var options = new StackPanel { Orientation = Orientation.Horizontal, Children = { _inTitle, _inUrl, _useRegex, _ignoreCase } };
        var top = new StackPanel
        {
            Children = { new TextBlock { Text = "Find" }, _find, new TextBlock { Text = "Replace with" }, _replacement, options, _summary },
        };
        var buttons = DialogLayout.Buttons(this, _ok);
        buttons.Margin = new Thickness(0, 8, 0, 0);
        DockPanel.SetDock(buttons, Dock.Bottom);
        DockPanel.SetDock(top, Dock.Top);
        Content = new DockPanel { Margin = new Thickness(12), Children = { buttons, top, _preview } };

        _find.TextChanged += (_, _) => Refresh();
        _replacement.TextChanged += (_, _) => Refresh();
        foreach (var box in new[] { _inTitle, _inUrl, _useRegex, _ignoreCase })
        {
            box.Checked += (_, _) => Refresh();
            box.Unchecked += (_, _) => Refresh();
        }
        Loaded += (_, _) => { _find.Focus(); Refresh(); };
    }

    private ReplaceSpec Spec() => new(_find.Text, _replacement.Text, _inTitle.IsChecked == true, _inUrl.IsChecked == true,
        _useRegex.IsChecked == true, _ignoreCase.IsChecked == true);

    private void Refresh()
    {
        _preview.ItemsSource = null;
        _ok.IsEnabled = false;
        if (_find.Text.Length == 0)
        {
            _summary.Text = "Enter the text to find.";
            return;
        }

        IReadOnlyList<ReplaceChange> changes;
        try { changes = Spec().Plan(_targets); }
        catch (ArgumentException e)
        {
            _summary.Text = "Invalid regular expression: " + e.Message;
            return;
        }

        _summary.Text = $"{_scopeLabel}: {changes.Count} of {_targets.Count} bookmark(s) will change";
        _preview.ItemsSource = changes.Take(PreviewLimit).Select(c =>
            $"{c.Node.Title} → {c.NewTitle}" + (c.NewUrl != c.Node.Url ? $"\n    {c.Node.Url} → {c.NewUrl}" : "")).ToList();
        _ok.IsEnabled = changes.Count > 0;
    }

    /// <returns>置換の指定。キャンセルなら null。</returns>
    public static ReplaceSpec? Show(Window owner, IReadOnlyList<BookmarkNode> targets, string scopeLabel)
    {
        var dialog = new ReplaceDialog(owner, targets, scopeLabel);
        return dialog.ShowDialog() == true ? dialog.Spec() : null;
    }
}
