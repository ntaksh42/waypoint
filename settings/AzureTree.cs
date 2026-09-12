using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;

namespace Waypoint.Settings;

/// <summary>Azure DevOps タブの Watched projects を Organization → Project のツリーで表示する (FR-6)。</summary>
public partial class MainWindow
{
    private bool _rebuildingAzureTree;
    private Point? _azureDragStart;

    private void RebuildAzureTree(object? preferSelect = null)
    {
        var filter = AzureSearch.Text.Trim();
        AzureProjectsTree.Items.Clear();
        TreeViewItem? toSelect = null;
        var groups = _azureProjects
            .GroupBy(row => row.Organization, StringComparer.OrdinalIgnoreCase)
            .Where(group => filter.Length == 0
                || group.Key.Contains(filter, StringComparison.OrdinalIgnoreCase)
                || group.Any(row => row.Project.Contains(filter, StringComparison.OrdinalIgnoreCase)))
            .OrderBy(group => group.Min(row => row.Priority));
        foreach (var group in groups)
        {
            var count = group.Count();
            var orgItem = new TreeViewItem { Header = BuildOrgHeader(group.Key, count, filter), Tag = group.Key, IsExpanded = true };
            AutomationProperties.SetName(orgItem, $"{group.Key}, {count} project{(count == 1 ? "" : "s")}, {(HasPat(group.Key) ? "PAT connected" : "no PAT")}");
            if (preferSelect is string organization && organization.Equals(group.Key, StringComparison.OrdinalIgnoreCase)) toSelect = orgItem;
            foreach (var row in group.OrderBy(row => row.Priority))
            {
                var projectItem = new TreeViewItem { Header = BuildProjectHeader(row, filter), Tag = row };
                AutomationProperties.SetName(projectItem, $"{row.Project}, priority {row.Priority}, " +
                    $"pull requests {(row.IncludePullRequests ? "on" : "off")}, pipelines {(row.IncludePipelines ? "on" : "off")}, work items {(row.IncludeWorkItems ? "on" : "off")}");
                if (preferSelect is AzureProjectRow selected && ReferenceEquals(selected, row)) toSelect = projectItem;
                orgItem.Items.Add(projectItem);
            }
            AzureProjectsTree.Items.Add(orgItem);
        }
        if (toSelect is not null)
        {
            _rebuildingAzureTree = true;
            toSelect.IsSelected = true;
            _rebuildingAzureTree = false;
        }
        else ClearAzureDetailsPanel();
    }

    private static object BuildOrgHeader(string organization, int projectCount, string filter)
    {
        var panel = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
        panel.Children.Add(new Ellipse
        {
            Width = 7, Height = 7, Margin = new Thickness(0, 0, 6, 0), VerticalAlignment = VerticalAlignment.Center,
            Fill = new SolidColorBrush(HasPat(organization) ? Color.FromRgb(0x2F, 0x85, 0x58) : Color.FromRgb(0xC7, 0xCC, 0xD3)),
            ToolTip = HasPat(organization) ? "PAT saved for this organization" : "No PAT saved for this organization",
        });
        panel.Children.Add(HighlightedText(string.IsNullOrWhiteSpace(organization) ? "(no organization)" : organization, filter, FontWeights.SemiBold));
        panel.Children.Add(new TextBlock
        {
            Text = $"  {projectCount} project{(projectCount == 1 ? "" : "s")}", FontSize = 11, Foreground = Brushes.Gray, Margin = new Thickness(6, 0, 0, 0),
        });
        return panel;
    }

    private static object BuildProjectHeader(AzureProjectRow row, string filter)
    {
        var panel = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
        panel.Children.Add(HighlightedText(string.IsNullOrWhiteSpace(row.Project) ? "(unnamed project)" : row.Project, filter, FontWeights.Normal));
        panel.Children.Add(new TextBlock
        {
            Text = $" #{row.Priority}", FontSize = 9.5, FontFamily = new FontFamily("Consolas"), Foreground = Brushes.Gray, Margin = new Thickness(5, 0, 0, 0),
        });
        var prLabel = row.IncludePullRequests && row.RepositoryCount > 0 ? $"PR·{row.RepositoryCount}" : "PR";
        panel.Children.Add(ScopeBadge(prLabel, row.IncludePullRequests));
        panel.Children.Add(ScopeBadge("CI", row.IncludePipelines));
        panel.Children.Add(ScopeBadge("WI", row.IncludeWorkItems));
        return panel;
    }

