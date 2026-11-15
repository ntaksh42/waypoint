using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings;

public sealed class MoveItemsWindow : Window
{
    private readonly List<MenuChoice> _all;
    private readonly ListBox _list = new();
    private readonly TextBox _filter = new();
    public MenuChoice? Selected { get; private set; }

    public MoveItemsWindow(IEnumerable<MenuChoice> choices)
    {
        _all = choices.ToList();
        Title = "Move to menu";
        Width = 460;
        Height = 520;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        var panel = new DockPanel { Margin = new Thickness(18) };
        Content = panel;
        var footer = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 14, 0, 0) };
        DockPanel.SetDock(footer, Dock.Bottom);
        var move = new Button { Content = "Move", IsDefault = true, Width = 88, Margin = new Thickness(0, 0, 8, 0) };
        move.Click += (_, _) => { Selected = _list.SelectedItem as MenuChoice; if (Selected is not null) DialogResult = true; };
        footer.Children.Add(move);
        footer.Children.Add(new Button { Content = "Cancel", IsCancel = true, Width = 88 });
        panel.Children.Add(footer);
        DockPanel.SetDock(_filter, Dock.Top);
        _filter.Margin = new Thickness(0, 0, 0, 10);
        _filter.TextChanged += (_, _) => Refresh();
        panel.Children.Add(_filter);
        panel.Children.Add(_list);
        Refresh();
    }

    private void Refresh()
    {
        var filter = _filter.Text.Trim();
        _list.ItemsSource = _all.Where(choice => filter.Length == 0 || choice.Name.Contains(filter, StringComparison.OrdinalIgnoreCase)).ToList();
    }
}
