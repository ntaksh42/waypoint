using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings;

/// <summary>型ごとに必要な項目だけを出す、設定項目の小さな編集ダイアログ。</summary>
public sealed class ItemEditorWindow : Window
{
    private readonly string _kind;
    private readonly JsonObject? _source;
    private readonly TextBox _name = new();
    private readonly TextBox _path = new();
    private readonly TextBox _target = new();
    private readonly ComboBox _knownFolder = new();
    private readonly ComboBox _open = new();
    private readonly CheckBox _showBranch = new() { Content = "Show Git branch name" };
    public JsonObject? Result { get; private set; }

    public ItemEditorWindow(string kind, JsonObject? source)
    {
        _kind = kind switch { "specialFolder" => "Special folder", "shell" => "Shell location", _ => char.ToUpperInvariant(kind[0]) + kind[1..] };
        _source = source;
        Title = source is null ? $"Add {kind}" : "Edit item";
        Width = 520;
        SizeToContent = SizeToContent.Height;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ResizeMode = ResizeMode.NoResize;
        var panel = new StackPanel { Margin = new Thickness(18) };
        Content = panel;
        _open.Items.Add("New window");
        _open.Items.Add("Reuse Explorer window");
        _open.SelectedIndex = 0;
        _knownFolder.ItemsSource = new[] { "Desktop", "Documents", "Pictures", "Downloads", "Music", "Videos", "Profile", "Public", "Recent", "StartMenu" };
        _knownFolder.SelectedIndex = 0;
        AddText(panel, "Name", _name);
        switch (_kind)
        {
            case "Folder":
                AddPath(panel, "Path", _path, true);
                AddOpen(panel);
                panel.Children.Add(_showBranch);
                break;
            case "File":
                AddPath(panel, "Path", _path, false);
                break;
            case "Special folder":
                panel.Children.Add(new TextBlock { Text = "Known folder", Margin = new Thickness(0, 10, 0, 3) });
                panel.Children.Add(_knownFolder);
                AddOpen(panel);
                break;
            case "Shell location": AddText(panel, "Target (for example shell:MyComputerFolder)", _target); break;
            case "Submenu": panel.Children.Add(_showBranch); break;
            case "Separator": break;
        }
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 18, 0, 0) };
        var ok = new Button { Content = "OK", IsDefault = true, Width = 86, Margin = new Thickness(0, 0, 8, 0) };
        ok.Click += (_, _) => Accept();
        var cancel = new Button { Content = "Cancel", IsCancel = true, Width = 86 };
        buttons.Children.Add(ok);
        buttons.Children.Add(cancel);
        panel.Children.Add(buttons);
        Load(source);
    }

    private void Load(JsonObject? source)
    {
        if (source is null) return;
        _name.Text = source["name"]?.GetValue<string>() ?? "";
        _path.Text = source["path"]?.GetValue<string>() ?? "";
        _target.Text = source["target"]?.GetValue<string>() ?? "";
        _knownFolder.SelectedItem = source["knownFolder"]?.GetValue<string>() ?? "Desktop";
        _open.SelectedIndex = source["open"]?.GetValue<string>() == "reuse" ? 1 : 0;
        _showBranch.IsChecked = source["showBranch"]?.GetValue<bool?>() ?? false;
    }

    private static void AddText(Panel panel, string label, TextBox box)
    {
        panel.Children.Add(new TextBlock { Text = label, Margin = new Thickness(0, 10, 0, 3) });
        panel.Children.Add(box);
    }

    private void AddPath(Panel panel, string label, TextBox box, bool folder)
    {
        panel.Children.Add(new TextBlock { Text = label, Margin = new Thickness(0, 10, 0, 3) });
        var row = new Grid();
        row.ColumnDefinitions.Add(new ColumnDefinition());
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(box);
        var browse = new Button { Content = "Browse...", Margin = new Thickness(8, 0, 0, 0) };
        browse.Click += (_, _) => Browse(box, folder);
        Grid.SetColumn(browse, 1);
        row.Children.Add(browse);
        panel.Children.Add(row);
    }

    private void AddOpen(Panel panel)
    {
        panel.Children.Add(new TextBlock { Text = "Open mode", Margin = new Thickness(0, 10, 0, 3) });
        panel.Children.Add(_open);
    }

    private void Browse(TextBox box, bool folder)
    {
        if (folder)
        {
            var dialog = new System.Windows.Forms.FolderBrowserDialog();
            if (dialog.ShowDialog() == System.Windows.Forms.DialogResult.OK) box.Text = dialog.SelectedPath;
        }
        else
        {
            var dialog = new Microsoft.Win32.OpenFileDialog();
            if (dialog.ShowDialog(this) == true) box.Text = dialog.FileName;
        }
    }

    private void Accept()
    {
        var nameRequired = _kind != "Separator";
        if (nameRequired && string.IsNullOrWhiteSpace(_name.Text)) { Error("Name is required."); return; }
        if ((_kind is "Folder" or "File") && string.IsNullOrWhiteSpace(_path.Text)) { Error("Path is required."); return; }
        if (_kind == "Shell location" && string.IsNullOrWhiteSpace(_target.Text)) { Error("Target is required."); return; }
        var type = _kind switch { "Special folder" => "specialFolder", "Shell location" => "shell", _ => _kind.ToLowerInvariant() };
        var item = _source?.DeepClone().AsObject() ?? new JsonObject();
        item["type"] = type;
        if (_kind == "Separator")
        {
            if (!string.IsNullOrWhiteSpace(_name.Text)) item["name"] = _name.Text.Trim();
        }
        else item["name"] = _name.Text.Trim();
        switch (_kind)
        {
            case "Folder":
                item["path"] = _path.Text.Trim();
                if (_open.SelectedIndex == 1) item["open"] = "reuse"; else item.Remove("open");
                if (_showBranch.IsChecked == true) item["showBranch"] = true; else item.Remove("showBranch");
                break;
            case "File": item["path"] = _path.Text.Trim(); break;
            case "Special folder":
                item["knownFolder"] = _knownFolder.SelectedItem?.ToString() ?? "Desktop";
                if (_open.SelectedIndex == 1) item["open"] = "reuse"; else item.Remove("open");
                break;
            case "Shell location": item["target"] = _target.Text.Trim(); break;
            case "Submenu":
                if (item["items"] is not JsonArray) item["items"] = new JsonArray();
                if (_showBranch.IsChecked == true) item["showBranch"] = true; else item.Remove("showBranch");
                break;
        }
        Result = item;
        DialogResult = true;
    }

    private void Error(string message) => MessageBox.Show(this, message, "Waypoint", MessageBoxButton.OK, MessageBoxImage.Warning);
}
