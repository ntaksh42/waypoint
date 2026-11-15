using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings;

/// <summary>フィルタ付きチェックボックス一覧。Azure の選択値を DSL にせず編集する。</summary>
public sealed class MultiSelectWindow : Window
{
    private readonly List<Choice> _choices;
    private readonly TextBox _filter = new() { Margin = new Thickness(0, 8, 0, 10) };
    private readonly StackPanel _list = new();

    public IReadOnlyList<string> Selected => _choices.Where(choice => choice.Selected).Select(choice => choice.Value).ToList();

    public MultiSelectWindow(string title, string description, IEnumerable<string> choices, IEnumerable<string> selected)
    {
        Title = title;
        Width = 560;
        Height = 620;
        MinWidth = 400;
        MinHeight = 360;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        var selectedValues = new HashSet<string>(selected, StringComparer.OrdinalIgnoreCase);
        _choices = choices.Distinct(StringComparer.OrdinalIgnoreCase)
            .Select(value => new Choice(value, selectedValues.Contains(value))).ToList();
        foreach (var value in selectedValues.Where(value => _choices.All(choice => !choice.Value.Equals(value, StringComparison.OrdinalIgnoreCase))))
            _choices.Add(new Choice(value, true));

        var root = new DockPanel { Margin = new Thickness(18) };
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 12, 0, 0) };
        var apply = new Button { Content = "Apply", Width = 88, IsDefault = true };
        apply.Click += (_, _) => DialogResult = true;
        buttons.Children.Add(apply);
        buttons.Children.Add(new Button { Content = "Cancel", Width = 88, Margin = new Thickness(8, 0, 0, 0), IsCancel = true });
        DockPanel.SetDock(buttons, Dock.Bottom);
        root.Children.Add(buttons);

        var top = new StackPanel();
        top.Children.Add(new TextBlock { Text = description, TextWrapping = TextWrapping.Wrap });
        _filter.ToolTip = "Filter";
        _filter.TextChanged += (_, _) => RebuildList();
        top.Children.Add(_filter);
        DockPanel.SetDock(top, Dock.Top);
        root.Children.Add(top);

        var scroll = new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = _list };
        root.Children.Add(scroll);
        Content = root;
        RebuildList();
    }

    private void RebuildList()
    {
        _list.Children.Clear();
        var filter = _filter.Text.Trim();
        foreach (var choice in _choices.Where(choice => filter.Length == 0 || choice.Value.Contains(filter, StringComparison.OrdinalIgnoreCase)))
        {
            var box = new CheckBox { Content = choice.Value, IsChecked = choice.Selected, Margin = new Thickness(Depth(choice.Value) * 16, 2, 0, 2) };
            box.Checked += (_, _) => choice.Selected = true;
            box.Unchecked += (_, _) => choice.Selected = false;
            _list.Children.Add(box);
        }
        if (_list.Children.Count == 0) _list.Children.Add(new TextBlock { Text = "No matches.", Foreground = System.Windows.Media.Brushes.DimGray });
    }

    private static int Depth(string value) => Math.Max(0, value.Count(character => character == '\\') - 1);

    private sealed class Choice(string value, bool selected)
    {
        public string Value { get; } = value;
        public bool Selected { get; set; } = selected;
    }
}
