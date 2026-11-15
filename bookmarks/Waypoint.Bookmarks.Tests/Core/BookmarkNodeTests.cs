using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Bookmarks.Tests.Core;

public class BookmarkNodeTests
{
    private static BookmarkTree Sample()
    {
        var leaf = new BookmarkNode("l", "leaf", "https://x.test/", "f2", 0);
        var inner = new BookmarkNode("f2", "inner", null, "f1", 0, new[] { leaf });
        var outer = new BookmarkNode("f1", "outer", null, "1", 0, new[] { inner, new BookmarkNode("m", "mid", "https://m.test/", "f1", 1) });
        return new BookmarkTree(new[] { new BookmarkNode("1", "bar", null, BookmarkIds.Root, 0, new[] { outer }) });
    }

    [Fact]
    public void Constructing_a_parent_links_its_children()
    {
        var tree = Sample();

        Assert.Same(tree.Find("f1"), tree.Find("f2")!.Parent);
        Assert.Null(tree.Find("1")!.Parent);
    }

    [Fact]
    public void FolderPath_lists_ancestors_from_the_top()
    {
        Assert.Equal("bar / outer / inner", Sample().Find("l")!.FolderPath);
        Assert.Equal("", Sample().Find("1")!.FolderPath);
    }

    [Fact]
    public void BookmarkCount_counts_only_bookmarks_below()
    {
        var tree = Sample();

        Assert.Equal(2, tree.Find("f1")!.BookmarkCount);
        Assert.Equal(1, tree.Find("f2")!.BookmarkCount);
        Assert.Equal(0, tree.Find("l")!.BookmarkCount);
    }

    [Fact]
    public void IsDescendantOf_follows_the_parent_chain()
    {
        var tree = Sample();

        Assert.True(tree.Find("l")!.IsDescendantOf(tree.Find("1")!));
        Assert.False(tree.Find("f1")!.IsDescendantOf(tree.Find("f2")!));
        Assert.False(tree.Find("f1")!.IsDescendantOf(tree.Find("f1")!));
    }

    [Fact]
    public void IsFolder_and_IsRoot_follow_url_and_parent()
    {
        var tree = Sample();

        Assert.True(tree.Find("1")!.IsRoot);
        Assert.False(tree.Find("f1")!.IsRoot);
        Assert.True(tree.Find("f1")!.IsFolder);
        Assert.False(tree.Find("l")!.IsFolder);
    }

    [Fact]
    public void A_node_cannot_have_two_parents()
    {
        var shared = new BookmarkNode("s", "s", "https://s.test/", "p1", 0);
        _ = new BookmarkNode("p1", "p1", null, "0", 0, new[] { shared });

        Assert.Throws<InvalidOperationException>(() => new BookmarkNode("p2", "p2", null, "0", 1, new[] { shared }));
    }

    [Fact]
    public void Flatten_lists_parents_before_children_in_display_order()
    {
        Assert.Equal(new[] { "1", "f1", "f2", "l", "m" }, Sample().Flatten().Select(n => n.Id));
    }

    [Fact]
    public void DefaultFolder_prefers_the_second_root()
    {
        var two = new BookmarkTree(new[]
        {
            new BookmarkNode("1", "bar", null, "0", 0),
            new BookmarkNode("2", "other", null, "0", 1),
        });

        Assert.Equal("2", two.DefaultFolder!.Id);
        Assert.Equal("1", new BookmarkTree(new[] { new BookmarkNode("1", "bar", null, "0", 0) }).DefaultFolder!.Id);
        Assert.Null(BookmarkTree.Empty.DefaultFolder);
    }
}