    private static Border ScopeBadge(string text, bool on) => new()
    {
        Margin = new Thickness(5, 0, 0, 0),
        Padding = new Thickness(4, 0, 4, 1),
        CornerRadius = new CornerRadius(3),
        Background = on ? new SolidColorBrush(Color.FromRgb(0xF7, 0xE7, 0xCB)) : Brushes.Transparent,
        BorderBrush = new SolidColorBrush(on ? Color.FromRgb(0xE4, 0xC3, 0x87) : Color.FromRgb(0xD8, 0xDC, 0xE3)),
        BorderThickness = new Thickness(1),
        Child = new TextBlock
        {
            Text = text, FontSize = 9, FontWeight = FontWeights.Bold,
            Foreground = new SolidColorBrush(on ? Color.FromRgb(0x9A, 0x63, 0x16) : Color.FromRgb(0x89, 0x91, 0xA0)),
        },
    };

    private static TextBlock HighlightedText(string text, string filter, FontWeight weight)
    {
        var block = new TextBlock { FontWeight = weight };
        var index = filter.Length == 0 ? -1 : text.IndexOf(filter, StringComparison.OrdinalIgnoreCase);
        if (index < 0) { block.Text = text; return block; }
        block.Inlines.Add(new Run(text[..index]));
        block.Inlines.Add(new Run(text.Substring(index, filter.Length)) { Background = Brushes.Moccasin });
        block.Inlines.Add(new Run(text[(index + filter.Length)..]));
        return block;
    }

    private static bool HasPat(string organization)
    {
        if (string.IsNullOrWhiteSpace(organization)) return false;
        try { CredentialStore.Load(organization); return true; }
        catch { return false; }
    }

    private void SelectAzureProject(AzureProjectRow row)
    {
        _selectedAzureProject = row;
        _loading = true;
        AzureDetailsPanel.IsEnabled = true;
        var project = row.ProjectNode;
        AzureOrganization.Text = Text(project, "organization");
        if (string.IsNullOrWhiteSpace(AzureConnectionOrganization.Text)) AzureConnectionOrganization.Text = AzureOrganization.Text;
        AzureProject.Text = Text(project, "project");
        AzureAliases.Text = string.Join(", ", (project["aliases"] as JsonArray ?? []).Select(entry => entry?.GetValue<string>() ?? ""));
        AzurePriority.Text = row.Priority.ToString();
        AzurePullRequests.IsChecked = row.IncludePullRequests;
        AzurePipelines.IsChecked = row.IncludePipelines;
        AzureWorkItems.IsChecked = row.IncludeWorkItems;
        AzureRepositories.Text = Lines(project["interestRepositories"]);
        AzureAreas.Text = Lines(project["interestAreas"]);
        AzureIterations.Text = Lines(project["interestIterations"]);
        _loading = false;
    }

    private void ClearAzureDetailsPanel()
    {
        _selectedAzureProject = null;
        AzureDetailsPanel.IsEnabled = false;
        AzureOrganization.Clear(); AzureProject.Clear(); AzureAliases.Clear(); AzurePriority.Clear();
        AzureRepositories.Clear(); AzureAreas.Clear(); AzureIterations.Clear();
    }

    private void AzureTree_SelectedItemChanged(object sender, RoutedPropertyChangedEventArgs<object> e)
    {
        if (!_rebuildingAzureTree) CommitAzureProject();
        var tag = (e.NewValue as TreeViewItem)?.Tag;
        if (tag is AzureProjectRow row) SelectAzureProject(row);
        else ClearAzureDetailsPanel();
        if (_rebuildingAzureTree) return;
        if (tag is AzureProjectRow selected) RebuildAzureTree(selected);
        else if (tag is string organization) RebuildAzureTree(organization);
    }

