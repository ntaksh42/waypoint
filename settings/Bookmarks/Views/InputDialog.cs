using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings.Bookmarks.Views;

/// <summary>1 行の文字列を入力するダイアログ。</summary>
internal sealed class InputDialog : Window
{
    private readonly TextBox _input;

    private InputDialog(Window owner, string title, string label, string initial)
    {
        DialogLayout.Configure(this, owner, title, 420, 170);
        _input = new TextBox { Text = initial, Margin = new Thickness(0, 4, 0, 12) };
        var ok = new Button { Content = "OK" };
        Content = new StackPanel
        {
            Margin = new Thickness(12),
            Children = { new TextBlock { Text = label }, _input, DialogLayout.Buttons(this, ok) },
        };
        Loaded += (_, _) => { _input.Focus(); _input.SelectAll(); };
    }

    /// <returns>入力された文字列。キャンセルまたは空なら null。</returns>
    public static string? Show(Window owner, string title, string label, string initial)
    {
        var dialog = new InputDialog(owner, title, label, initial);
        var text = dialog.ShowDialog() == true ? dialog._input.Text.Trim() : "";
        return text.Length > 0 ? text : null;
    }
}
