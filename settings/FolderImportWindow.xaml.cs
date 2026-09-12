using System.ComponentModel;
using System.Text.Json.Nodes;
using System.Windows;

namespace Waypoint.Settings;

public partial class FolderImportWindow : Window
{
    private FolderImportNode? _root;
    public JsonObject? ResultItem { get; private set; }
    public List<JsonObject>? ResultChildren { get; private set; }

    public FolderImportWindow() => InitializeComponent();

    public FolderImportWindow(string rootPath) : this()
    {
        Title = $"Import subfolders — {System.IO.Path.GetFileName(rootPath.TrimEnd(System.IO.Path.DirectorySeparatorChar))}";
        RootPath.Text = rootPath;
        RootPath.IsReadOnly = true;
        BrowseButton.IsEnabled = false;
        Loaded += (_, _) => Preview_Click(this, new RoutedEventArgs());
    }

    private void Browse_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new System.Windows.Forms.FolderBrowserDialog();
        if (dialog.ShowDialog() == System.Windows.Forms.DialogResult.OK) RootPath.Text = dialog.SelectedPath;
    }

    private void Preview_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            if (!int.TryParse((Depth.SelectedItem as System.Windows.Controls.ComboBoxItem)?.Content?.ToString(), out var depth)) depth = 2;
            _root = FolderImportNode.Scan(RootPath.Text.Trim(), depth);
            PreviewTree.ItemsSource = new[] { _root };
        }
        catch (Exception error) { MessageBox.Show(this, error.Message, "Import", MessageBoxButton.OK, MessageBoxImage.Warning); }
    }

    private void Import_Click(object sender, RoutedEventArgs e)
    {
        if (_root is null || !int.TryParse((Depth.SelectedItem as System.Windows.Controls.ComboBoxItem)?.Content?.ToString(), out _)) { MessageBox.Show(this, "Preview a folder before importing.", "Import", MessageBoxButton.OK, MessageBoxImage.Warning); return; }
        ResultItem = _root.ToItem();
        ResultChildren = _root.Children.Select(child => child.ToItem()).Where(child => child is not null).Cast<JsonObject>().ToList();
        if (ResultItem is null) { MessageBox.Show(this, "Select at least one folder to import.", "Import", MessageBoxButton.OK, MessageBoxImage.Warning); return; }
        DialogResult = true;
    }
}

public sealed class FolderImportNode(string name, string path) : INotifyPropertyChanged
{
    private string _name = name;
    private bool _included = true;
    public string Name { get => _name; set { _name = value; PropertyChanged?.Invoke(this, new(nameof(Name))); } }
    public string Path { get; } = path;
    public bool Included { get => _included; set { _included = value; PropertyChanged?.Invoke(this, new(nameof(Included))); } }
    public List<FolderImportNode> Children { get; } = [];
    public event PropertyChangedEventHandler? PropertyChanged;

    public static FolderImportNode Scan(string root, int depth)
    {
        if (string.IsNullOrWhiteSpace(root) || !Directory.Exists(root)) throw new InvalidDataException("The selected path is not a folder.");
        return ScanNode(root, depth);
    }

    private static FolderImportNode ScanNode(string path, int remaining)
    {
        var node = new FolderImportNode(System.IO.Path.GetFileName(path.TrimEnd(System.IO.Path.DirectorySeparatorChar)) is { Length: > 0 } name ? name : path, path);
        if (remaining == 0) return node;
        foreach (var child in Directory.EnumerateDirectories(path).OrderBy(value => value, StringComparer.OrdinalIgnoreCase))
        {
            if ((File.GetAttributes(child) & FileAttributes.ReparsePoint) != 0) continue;
            try { node.Children.Add(ScanNode(child, remaining - 1)); } catch (UnauthorizedAccessException) { }
        }
        return node;
    }

    public JsonObject? ToItem()
    {
        if (!Included) return null;
        var children = Children.Select(child => child.ToItem()).Where(child => child is not null).Cast<JsonObject>().ToList();
        if (children.Count == 0) return new JsonObject { ["type"] = "folder", ["name"] = Name, ["path"] = Path };
        var items = new JsonArray { new JsonObject { ["type"] = "folder", ["name"] = "Open this folder", ["path"] = Path } };
        foreach (var child in children) items.Add(child);
        return new JsonObject { ["type"] = "submenu", ["name"] = Name, ["items"] = items };
    }
}
