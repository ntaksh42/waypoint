using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;

namespace Waypoint.Bookmarks.Tests.Fakes;

/// <summary>chrome.bookmarks と同じ規則 (特に move の挿入位置) で動く、メモリ上のお気に入り。</summary>
internal sealed class FakeBookmarkGateway : IBookmarkGateway
{
    private sealed class Item
    {
        public Item(string id, string title, string? url, Item? parent)
        {
            Id = id;
            Title = title;
            Url = url;
            Parent = parent;
        }

        public string Id { get; }
        public string Title { get; set; }
        public string? Url { get; set; }
        public Item? Parent { get; set; }
        public List<Item> Kids { get; } = new();
    }

    private readonly Dictionary<string, Item> _items = new();
    private readonly Item _root;
    private int _nextId = 100;

    public const string BarId = "1";
    public const string OtherId = "2";

    public FakeBookmarkGateway()
    {
        _root = new Item(BookmarkIds.Root, "", null, null);
        _items[_root.Id] = _root;
        Register(new Item(BarId, "お気に入りバー", null, _root));
        Register(new Item(OtherId, "その他のお気に入り", null, _root));
    }

    /// <summary>書き込みの呼び出し記録 (操作名と引数)。</summary>
    public List<string> Calls { get; } = new();

    public string AddFolder(string parentId, string title) => Register(new Item(NewId(), title, null, _items[parentId]));

    public string AddBookmark(string parentId, string title, string url) => Register(new Item(NewId(), title, url, _items[parentId]));

    private string NewId() => (_nextId++).ToString();

    private string Register(Item item)
    {
        _items[item.Id] = item;
        item.Parent!.Kids.Add(item);
        return item.Id;
    }

    public Task UpdateAsync(string id, string title, string? url)
    {
        Calls.Add($"update {id}");
        var item = _items[id];
        item.Title = title;
        if (item.Url is not null && url is not null) item.Url = url;
        return Task.CompletedTask;
    }

    public Task MoveAsync(string id, string parentId, int? index)
    {
        Calls.Add($"move {id} -> {parentId}@{index}");
        var item = _items[id];
        var oldParent = item.Parent!;
        var oldIndex = oldParent.Kids.IndexOf(item);
        var newParent = _items[parentId];

        oldParent.Kids.RemoveAt(oldIndex);
        var position = index ?? newParent.Kids.Count;
        if (oldParent == newParent && position > oldIndex) position--;
        newParent.Kids.Insert(Math.Clamp(position, 0, newParent.Kids.Count), item);
        item.Parent = newParent;
        return Task.CompletedTask;
    }

    public Task RemoveAsync(string id, bool recursive)
    {
        Calls.Add($"remove {id}");
        var item = _items[id];
        if (item.Kids.Count > 0 && !recursive) throw new InvalidOperationException("空でないフォルダは再帰指定が必要です。");
        item.Parent!.Kids.Remove(item);
        Forget(item);
        return Task.CompletedTask;
    }

    private void Forget(Item item)
    {
        _items.Remove(item.Id);
        foreach (var k in item.Kids) Forget(k);
    }

    public Task<string> CreateAsync(string parentId, string title, string? url, int? index)
    {
        Calls.Add($"create {title}");
        var parent = _items[parentId];
        var item = new Item(NewId(), title, url, parent);
        _items[item.Id] = item;
        parent.Kids.Insert(Math.Clamp(index ?? parent.Kids.Count, 0, parent.Kids.Count), item);
        return Task.FromResult(item.Id);
    }

    public BookmarkTree ToTree() => new(_root.Kids.Select((k, i) => Build(k, i)));

    public BookmarkSnapshot ToSnapshot() => new(ToTree(), "[]");

    private static BookmarkNode Build(Item item, int index) =>
        new(item.Id, item.Title, item.Url, item.Parent!.Id, index, item.Kids.Select((k, i) => Build(k, i)));
}
