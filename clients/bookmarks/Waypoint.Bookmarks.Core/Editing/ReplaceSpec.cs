using System.Text.RegularExpressions;
using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Bookmarks.Core.Editing;

/// <summary>置換による 1 件の変更。</summary>
public sealed record ReplaceChange(BookmarkNode Node, string NewTitle, string? NewUrl);

/// <summary>名前・URL の一括置換の指定。</summary>
public sealed record ReplaceSpec(string Find, string Replacement, bool InTitle, bool InUrl, bool UseRegex, bool IgnoreCase)
{
    /// <summary>対象のうち、実際に変わるものだけを返す。置換後の URL が空になるものは URL を変えない。</summary>
    /// <exception cref="ArgumentException">正規表現が正しくないとき。</exception>
    public IReadOnlyList<ReplaceChange> Plan(IEnumerable<BookmarkNode> targets)
    {
        if (Find.Length == 0) return Array.Empty<ReplaceChange>();

        var options = IgnoreCase ? RegexOptions.IgnoreCase : RegexOptions.None;
        var regex = new Regex(UseRegex ? Find : Regex.Escape(Find), options);
        var replacement = UseRegex ? Replacement : Replacement.Replace("$", "$$");

        var changes = new List<ReplaceChange>();
        foreach (var n in targets)
        {
            var title = InTitle ? regex.Replace(n.Title, replacement) : n.Title;
            var url = n.Url;
            if (InUrl && url is not null)
            {
                var replaced = regex.Replace(url, replacement);
                if (!string.IsNullOrWhiteSpace(replaced)) url = replaced;
            }
            if (title != n.Title || url != n.Url) changes.Add(new ReplaceChange(n, title, url));
        }
        return changes;
    }
}
