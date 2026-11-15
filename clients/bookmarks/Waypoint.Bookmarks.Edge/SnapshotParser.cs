using System.Text.Json;
using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Bookmarks.Edge;

/// <summary>拡張機能が送るスナップショット (JSON の配列) を <see cref="BookmarkTree"/> にする。</summary>
internal static class SnapshotParser
{
    public static BookmarkTree Parse(JsonElement roots) =>
        new(roots.EnumerateArray().Select(ParseNode));

    private static BookmarkNode ParseNode(JsonElement e)
    {
        var children = e.TryGetProperty("children", out var ch) && ch.ValueKind == JsonValueKind.Array
            ? ch.EnumerateArray().Select(ParseNode)
            : null;

        return new BookmarkNode(
            id: e.GetProperty("id").GetString()!,
            title: e.TryGetProperty("title", out var t) ? t.GetString() ?? "" : "",
            url: e.TryGetProperty("url", out var u) && u.ValueKind == JsonValueKind.String ? u.GetString() : null,
            parentId: e.TryGetProperty("parentId", out var p) ? p.GetString() ?? "" : "",
            index: e.TryGetProperty("index", out var i) && i.ValueKind == JsonValueKind.Number ? i.GetInt32() : 0,
            children: children);
    }
}
