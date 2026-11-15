namespace Waypoint.Bookmarks.Core.Bookmarks;

/// <summary>お気に入りまたはフォルダの 1 件。作成後は変更できない (Edge から届いた時点の状態)。</summary>
public sealed class BookmarkNode
{
    private readonly List<BookmarkNode> _children;

    /// <param name="children">子。ここで親子関係が結ばれるため、1 つの親にしか渡せない。</param>
    public BookmarkNode(string id, string title, string? url, string parentId, int index,
        IEnumerable<BookmarkNode>? children = null)
    {
        Id = id;
        Title = title;
        Url = url;
        ParentId = parentId;
        Index = index;
        _children = children?.ToList() ?? new List<BookmarkNode>();
        foreach (var child in _children) child.AttachTo(this);
    }

    public string Id { get; }
    public string Title { get; }

    /// <summary>お気に入りの URL。フォルダは null。</summary>
    public string? Url { get; }

    public string ParentId { get; }

    /// <summary>親の中での位置 (0 から)。</summary>
    public int Index { get; }

    public BookmarkNode? Parent { get; private set; }
    public IReadOnlyList<BookmarkNode> Children => _children;

    public bool IsFolder => Url is null;

    /// <summary>お気に入りバーなど、Edge が用意する最上位のフォルダ。名前変更・移動・削除はできない。</summary>
    public bool IsRoot => ParentId == BookmarkIds.Root;

    /// <summary>この節点より下にあるお気に入りの件数 (フォルダは数えない)。</summary>
    public int BookmarkCount => Descendants().Count(n => !n.IsFolder);

    /// <summary>親フォルダの名前を「 / 」でつないだもの。最上位は空文字列。</summary>
    public string FolderPath
    {
        get
        {
            var names = new List<string>();
            for (var p = Parent; p is not null; p = p.Parent) names.Add(p.Title);
            names.Reverse();
            return string.Join(" / ", names);
        }
    }

    public IEnumerable<BookmarkNode> Descendants()
    {
        foreach (var child in _children)
        {
            yield return child;
            foreach (var d in child.Descendants()) yield return d;
        }
    }

    public bool IsDescendantOf(BookmarkNode other)
    {
        for (var p = Parent; p is not null; p = p.Parent)
            if (p.Id == other.Id) return true;
        return false;
    }

    private void AttachTo(BookmarkNode parent)
    {
        if (Parent is not null) throw new InvalidOperationException($"節点 {Id} は既に別の親を持っています。");
        Parent = parent;
    }
}