    private void AzureSearch_Changed(object sender, TextChangedEventArgs e)
    {
        CommitAzureProject();
        RebuildAzureTree(_selectedAzureProject);
    }

    private void AddAzureProject_Click(object sender, RoutedEventArgs e)
    {
        CommitAzureProject();
        var project = NewAzureProject("", "");
        Array(AzureSettings(), "projects").Add(project);
        var row = new AzureProjectRow(project);
        _azureProjects.Add(row);
        Changed();
        RebuildAzureTree(row);
    }

    private void RemoveAzureProject_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedAzureProject is null) return;
        if (MessageBox.Show(this, $"Remove {_selectedAzureProject.Project}?", "Waypoint", MessageBoxButton.YesNo, MessageBoxImage.Warning) != MessageBoxResult.Yes) return;
        Array(AzureSettings(), "projects").Remove(_selectedAzureProject.ProjectNode);
        _azureProjects.Remove(_selectedAzureProject);
        Changed();
        RebuildAzureTree();
    }

    private void AzureCtxAddProject_Click(object sender, RoutedEventArgs e)
    {
        if (AzureProjectsTree.SelectedItem is not TreeViewItem { Tag: string organization }) return;
        CommitAzureProject();
        var project = NewAzureProject(organization, "");
        Array(AzureSettings(), "projects").Add(project);
        var row = new AzureProjectRow(project);
        _azureProjects.Add(row);
        Changed();
        RebuildAzureTree(row);
    }

    private void AzureTree_ContextMenuOpening(object sender, ContextMenuEventArgs e)
    {
        var tag = (AzureProjectsTree.SelectedItem as TreeViewItem)?.Tag;
        AzureCtxAddProject.Visibility = tag is string ? Visibility.Visible : Visibility.Collapsed;
        AzureCtxRemove.Visibility = tag is AzureProjectRow ? Visibility.Visible : Visibility.Collapsed;
        if (tag is null) e.Handled = true;
    }

    private void AzureTree_PreviewMouseRightButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<TreeViewItem>(e.OriginalSource as DependencyObject) is { } item) item.IsSelected = true;
    }

    private void AzureTree_PreviewMouseMove(object sender, MouseEventArgs e)
    {
        if (e.LeftButton != MouseButtonState.Pressed) { _azureDragStart = null; return; }
        if (FindAncestor<TreeViewItem>(Mouse.DirectlyOver as DependencyObject) is not { Tag: AzureProjectRow row } sourceItem) { _azureDragStart = null; return; }
        var now = e.GetPosition(AzureProjectsTree);
        if (_azureDragStart is null) { _azureDragStart = now; return; }
        if (Math.Abs(now.X - _azureDragStart.Value.X) < SystemParameters.MinimumHorizontalDragDistance
            && Math.Abs(now.Y - _azureDragStart.Value.Y) < SystemParameters.MinimumVerticalDragDistance) return;
        DragDrop.DoDragDrop(sourceItem, new DataObject(typeof(AzureProjectRow), row), DragDropEffects.Move);
        _azureDragStart = null;
    }

    private void AzureTree_Drop(object sender, DragEventArgs e)
    {
        if (!e.Data.GetDataPresent(typeof(AzureProjectRow)) || e.Data.GetData(typeof(AzureProjectRow)) is not AzureProjectRow source) return;
        if (FindAncestor<TreeViewItem>(e.OriginalSource as DependencyObject) is not { Tag: AzureProjectRow target } || ReferenceEquals(target, source)) return;
        if (!string.Equals(target.Organization, source.Organization, StringComparison.OrdinalIgnoreCase)) return;
        CommitAzureProject();
        var ordered = _azureProjects.OrderBy(row => row.Priority).ToList();
        ordered.Remove(source);
        ordered.Insert(ordered.IndexOf(target), source);
        for (var index = 0; index < ordered.Count; index++) ordered[index].ProjectNode["priority"] = index;
        Changed();
        RebuildAzureTree(source);
    }
}
