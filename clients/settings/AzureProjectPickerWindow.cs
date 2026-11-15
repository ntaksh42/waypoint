using System.Collections.ObjectModel;
using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings;

public sealed class AzureProjectPickerWindow : Window
{
    private readonly ObservableCollection<AzureAvailableProject> _projects;
    public IReadOnlyList<string> Selected => _projects.Where(project => project.Selected).Select(project => project.Name).ToList();

    public AzureProjectPickerWindow(IEnumerable<string> projects)
    {
        _projects = new ObservableCollection<AzureAvailableProject>(projects.Select(name => new AzureAvailableProject(name)));
        Title = "Select Azure DevOps projects";
        Width = 560; Height = 620; WindowStartupLocation = WindowStartupLocation.CenterOwner;
        var root = new DockPanel { Margin = new Thickness(18) };
        Content = root;
        var footer = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 14, 0, 0) };
        DockPanel.SetDock(footer, Dock.Bottom);
        var add = new Button { Content = "Add selected", IsDefault = true, Width = 110, Margin = new Thickness(0, 0, 8, 0) };
        add.Click += (_, _) => DialogResult = true;
        footer.Children.Add(add); footer.Children.Add(new Button { Content = "Cancel", IsCancel = true, Width = 88 }); root.Children.Add(footer);
        var grid = new DataGrid { AutoGenerateColumns = false, CanUserAddRows = false, ItemsSource = _projects };
        grid.Columns.Add(new DataGridCheckBoxColumn { Header = "", Binding = new System.Windows.Data.Binding(nameof(AzureAvailableProject.Selected)), Width = 42 });
        grid.Columns.Add(new DataGridTextColumn { Header = "Project", Binding = new System.Windows.Data.Binding(nameof(AzureAvailableProject.Name)), Width = new DataGridLength(1, DataGridLengthUnitType.Star) });
        root.Children.Add(grid);
    }
}

public sealed class AzureAvailableProject(string name)
{
    public string Name { get; } = name;
    public bool Selected { get; set; }
}
