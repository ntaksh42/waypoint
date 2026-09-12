using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace Waypoint.Settings;

public partial class MainWindow : Window
{
    private readonly ConfigStore _store = new();
    private readonly ObservableCollection<ItemRow> _rows = [];
    private readonly ObservableCollection<VariableRow> _variables = [];
    private readonly ObservableCollection<AzureProjectRow> _azureProjects = [];
    private MenuNode? _selectedMenu;
    private AzureProjectRow? _selectedAzureProject;
    private bool _dirty;
    private bool _loading;

    public MainWindow()
    {
        InitializeComponent();
        ItemsGrid.ItemsSource = _rows;
        VariablesGrid.ItemsSource = _variables;
        try
        {
            _store.Load();
            LoadControls();
            RebuildTree();
            if (Environment.GetCommandLineArgs().Contains("--azure-suggest")) RootTabs.SelectedIndex = 3;
            TrackPersistentControls();
        }
        catch (Exception error)
        {
            MessageBox.Show(this, $"Could not read the configuration. It was not changed.\n\n{error.Message}", "Waypoint", MessageBoxButton.OK, MessageBoxImage.Error);
            Close();
        }
    }

    private static JsonObject Object(JsonObject parent, string name)
        => parent[name] as JsonObject ?? (JsonObject)(parent[name] = new JsonObject());
    private static JsonArray Array(JsonObject parent, string name)
        => parent[name] as JsonArray ?? (JsonArray)(parent[name] = new JsonArray());
    private static bool Bool(JsonObject source, string name, bool fallback = false)
        => source[name]?.GetValue<bool?>() ?? fallback;
    private static string Text(JsonObject source, string name, string fallback = "")
        => source[name]?.GetValue<string>() ?? fallback;

    private void LoadControls()
    {
        _loading = true;
        var settings = _store.Settings;
        var quick = Object(settings, "quickLaunch");
        QuickLaunchHotkey.Text = Text(quick, "hotkey", "Alt+Space");
        RecentFolders.IsChecked = Bool(quick, "includeRecentFolders", true);
        FrequentFolders.IsChecked = Bool(quick, "includeFrequentFolders", true);
        OpenWindows.IsChecked = Bool(quick, "includeOpenWindows", true);
        Bookmarks.IsChecked = Bool(quick, "includeBookmarks", true);
        BrowserHistory.IsChecked = Bool(quick, "includeBrowserHistory", true);
        Apps.IsChecked = Bool(quick, "includeApps", true);
        Everything.IsChecked = Bool(quick, "includeEverything", true);
        SearchPaths.IsChecked = Bool(quick, "searchPaths");
        VisibleResults.Text = (quick["visibleResults"]?.GetValue<int?>() ?? 12).ToString();
        AzureEnabled.IsChecked = Bool(AzureSettings(), "enabled");
        LoadAzureProjects();
        _variables.Clear();
        foreach (var pair in _store.Variables) _variables.Add(new VariableRow(pair.Key, pair.Value?.GetValue<string>() ?? ""));
        _loading = false;
    }

    private void TrackPersistentControls()
    {
        foreach (var box in new[] { QuickLaunchHotkey, VisibleResults, AzureOrganization, AzureProject, AzureAliases, AzurePriority, AzureRepositories, AzureAreas, AzureIterations })
            box.TextChanged += (_, _) => Changed();
        foreach (var box in new[] { RecentFolders, FrequentFolders, OpenWindows, Bookmarks, BrowserHistory, Apps, Everything, SearchPaths, AzureEnabled, AzurePullRequests, AzurePipelines, AzureWorkItems })
        {
            box.Checked += (_, _) => Changed();
            box.Unchecked += (_, _) => Changed();
        }
        VariablesGrid.CellEditEnding += (_, _) => Changed();
    }

