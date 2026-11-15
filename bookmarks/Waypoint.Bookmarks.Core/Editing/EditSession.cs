using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Bookmarks.Core.Editing;

/// <summary>
/// 1 回の操作 (複数件をまとめて変更) の間だけ使う。ブラウザへの書き込みと同時に、元に戻すための逆操作を記録する。
/// 書き込みの結果がスナップショットとして届く前でも並び順を追えるよう、フォルダごとの子 ID の並びを手元で模擬する。
/// </summary>
public sealed class EditSession
{
    private readonly IBookmarkGateway _gateway;
    private readonly Dictionary<string, List<string>> _childIds = new();
    private readonly Dictionary<string, string> _parentOf = new();
    private readonly List<Func<Task>> _inverses = new();

    public EditSession(IBookmarkGateway gateway, BookmarkTree tree)
    {
        _gateway = gateway;
        _childIds[BookmarkIds.Root] = tree.Roots.Select(r => r.Id).ToList();
        foreach (var n in tree.Flatten())
        {
            _parentOf[n.Id] = n.ParentId;
            if (n.IsFolder) _childIds[n.Id] = n.Children.Select(c => c.Id).ToList();
        }
    }

    /// <summary>ここまでの操作の現在の並び (書き込みを反映済み)。</summary>
    public IReadOnlyList<string> ChildIds(string folderId) => _childIds[folderId];

    public string ParentIdOf(string id) => _parentOf[id];

    /// <summary>ここまでに記録した逆操作。何も変えていなければ null。</summary>
    public UndoEntry? ToUndoEntry(string label) => _inverses.Count == 0 ? null : new UndoEntry(label, _inverses.ToList());

    public async Task UpdateAsync(BookmarkNode node, string title, string? url)
    {
        var newUrl = node.IsFolder ? null : url ?? node.Url;
        if (title == node.Title && newUrl == node.Url) return;

        var id = node.Id;
        var oldTitle = node.Title;
        var oldUrl = node.Url;
        await _gateway.UpdateAsync(id, title, newUrl);
        _inverses.Add(() => _gateway.UpdateAsync(id, oldTitle, oldUrl));
    }

    /// <param name="index">移動前の並びでの挿入位置 (<see cref="IBookmarkGateway.MoveAsync"/> と同じ)。null なら末尾。</param>
    public async Task MoveAsync(string id, string parentId, int? index)
    {
        var oldParent = _parentOf[id];
        var oldList = _childIds[oldParent];
        var oldIndex = oldList.IndexOf(id);
        var newList = _childIds[parentId];

        int position;
        if (oldParent == parentId)
        {
            position = index is null ? newList.Count - 1 : index.Value > oldIndex ? index.Value - 1 : index.Value;
            position = Math.Clamp(position, 0, newList.Count - 1);
            if (position == oldIndex) return;
        }
        else
        {
            position = index is null ? newList.Count : Math.Min(index.Value, newList.Count);
        }

        await _gateway.MoveAsync(id, parentId, index);
        oldList.RemoveAt(oldIndex);
        newList.Insert(position, id);
        _parentOf[id] = parentId;

        // 同じフォルダ内で元の位置へ戻すとき、挿入位置は移動前の並びで数えるため、後ろへ戻す場合だけ 1 足す
        var back = oldParent == parentId && oldIndex >= position ? oldIndex + 1 : oldIndex;
        _inverses.Add(() => _gateway.MoveAsync(id, oldParent, back));
    }

    /// <summary>節点を削除する。フォルダは中身ごと消える。元に戻すときは、同じ位置に新しい ID で作り直す。</summary>
    public async Task RemoveAsync(BookmarkNode node)
    {
        var parent = _parentOf[node.Id];
        var index = _childIds[parent].IndexOf(node.Id);
        await _gateway.RemoveAsync(node.Id, recursive: node.IsFolder);

        _childIds[parent].RemoveAt(index);
        foreach (var gone in node.Descendants().Append(node))
        {
            _childIds.Remove(gone.Id);
            _parentOf.Remove(gone.Id);
        }
        _inverses.Add(() => RecreateAsync(node, parent, index));
    }

    private async Task RecreateAsync(BookmarkNode node, string parentId, int index)
    {
        var id = await _gateway.CreateAsync(parentId, node.Title, node.Url, index);
        for (var i = 0; i < node.Children.Count; i++) await RecreateAsync(node.Children[i], id, i);
    }

    /// <returns>作ったフォルダの ID。</returns>
    public async Task<string> CreateFolderAsync(string parentId, string title)
    {
        var id = await _gateway.CreateAsync(parentId, title, url: null, index: null);
        _childIds[parentId].Add(id);
        _childIds[id] = new List<string>();
        _parentOf[id] = parentId;
        _inverses.Add(() => _gateway.RemoveAsync(id, recursive: true));
        return id;
    }
}
