namespace Waypoint.Bookmarks.Core.Bookmarks;

/// <summary>ある時点のお気に入り全体。最上位フォルダ (お気に入りバーなど) の並びと、ID での検索を持つ。</summary>
public sealed class BookmarkTree
{
    public static BookmarkTree Empty { get; } = new(Array.Empty<BookmarkNode>());

    private readonly Dictionary<string, BookmarkNode> _byId;

    public BookmarkTree(IEnumerable<BookmarkNode> roots)
    {
        Roots = roots.ToList();
        _byId = Flatten().ToDictionary(n => n.Id);
    }

    public IReadOnlyList<BookmarkNode> Roots { get; }

    public bool IsEmpty => Roots.Count == 0;

    /// <summary>新しいフォルダの既定の置き場。2 番目の最上位 (その他のお気に入り) があればそれ、なければ先頭。</summary>
    public BookmarkNode? DefaultFolder => Roots.Count > 1 ? Roots[1] : Roots.FirstOrDefault();

    public BookmarkNode? Find(string id) => _byId.GetValueOrDefault(id);

    /// <summary>全節点を、親 → 子の順 (表示順) に返す。</summary>
    public IEnumerable<BookmarkNode> Flatten()
    {
        foreach (var root in Roots)
        {
            yield return root;
            foreach (var d in root.Descendants()) yield return d;
        }
    }

    public IEnumerable<BookmarkNode> Bookmarks => Flatten().Where(n => !n.IsFolder);
    public IEnumerable<BookmarkNode> Folders => Flatten().Where(n => n.IsFolder);
}
