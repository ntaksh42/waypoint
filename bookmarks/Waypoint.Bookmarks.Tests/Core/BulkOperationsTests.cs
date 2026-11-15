using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Bookmarks.Tests.Fakes;

namespace Waypoint.Bookmarks.Tests.Core;

public class BulkOperationsTests
{
    private readonly FakeBookmarkGateway _store = new();

    private string Order(string folderId) => string.Join(",", _store.ToTree().Find(folderId)!.Children.Select(n => n.Title));

    private EditSession NewSession() => new(_store, _store.ToTree());

    [Fact]
    public async Task SortFolder_puts_folders_first_then_sorts_by_name()
    {
        var f = _store.AddFolder(FakeBookmarkGateway.BarId, "root");
        _store.AddBookmark(f, "banana", "https://b.example");
        _store.AddFolder(f, "zeta");
        _store.AddBookmark(f, "apple", "https://a.example");
        _store.AddFolder(f, "alpha");

        await BulkOperations.SortFolderAsync(NewSession(), _store.ToTree().Find(f)!);

        Assert.Equal("alpha,zeta,apple,banana", Order(f));
    }

    [Fact]
    public async Task SortFolder_can_be_undone()
    {
        var f = _store.AddFolder(FakeBookmarkGateway.BarId, "root");
        _store.AddBookmark(f, "c", "https://c.example");
        _store.AddBookmark(f, "a", "https://a.example");
        _store.AddBookmark(f, "b", "https://b.example");

        var session = NewSession();
        await BulkOperations.SortFolderAsync(session, _store.ToTree().Find(f)!);
        Assert.Equal("a,b,c", Order(f));

        await session.ToUndoEntry("sort")!.ApplyAsync();
        Assert.Equal("c,a,b", Order(f));
    }

    [Fact]
    public async Task SortFolder_sends_nothing_when_already_sorted()
    {
        var f = _store.AddFolder(FakeBookmarkGateway.BarId, "root");
        _store.AddBookmark(f, "a", "https://a.example");
        _store.AddBookmark(f, "b", "https://b.example");

        await BulkOperations.SortFolderAsync(NewSession(), _store.ToTree().Find(f)!);

        Assert.Empty(_store.Calls);
    }

    [Fact]
    public async Task Nudge_up_moves_selected_items_by_one_and_keeps_those_at_the_top()
    {
        var f = _store.AddFolder(FakeBookmarkGateway.BarId, "root");
        var a = _store.AddBookmark(f, "A", "https://a.example");
        _store.AddBookmark(f, "B", "https://b.example");
        var c = _store.AddBookmark(f, "C", "https://c.example");
        var tree = _store.ToTree();

        await BulkOperations.NudgeAsync(NewSession(), new[] { tree.Find(a)!, tree.Find(c)! }, -1);

        Assert.Equal("A,C,B", Order(f));
    }

    [Fact]
    public async Task Nudge_down_moves_selected_items_by_one_and_keeps_those_at_the_bottom()
    {
        var f = _store.AddFolder(FakeBookmarkGateway.BarId, "root");
        var a = _store.AddBookmark(f, "A", "https://a.example");
        var b = _store.AddBookmark(f, "B", "https://b.example");
        var c = _store.AddBookmark(f, "C", "https://c.example");
        var tree = _store.ToTree();

        await BulkOperations.NudgeAsync(NewSession(), new[] { tree.Find(a)!, tree.Find(b)!, tree.Find(c)! }, 1);

        Assert.Equal("A,B,C", Order(f));
        Assert.Empty(_store.Calls);
    }

    [Fact]
    public async Task Nudge_down_swaps_with_the_next_item()
    {
        var f = _store.AddFolder(FakeBookmarkGateway.BarId, "root");
        var a = _store.AddBookmark(f, "A", "https://a.example");
        _store.AddBookmark(f, "B", "https://b.example");
        _store.AddBookmark(f, "C", "https://c.example");

        await BulkOperations.NudgeAsync(NewSession(), new[] { _store.ToTree().Find(a)! }, 1);

        Assert.Equal("B,A,C", Order(f));
    }

    [Fact]
    public async Task Nudge_with_invalid_direction_throws()
    {
        await Assert.ThrowsAsync<ArgumentOutOfRangeException>(
            () => BulkOperations.NudgeAsync(NewSession(), Array.Empty<BookmarkNode>(), 2));
    }

    [Fact]
    public async Task MoveToFolder_appends_in_order_and_skips_items_already_there()
    {
        var src = _store.AddFolder(FakeBookmarkGateway.BarId, "src");
        var dst = _store.AddFolder(FakeBookmarkGateway.BarId, "dst");
        var a = _store.AddBookmark(src, "A", "https://a.example");
        var b = _store.AddBookmark(src, "B", "https://b.example");
        var already = _store.AddBookmark(dst, "already", "https://d.example");
        var tree = _store.ToTree();

        await BulkOperations.MoveToFolderAsync(NewSession(), new[] { tree.Find(a)!, tree.Find(already)!, tree.Find(b)! }, dst);

        Assert.Equal("already,A,B", Order(dst));
        Assert.Equal("", Order(src));
        Assert.Equal(2, _store.Calls.Count);
    }

    [Fact]
    public async Task MoveToFolder_can_be_undone()
    {
        var src = _store.AddFolder(FakeBookmarkGateway.BarId, "src");
        var dst = _store.AddFolder(FakeBookmarkGateway.BarId, "dst");
        var a = _store.AddBookmark(src, "A", "https://a.example");
        _store.AddBookmark(src, "B", "https://b.example");
        var c = _store.AddBookmark(src, "C", "https://c.example");
        var tree = _store.ToTree();

        var session = NewSession();
        await BulkOperations.MoveToFolderAsync(session, new[] { tree.Find(a)!, tree.Find(c)! }, dst);
        Assert.Equal("B", Order(src));

        await session.ToUndoEntry("move")!.ApplyAsync();
        Assert.Equal("A,B,C", Order(src));
        Assert.Equal("", Order(dst));
    }
}
