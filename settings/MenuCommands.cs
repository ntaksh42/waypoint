using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace Waypoint.Settings;

public partial class MainWindow
{
    private readonly List<JsonObject> _clipboard = [];
    private Point? _dragStart;

    private static IEnumerable<ItemRow> FindItems(JsonArray items, List<int> path, string menu, string query)
    {
        for (var index = 0; index < items.Count; index++)
        {
            if (items[index] is not JsonObject item) continue;
            var row = new ItemRow(item, index) { MenuPath = new List<int>(path), MenuName = menu };
            if (row.Name.Contains(query, StringComparison.OrdinalIgnoreCase) || row.Target.Contains(query, StringComparison.OrdinalIgnoreCase)) yield return row;
            if (Text(item, "type") != "submenu") continue;
            var childPath = new List<int>(path) { index };
            foreach (var child in FindItems(Array(item, "items"), childPath, row.Name, query)) yield return child;
        }
    }

    private IEnumerable<MenuChoice> MenuChoices()
    {
        return CollectMenus(_store.Items, [], "Main");
    }

    private static IEnumerable<MenuChoice> CollectMenus(JsonArray items, List<int> path, string label)
    {
        yield return new MenuChoice(label, new List<int>(path));
        for (var index = 0; index < items.Count; index++)
        {
            if (items[index] is not JsonObject item || Text(item, "type") != "submenu") continue;
            var childPath = new List<int>(path) { index };
            foreach (var child in CollectMenus(Array(item, "items"), childPath, label == "Main" ? Text(item, "name") : $"{label} > {Text(item, "name")}")) yield return child;
        }
    }

    private void SelectMenu(IReadOnlyList<int> path)
    {
        var node = new MenuNode("Main", [], _store.Items);
        AddSubmenus(node);
        foreach (var index in path)
        {
            node = node.Children.FirstOrDefault(child => child.Path.Count > 0 && child.Path[^1] == index) ?? node;
        }
        _selectedMenu = node;
        RebuildTree();
        var treeItem = FindTreeItem(MenuTree.Items, path);
        if (treeItem is not null) SelectTreeItem(treeItem);
        RefreshItems();
    }

    private static TreeViewItem? FindTreeItem(ItemCollection items, IReadOnlyList<int> path)
    {
        foreach (var item in items.OfType<TreeViewItem>())
        {
            if (item.Tag is MenuNode node && node.Path.SequenceEqual(path)) return item;
            if (FindTreeItem(item.Items, path) is { } child) return child;
        }
        return null;
    }

    private void Copy_Click(object sender, RoutedEventArgs e)
    {
        if (!string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        _clipboard.Clear();
        _clipboard.AddRange(SelectedRows().Select(row => row.Item.DeepClone().AsObject()));
        StatusText.Text = _clipboard.Count == 0 ? "Nothing selected" : $"Copied {_clipboard.Count} item(s)";
    }

    private void Paste_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedMenu is null || _clipboard.Count == 0 || !string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        foreach (var item in _clipboard) _selectedMenu.Items.Add(item.DeepClone());
        Changed();
        RefreshItems();
    }