    private void RebuildTree()
    {
        var root = new MenuNode("Main", [], _store.Items);
        AddSubmenus(root);
        MenuTree.Items.Clear();
        var item = CreateTreeItem(root, MenuFilter.Text.Trim());
        if (item is not null) MenuTree.Items.Add(item);
        if (_selectedMenu is null) SelectTreeItem(item);
    }

    private static void AddSubmenus(MenuNode parent)
    {
        for (var index = 0; index < parent.Items.Count; index++)
        {
            if (parent.Items[index] is not JsonObject { } item || Text(item, "type") != "submenu") continue;
            var path = new List<int>(parent.Path) { index };
            var child = new MenuNode(Text(item, "name", "Submenu"), path, Array(item, "items"));
            parent.Children.Add(child);
            AddSubmenus(child);
        }
    }

    private static TreeViewItem? CreateTreeItem(MenuNode node, string filter)
    {
        var children = node.Children.Select(child => CreateTreeItem(child, filter)).Where(child => child is not null).Cast<TreeViewItem>().ToList();
        if (!string.IsNullOrWhiteSpace(filter) && !node.Title.Contains(filter, StringComparison.OrdinalIgnoreCase) && children.Count == 0) return null;
        var item = new TreeViewItem { Header = node.Title, Tag = node, IsExpanded = true };
        foreach (var child in children) item.Items.Add(child);
        return item;
    }

    private static void SelectTreeItem(TreeViewItem? item)
    {
        if (item is null) return;
        item.IsSelected = true;
        item.Focus();
    }

    private void RefreshItems()
    {
        _rows.Clear();
        var query = ItemSearch.Text.Trim();
        if (query.Length > 0)
        {
            foreach (var hit in FindItems(_store.Items, [], "Main", query)) _rows.Add(hit);
            return;
        }
        if (_selectedMenu is null) return;
        for (var index = 0; index < _selectedMenu.Items.Count; index++)
        {
            if (_selectedMenu.Items[index] is not JsonObject item) continue;
            var row = new ItemRow(item, index);
            _rows.Add(row);
        }
    }

    private void MenuTree_SelectedItemChanged(object sender, RoutedPropertyChangedEventArgs<object> e)
    {
        _selectedMenu = (e.NewValue as TreeViewItem)?.Tag as MenuNode;
        RefreshItems();
    }

    private void MenuFilter_Changed(object sender, TextChangedEventArgs e) => RebuildTree();
    private void ItemSearch_Changed(object sender, TextChangedEventArgs e) => RefreshItems();
    private void ItemsGrid_DoubleClick(object sender, MouseButtonEventArgs e) => EditSelected();

