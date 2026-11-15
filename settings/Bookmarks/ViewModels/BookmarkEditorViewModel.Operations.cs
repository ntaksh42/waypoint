using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;

namespace Waypoint.Settings.Bookmarks.ViewModels;

/// <summary>お気に入りとフォルダへの操作 (一括操作、フォルダ操作、ドラッグ＆ドロップ)。</summary>
internal sealed partial class BookmarkEditorViewModel
{
    // ---- 一覧の操作 ----

    /// <summary>セルの編集内容が確定できるか。URL を空にはできない。</summary>
    public bool TryValidateEdit(bool isTitleColumn, string text)
    {
        if (isTitleColumn || text.Trim().Length > 0) return true;
        _dialogs.ShowInfo(AppTitle, "Enter a URL.");
        return false;
    }

    public Task CommitEditAsync(RowViewModel row, bool isTitleColumn, string text)
    {
        var title = isTitleColumn ? text : row.Node.Title;
        var url = isTitleColumn ? row.Node.Url : text.Trim();
        if (title == row.Node.Title && url == row.Node.Url) return Task.CompletedTask;
        return EditAsync("Edit", s => s.UpdateAsync(row.Node, title, url));
    }

    private Task DeleteSelectedRowsAsync()
    {
        var rows = SelectedRows();
        if (rows.Count == 0) return Task.CompletedTask;

        var names = string.Join("\n", rows.Take(8).Select(r => "• " + (r.Title.Length > 0 ? r.Title : r.Url)));
        var more = rows.Count > 8 ? $"\n\u2026 and {rows.Count - 8} more" : "";
        if (!_dialogs.Confirm("Delete", $"Delete {rows.Count} bookmark(s)?\n\n{names}{more}\n\nYou can restore them with Undo."))
            return Task.CompletedTask;

        var nodes = rows.Select(r => r.Node).ToList();
        return EditAsync($"Delete {nodes.Count}", s => BulkOperations.RemoveAsync(s, nodes), backupFirst: nodes.Count > 1);
    }

    private Task MoveSelectedRowsAsync()
    {
        var nodes = SelectedRows().Select(r => r.Node).ToList();
        if (nodes.Count == 0) return Task.CompletedTask;

        return _dialogs.PickFolder($"Move {nodes.Count} bookmark(s) to", _tree) is { } destination
            ? MoveNodesToFolderAsync(nodes, destination.Id)
            : Task.CompletedTask;
    }

    private Task MoveNodesToFolderAsync(IReadOnlyCollection<BookmarkNode> nodes, string folderId) =>
        EditAsync($"Move {nodes.Count}", s => BulkOperations.MoveToFolderAsync(s, nodes, folderId));

    private Task NudgeAsync(int direction)
    {
        var nodes = SelectedRows().Select(r => r.Node).ToList();
        return nodes.Count == 0
            ? Task.CompletedTask
            : EditAsync(direction < 0 ? "Move up" : "Move down", s => BulkOperations.NudgeAsync(s, nodes, direction));
    }

    private Task ReplaceAsync()
    {
        var selected = SelectedRows();
        var targets = (selected.Count > 0 ? selected : Rows.ToList()).Select(r => r.Node).ToList();
        if (targets.Count == 0) return Task.CompletedTask;

        if (_dialogs.AskReplace(targets, selected.Count > 0 ? "Selected" : "Shown") is not { } spec) return Task.CompletedTask;
        var changes = spec.Plan(targets);
        return changes.Count == 0
            ? Task.CompletedTask
            : EditAsync($"Replace {changes.Count}", s => BulkOperations.ApplyReplaceAsync(s, changes), backupFirst: true);
    }

    private Task RemoveDuplicatesAsync()
    {
        var extras = DuplicateFinder.FindExtras(_tree);
        if (extras.Count == 0)
        {
            _dialogs.ShowInfo("Remove duplicates", "There are no duplicate bookmarks.");
            return Task.CompletedTask;
        }

        var names = string.Join("\n", extras.Take(10).Select(d => $"• {d.Title}"));
        var more = extras.Count > 10 ? $"\n\u2026 and {extras.Count - 10} more" : "";
        if (!_dialogs.Confirm("Remove duplicates", $"Delete {extras.Count} duplicate bookmark(s)? The first bookmark for each URL is kept.\n\n{names}{more}"))
            return Task.CompletedTask;

        return EditAsync($"Remove {extras.Count} duplicates", s => BulkOperations.RemoveAsync(s, extras), backupFirst: true);
    }