    private void MoveToMenu_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedMenu is null || string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        var rows = SelectedRows();
        if (rows.Count == 0) return;
        var picker = new MoveItemsWindow(MenuChoices()) { Owner = this };
        if (picker.ShowDialog() != true || picker.Selected is null) return;
        var target = picker.Selected.Path.ToList();
        if (target.SequenceEqual(_selectedMenu.Path) || IsInsideSelection(target, rows)) return;
        var moving = rows.Select(row => row.Item.DeepClone()).ToList();
        // 同じ親の後ろにあるサブメニューへ移す場合、削除でその添字がずれる。
        if (target.Count > _selectedMenu.Path.Count && _selectedMenu.Path.SequenceEqual(target.Take(_selectedMenu.Path.Count)))
        {
            var level = _selectedMenu.Path.Count;
            target[level] -= rows.Count(row => row.Index < target[level]);
        }
        foreach (var row in rows.OrderByDescending(row => row.Index)) _selectedMenu.Items.RemoveAt(row.Index);
        var destination = ItemsAt(target);
        if (destination is not null) foreach (var item in moving) destination.Add(item);
        Changed();
        RefreshItems();
        RebuildTree();
    }

    private bool IsInsideSelection(IReadOnlyList<int> target, IReadOnlyList<ItemRow> rows)
    {
        if (_selectedMenu is null || target.Count <= _selectedMenu.Path.Count) return false;
        if (!_selectedMenu.Path.SequenceEqual(target.Take(_selectedMenu.Path.Count))) return false;
        return rows.Any(row => row.Index == target[_selectedMenu.Path.Count]);
    }

    private JsonArray? ItemsAt(IReadOnlyList<int> path)
    {
        JsonArray items = _store.Items;
        foreach (var index in path)
        {
            if (items[index] is not JsonObject item || Text(item, "type") != "submenu") return null;
            items = Array(item, "items");
        }
        return items;
    }

    private void SpecialFolders_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        _selectedMenu.Items.Add(MySpecialFolders());
        Changed();
        RefreshItems();
        RebuildTree();
    }

    private static JsonObject MySpecialFolders() => JsonNode.Parse("""
    {"type":"submenu","name":"My Special Folders","items":[{"type":"specialFolder","name":"Desktop","knownFolder":"Desktop"},{"type":"specialFolder","name":"Documents","knownFolder":"Documents"},{"type":"specialFolder","name":"Pictures","knownFolder":"Pictures"},{"type":"specialFolder","name":"Downloads","knownFolder":"Downloads"},{"type":"separator"},{"type":"shell","name":"This PC","target":"shell:MyComputerFolder"},{"type":"shell","name":"Network","target":"shell:NetworkPlacesFolder"},{"type":"shell","name":"All Control Panel Items","target":"shell:ControlPanelFolder"},{"type":"shell","name":"Recycle Bin","target":"shell:RecycleBinFolder"}]}
    """)!.AsObject();

    private void Import_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text)) return;
        var dialog = new FolderImportWindow { Owner = this };
        if (dialog.ShowDialog() != true || dialog.ResultItem is null) return;
        _selectedMenu.Items.Add(dialog.ResultItem);
        Changed();
        RefreshItems();
        RebuildTree();
    }

    private void ItemsGrid_PreviewMouseRightButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<DataGridRow>(e.OriginalSource as DependencyObject) is { } row) row.IsSelected = true;
    }

    private static T? FindAncestor<T>(DependencyObject? current) where T : DependencyObject
    {
        while (current is not null)
        {
            if (current is T match) return match;
            current = VisualTreeHelper.GetParent(current);
        }
        return null;
    }

    private void ItemsGrid_ContextMenuOpening(object sender, ContextMenuEventArgs e)
    {
        if (_selectedMenu is null || !string.IsNullOrWhiteSpace(ItemSearch.Text) || ItemsGrid.SelectedItem is not ItemRow { Type: "folder" }) e.Handled = true;
    }

    private void ImportSubfolders_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedMenu is null || ItemsGrid.SelectedItem is not ItemRow row || row.Type != "folder") return;
        var path = row.Item["path"]?.GetValue<string>() ?? "";
        var dialog = new FolderImportWindow(path) { Owner = this };
        if (dialog.ShowDialog() != true || dialog.ResultChildren is not { Count: > 0 } children) return;
        row.Item.Remove("path");
        row.Item.Remove("open");
        row.Item.Remove("icon");
        row.Item["type"] = "submenu";
        var items = new JsonArray { new JsonObject { ["type"] = "folder", ["name"] = "Open this folder", ["path"] = path } };
        foreach (var child in children) items.Add(child);
        row.Item["items"] = items;
        Changed();
        RefreshItems();
        RebuildTree();
    }

    private void ItemsGrid_PreviewMouseMove(object sender, MouseEventArgs e)
    {
        if (e.LeftButton != MouseButtonState.Pressed || ItemsGrid.SelectedItem is not ItemRow row || row.MenuPath is not null) return;
        var now = e.GetPosition(ItemsGrid);
        if (_dragStart is null) { _dragStart = now; return; }
        if (Math.Abs(now.X - _dragStart.Value.X) < SystemParameters.MinimumHorizontalDragDistance && Math.Abs(now.Y - _dragStart.Value.Y) < SystemParameters.MinimumVerticalDragDistance) return;
        DragDrop.DoDragDrop(ItemsGrid, new DataObject(typeof(ItemRow), row), DragDropEffects.Move);
        _dragStart = null;
    }

    private void ItemsGrid_Drop(object sender, DragEventArgs e)
    {
        if (_selectedMenu is null || !e.Data.GetDataPresent(typeof(ItemRow))) return;
        if (e.Data.GetData(typeof(ItemRow)) is not ItemRow source || source.MenuPath is not null) return;
        var target = ((e.OriginalSource as FrameworkElement)?.DataContext as ItemRow)?.Index ?? _selectedMenu.Items.Count;
        if (source.Index == target || source.Index + 1 == target) return;
        var item = _selectedMenu.Items[source.Index];
        _selectedMenu.Items.RemoveAt(source.Index);
        if (target > source.Index) target--;
        _selectedMenu.Items.Insert(target, item);
        Changed();
        RefreshItems();
    }

    private void Window_PreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (Keyboard.FocusedElement is TextBox && e.Key != Key.Escape) return;
        if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.F) { RootTabs.SelectedIndex = 0; ItemSearch.Focus(); e.Handled = true; }
        else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.A
            && RootTabs.SelectedIndex == 0 && string.IsNullOrWhiteSpace(ItemSearch.Text))
        {
            ItemsGrid.SelectAll();
            e.Handled = true;
        }
        else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.C) { Copy_Click(sender, e); e.Handled = true; }
        else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.V) { Paste_Click(sender, e); e.Handled = true; }
        else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.D) { Duplicate_Click(sender, e); e.Handled = true; }
        else if (Keyboard.Modifiers == ModifierKeys.Alt && e.Key == Key.Up) { MoveSelected(-1); e.Handled = true; }
        else if (Keyboard.Modifiers == ModifierKeys.Alt && e.Key == Key.Down) { MoveSelected(1); e.Handled = true; }
        else if (e.Key == Key.Delete) { Remove_Click(sender, e); e.Handled = true; }
        else if (e.Key == Key.Escape && ItemSearch.Text.Length > 0) { ItemSearch.Clear(); e.Handled = true; }
    }
}
