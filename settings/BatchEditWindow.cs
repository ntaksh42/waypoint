using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings;

public sealed class BatchEditWindow : Window
{
    private readonly ComboBox _open = new();
    private readonly ComboBox _branch = new();
    public string? OpenMode => _open.SelectedIndex switch { 1 => "newWindow", 2 => "reuse", _ => null };
    public bool? ShowBranch => _branch.SelectedIndex switch { 1 => true, 2 => false, _ => null };

    public BatchEditWindow(int count)
    {
        Title = "Edit selected items";
        Width = 420;
        SizeToContent = SizeToContent.Height;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        var panel = new StackPanel { Margin = new Thickness(18) };
        Content = panel;
        panel.Children.Add(new TextBlock { Text = $"Apply shared properties to {count} selected item(s).", TextWrapping = TextWrapping.Wrap });
        panel.Children.Add(new TextBlock { Text = "Open mode", Margin = new Thickness(0, 14, 0, 4) });
        _open.ItemsSource = new[] { "Don't change", "New window", "Reuse Explorer window" };
        _open.SelectedIndex = 0;
        panel.Children.Add(_open);
        panel.Children.Add(new TextBlock { Text = "Show Git branch name", Margin = new Thickness(0, 14, 0, 4) });
        _branch.ItemsSource = new[] { "Don't change", "On", "Off" };
        _branch.SelectedIndex = 0;
        panel.Children.Add(_branch);
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 18, 0, 0) };
        var apply = new Button { Content = "Apply", IsDefault = true, Width = 88, Margin = new Thickness(0, 0, 8, 0) };
        apply.Click += (_, _) => DialogResult = true;
        buttons.Children.Add(apply);
        buttons.Children.Add(new Button { Content = "Cancel", IsCancel = true, Width = 88 });
        panel.Children.Add(buttons);
    }
}