    private void Add_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        var kind = ((AddKind.SelectedItem as ComboBoxItem)?.Content as string) ?? "Folder";
        var editor = new ItemEditorWindow(kind, null) { Owner = this };
        if (editor.ShowDialog() != true || editor.Result is null) return;
        _selectedMenu.Items.Add(editor.Result);
        Changed();
        RefreshItems();
    }

    private void Edit_Click(object sender, RoutedEventArgs e) => EditSelected();
    private void EditSelected()
    {
        if (ItemsGrid.SelectedItems.Count > 1) { EditBatch(); return; }
        if (ItemsGrid.SelectedItems.Count != 1 || ItemsGrid.SelectedItem is not ItemRow row) return;
        if (row.MenuPath is not null)
        {
            SelectMenu(row.MenuPath);
            ItemSearch.Clear();
            RefreshItems();
            ItemsGrid.SelectedItem = _rows.FirstOrDefault(candidate => candidate.Index == row.Index);
            return;
        }
        var editor = new ItemEditorWindow(row.Type, row.Item) { Owner = this };
        if (editor.ShowDialog() != true || editor.Result is null || _selectedMenu is null) return;
        _selectedMenu.Items[row.Index] = editor.Result;
        Changed();
        RefreshItems();
    }

    private void EditBatch()
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        var rows = SelectedRows();
        var editor = new BatchEditWindow(rows.Count) { Owner = this };
        if (editor.ShowDialog() != true) return;
        foreach (var row in rows)
        {
            var item = row.Item;
            if (editor.OpenMode is not null && (Text(item, "type") == "folder" || Text(item, "type") == "specialFolder")) item["open"] = editor.OpenMode;
            if (editor.ShowBranch is not null && (Text(item, "type") == "folder" || Text(item, "type") == "submenu")) item["showBranch"] = editor.ShowBranch.Value;
        }
        Changed();
        RefreshItems();
    }

    private List<ItemRow> SelectedRows() => ItemsGrid.SelectedItems.Cast<ItemRow>().OrderBy(row => row.Index).ToList();
    private void Remove_Click(object sender, RoutedEventArgs e)
    {
        if (!string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        var rows = SelectedRows();
        if (rows.Count == 0 || _selectedMenu is null || MessageBox.Show(this, $"Remove {rows.Count} selected item(s)?", "Waypoint", MessageBoxButton.YesNo, MessageBoxImage.Warning) != MessageBoxResult.Yes) return;
        foreach (var row in rows.OrderByDescending(row => row.Index)) _selectedMenu.Items.RemoveAt(row.Index);
        Changed();
        RefreshItems();
    }

    private void Duplicate_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        var rows = SelectedRows();
        foreach (var row in rows.OrderByDescending(row => row.Index)) _selectedMenu.Items.Insert(row.Index + 1, row.Item.DeepClone());
        if (rows.Count > 0) { Changed(); RefreshItems(); }
    }

    private void Up_Click(object sender, RoutedEventArgs e) => MoveSelected(-1);
    private void Down_Click(object sender, RoutedEventArgs e) => MoveSelected(1);
    private void MoveSelected(int delta)
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        var rows = SelectedRows();
        if (rows.Count == 0 || (delta < 0 && rows[0].Index == 0) || (delta > 0 && rows[^1].Index == _selectedMenu.Items.Count - 1)) return;
        foreach (var row in delta < 0 ? rows : rows.AsEnumerable().Reverse())
        {
            var other = row.Index + delta;
            (_selectedMenu.Items[row.Index], _selectedMenu.Items[other]) = (_selectedMenu.Items[other], _selectedMenu.Items[row.Index]);
        }
        Changed();
        RefreshItems();
    }

    private void AddVariable_Click(object sender, RoutedEventArgs e) { _variables.Add(new VariableRow("", "")); Changed(); }
    private void RemoveVariable_Click(object sender, RoutedEventArgs e)
    {
        if (VariablesGrid.SelectedItem is VariableRow row) { _variables.Remove(row); Changed(); }
    }

    private JsonObject AzureSettings()
    {
        var quick = Object(_store.Settings, "quickLaunch");
        return Object(quick, "azureDevops");
    }

    private void LoadAzureProjects()
    {
        _azureProjects.Clear();
        foreach (var project in Array(AzureSettings(), "projects").OfType<JsonObject>())
            _azureProjects.Add(new AzureProjectRow(project));
        ClearAzureDetailsPanel();
        RebuildAzureTree();
    }

    private static string Lines(JsonNode? value) => string.Join(Environment.NewLine,
        (value as JsonArray ?? []).Select(entry => entry?.GetValue<string>() ?? ""));

    private static JsonArray LinesToArray(string value) => new(value
        .Split(['\r', '\n', ','], StringSplitOptions.RemoveEmptyEntries)
        .Select(line => (JsonNode?)line.Trim()).ToArray());

    private void CommitAzureProject()
    {
        if (_loading || _selectedAzureProject is null) return;
        var project = _selectedAzureProject.ProjectNode;
        project["organization"] = AzureOrganization.Text.Trim();
        project["project"] = AzureProject.Text.Trim();
        project["aliases"] = LinesToArray(AzureAliases.Text);
        if (!int.TryParse(AzurePriority.Text, out var priority) || priority < 0) priority = 0;
        project["priority"] = priority;
        project["includePullRequests"] = AzurePullRequests.IsChecked == true;
        project["includePipelines"] = AzurePipelines.IsChecked == true;
        project["includeWorkItems"] = AzureWorkItems.IsChecked == true;
        project["interestRepositories"] = LinesToArray(AzureRepositories.Text);
        project["interestAreas"] = LinesToArray(AzureAreas.Text);
        project["interestIterations"] = LinesToArray(AzureIterations.Text);
    }

    private void Save_Click(object sender, RoutedEventArgs e) => Save();
    private void SaveClose_Click(object sender, RoutedEventArgs e) { if (Save()) Close(); }
    private bool Save()
    {
        try
        {
            ApplyControls();
            _store.Save();
            _dirty = false;
            StatusText.Text = "Saved and reloaded.";
            return true;
        }
        catch (Exception error)
        {
            MessageBox.Show(this, error.Message, "Save failed", MessageBoxButton.OK, MessageBoxImage.Error);
            return false;
        }
    }

    private void ApplyControls()
    {
        var settings = _store.Settings;
        var quick = Object(settings, "quickLaunch");
        CommitAzureProject();
        if (string.IsNullOrWhiteSpace(QuickLaunchHotkey.Text)) throw new InvalidDataException("Quick Launch hotkey is required.");
        if (!int.TryParse(VisibleResults.Text, out var visible) || visible is < 12 or > 24) throw new InvalidDataException("Visible results must be between 12 and 24.");
        settings.Remove("trigger");
        settings.Remove("menu");
        quick["hotkey"] = QuickLaunchHotkey.Text.Trim();
        quick["includeRecentFolders"] = RecentFolders.IsChecked == true;
        quick["includeFrequentFolders"] = FrequentFolders.IsChecked == true;
        quick["includeOpenWindows"] = OpenWindows.IsChecked == true;
        quick["includeBookmarks"] = Bookmarks.IsChecked == true;
        quick["includeBrowserHistory"] = BrowserHistory.IsChecked == true;
        quick["includeApps"] = Apps.IsChecked == true;
        quick["includeEverything"] = Everything.IsChecked == true;
        quick["searchPaths"] = SearchPaths.IsChecked == true;
        quick["visibleResults"] = visible;
        AzureSettings()["enabled"] = AzureEnabled.IsChecked == true;
        var variables = _store.Variables;
        variables.Clear();
        foreach (var row in _variables)
        {
            if (string.IsNullOrWhiteSpace(row.Name)) throw new InvalidDataException("Variable name is required.");
            if (variables.ContainsKey(row.Name)) throw new InvalidDataException($"Variable name is duplicated: {row.Name}");
            variables[row.Name] = row.Value;
        }
    }

    private void Window_Drop(object sender, System.Windows.DragEventArgs e)
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text) || !e.Data.GetDataPresent(DataFormats.FileDrop)) return;
        foreach (var path in (string[])e.Data.GetData(DataFormats.FileDrop))
        {
            var info = new FileInfo(path);
            var isFolder = Directory.Exists(path);
            if (!isFolder && !info.Exists) continue;
            _selectedMenu.Items.Add(new JsonObject { ["type"] = isFolder ? "folder" : "file", ["name"] = Path.GetFileName(path), ["path"] = path });
        }
        Changed();
        RefreshItems();
    }

    private void Window_Closing(object? sender, CancelEventArgs e)
    {
        if (_dirty)
        {
            var result = MessageBox.Show(this, "Save changes before closing?", "Waypoint", MessageBoxButton.YesNoCancel, MessageBoxImage.Question);
            if (result == MessageBoxResult.Cancel) { e.Cancel = true; return; }
            if (result == MessageBoxResult.Yes && !Save()) { e.Cancel = true; return; }
        }
        _recorder?.Dispose();
    }

    private void Changed()
    {
        if (_loading) return;
        _dirty = true;
        StatusText.Text = "Unsaved changes";
    }
}