    private void OpenSelectedUrls()
    {
        foreach (var row in SelectedRows().Take(MaxOpenUrls))
            if (Uri.TryCreate(row.Node.Url, UriKind.Absolute, out var uri) && uri.Scheme is "http" or "https")
                _dialogs.OpenUrl(uri.AbsoluteUri);
    }

    // ---- フォルダの操作 ----

    private async Task NewFolderAsync()
    {
        var parentId = SelectedFolder?.Node?.Id ?? _tree.DefaultFolder?.Id;
        if (parentId is null) return;
        if (_dialogs.Ask("New folder", "Folder name", "New folder") is not { } name) return;

        string? createdId = null;
        await EditAsync("New folder", async s => createdId = await s.CreateFolderAsync(parentId, name));
        if (createdId is not null) SelectFolder(createdId);
    }

    /// <summary>最新のフォルダツリーから該当のフォルダを選び、親をすべて開く。</summary>
    private void SelectFolder(string folderId)
    {
        var folder = FolderRoots.SelectMany(r => r.SelfAndDescendants()).FirstOrDefault(f => f.Id == folderId);
        if (folder is null) return;
        for (var p = folder.Parent; p is not null; p = p.Parent) p.IsExpanded = true;
        folder.IsSelected = true;
    }

    private Task RenameFolderAsync()
    {
        if (SelectedFolder?.Node is not { IsRoot: false } node) return Task.CompletedTask;
        if (_dialogs.Ask("Rename folder", "Folder name", node.Title) is not { } name) return Task.CompletedTask;
        return EditAsync("Rename folder", s => s.UpdateAsync(node, name, null));
    }

    private Task DeleteFolderAsync()
    {
        if (SelectedFolder?.Node is not { IsRoot: false } node) return Task.CompletedTask;
        if (!_dialogs.Confirm("Delete folder",
                $"Delete the folder \"{node.Title}\" and the {node.BookmarkCount} bookmark(s) in it?\nYou can restore it with Undo."))
            return Task.CompletedTask;

        _scopeFolderId = node.ParentId;
        return EditAsync("Delete folder", s => s.RemoveAsync(node), backupFirst: true);
    }

    private Task SortFolderAsync()
    {
        if (SelectedFolder?.Node is not { } node) return Task.CompletedTask;
        if (!_dialogs.Confirm("Sort by name", $"Sort the items directly in \"{node.Title}\" by name, folders first?"))
            return Task.CompletedTask;
        return EditAsync("Sort by name", s => BulkOperations.SortFolderAsync(s, node), backupFirst: true);
    }

    // ---- ドラッグ＆ドロップ (View から呼ばれる) ----

    public bool CanDragFolder(FolderViewModel folder) => CanEdit && folder.Node is { IsRoot: false };

    public bool CanDragRows() => CanEdit;

    public bool CanDropRows(FolderViewModel target) => CanEdit && target.Node is not null;

    public bool CanDropFolder(string folderId, FolderViewModel target)
    {
        if (!CanEdit || target.Node is not { } targetNode || _tree.Find(folderId) is not { } source) return false;
        return targetNode.Id != source.Id && !targetNode.IsDescendantOf(source);
    }

    public Task DropRowsAsync(IEnumerable<string> rowIds, FolderViewModel target)
    {
        if (target.Node is null) return Task.CompletedTask;
        var ids = rowIds.ToHashSet();
        var nodes = _tree.Bookmarks.Where(n => ids.Contains(n.Id)).ToList();
        return nodes.Count == 0 ? Task.CompletedTask : MoveNodesToFolderAsync(nodes, target.Node.Id);
    }

    public Task DropFolderAsync(string folderId, FolderViewModel target)
    {
        if (target.Node is null) return Task.CompletedTask;
        return EditAsync("Move folder", s => s.MoveAsync(folderId, target.Node.Id, null));
    }

    public IReadOnlyList<string> SelectedRowIds() => SelectedRows().Select(r => r.Node.Id).ToList();
}
