using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Bookmarks.Tests.Fakes;

namespace Waypoint.Bookmarks.Tests.Core;

public class EditSessionTests
{
    private readonly FakeBookmarkGateway _store = new();
    private string _folder = "";
    private string _a = "";
    private string _b = "";
    private string _c = "";

    public EditSessionTests()
    {
        _folder = _store.AddFolder(FakeBookmarkGateway.OtherId, "F");
        _a = _store.AddBookmark(_folder, "A", "https://example.com/a");
        _b = _store.AddBookmark(_folder, "B", "https://example.com/b");
        _c = _store.AddBookmark(_folder, "C", "https://example.com/c");
    }

    private EditSession NewSession() => new(_store, _store.ToTree());

    private string Order() => string.Join(",", _store.ToTree().Find(_folder)!.Children.Select(n => n.Title));

    [Fact]
    public async Task Move_forward_inserts_before_the_given_index()
    {
        await NewSession().MoveAsync(_a, _folder, 3);
        Assert.Equal("B,C,A", Order());
    }

    [Fact]
    public async Task Move_to_same_position_sends_nothing()
    {
        await NewSession().MoveAsync(_b, _folder, 1);
        await NewSession().MoveAsync(_b, _folder, 2);
        Assert.Empty(_store.Calls);
    }

    [Theory]
    [InlineData(0, 3)]
    [InlineData(2, 0)]
    [InlineData(1, 0)]
    public async Task Undo_restores_the_original_order_for_any_move(int from, int to)
    {
        var ids = new[] { _a, _b, _c };
        var session = NewSession();
        await session.MoveAsync(ids[from], _folder, to);

        await session.ToUndoEntry("move")!.ApplyAsync();

        Assert.Equal("A,B,C", Order());
    }

    [Fact]
    public async Task Undo_of_several_moves_restores_the_original_order()
    {
        var session = NewSession();
        await session.MoveAsync(_c, _folder, 0);
        await session.MoveAsync(_a, _folder, 3);
        await session.MoveAsync(_b, FakeBookmarkGateway.BarId, null);

        await session.ToUndoEntry("moves")!.ApplyAsync();

        Assert.Equal("A,B,C", Order());
        Assert.Empty(_store.ToTree().Find(FakeBookmarkGateway.BarId)!.Children);
    }

    [Fact]
    public async Task Update_changes_title_and_url_and_undo_restores_them()
    {
        var session = NewSession();
        var node = _store.ToTree().Find(_a)!;
        await session.UpdateAsync(node, "A2", "https://example.org/a");

        var changed = _store.ToTree().Find(_a)!;
        Assert.Equal(("A2", "https://example.org/a"), (changed.Title, changed.Url));

        await session.ToUndoEntry("edit")!.ApplyAsync();
        var restored = _store.ToTree().Find(_a)!;
        Assert.Equal(("A", "https://example.com/a"), (restored.Title, restored.Url));
    }

    [Fact]
    public async Task Update_without_changes_does_nothing()
    {
        var session = NewSession();
        await session.UpdateAsync(_store.ToTree().Find(_a)!, "A", null);

        Assert.Empty(_store.Calls);
        Assert.Null(session.ToUndoEntry("edit"));
    }

    [Fact]
    public async Task Removing_a_folder_and_undoing_recreates_it_with_its_children_in_place()
    {
        var other = FakeBookmarkGateway.OtherId;
        var tail = _store.AddBookmark(other, "tail", "https://example.com/t");
        var session = NewSession();

        await session.RemoveAsync(_store.ToTree().Find(_folder)!);
        Assert.Null(_store.ToTree().Find(_folder));

        await session.ToUndoEntry("remove")!.ApplyAsync();

        var otherNode = _store.ToTree().Find(other)!;
        Assert.Equal(new[] { "F", "tail" }, otherNode.Children.Select(n => n.Title));
        Assert.Equal(new[] { "A", "B", "C" }, otherNode.Children[0].Children.Select(n => n.Title));
        Assert.NotNull(_store.ToTree().Find(tail));
    }

    [Fact]
    public async Task Removing_several_bookmarks_and_undoing_restores_their_positions()
    {
        var session = NewSession();
        var tree = _store.ToTree();
        await session.RemoveAsync(tree.Find(_a)!);
        await session.RemoveAsync(tree.Find(_c)!);
        Assert.Equal("B", Order());

        await session.ToUndoEntry("remove")!.ApplyAsync();
        Assert.Equal("A,B,C", Order());
    }

    [Fact]
    public async Task CreateFolder_can_be_undone()
    {
        var session = NewSession();
        var id = await session.CreateFolderAsync(FakeBookmarkGateway.BarId, "new");
        Assert.NotNull(_store.ToTree().Find(id));

        await session.ToUndoEntry("create")!.ApplyAsync();
        Assert.Null(_store.ToTree().Find(id));
    }

    [Fact]
    public async Task Session_tracks_order_without_a_fresh_snapshot()
    {
        var session = NewSession();
        await session.MoveAsync(_c, _folder, 0);
        await session.MoveAsync(_a, _folder, 0);

        Assert.Equal(new[] { _a, _c, _b }, session.ChildIds(_folder));
        Assert.Equal("A,C,B", Order());
    }
}
