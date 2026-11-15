using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Bookmarks.Core.Editing;

/// <summary>同じ URL のお気に入りを見つける。大文字小文字・末尾の「/」の違いは同じ URL とみなす。</summary>
public static class DuplicateFinder
{
    public static string KeyOf(string url)
    {
        try { return new Uri(url.Trim()).AbsoluteUri.TrimEnd('/'); }
        catch (UriFormatException) { return url.Trim().TrimEnd('/'); }
    }

    /// <summary>URL ごとの件数。URL が空のお気に入りは数えない。</summary>
    public static Dictionary<string, int> CountByUrl(BookmarkTree tree) =>
        tree.Bookmarks
            .Where(n => !string.IsNullOrWhiteSpace(n.Url))
            .GroupBy(n => KeyOf(n.Url!))
            .ToDictionary(g => g.Key, g => g.Count());

    /// <summary>同じ URL の 2 件目以降 (先に出てきた 1 件は残す側とみなす)。</summary>
    public static List<BookmarkNode> FindExtras(BookmarkTree tree)
    {
        var seen = new HashSet<string>();
        return tree.Bookmarks
            .Where(n => !string.IsNullOrWhiteSpace(n.Url) && !seen.Add(KeyOf(n.Url!)))
            .ToList();
    }
}
