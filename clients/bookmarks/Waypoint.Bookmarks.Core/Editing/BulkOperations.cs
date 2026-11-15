using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Bookmarks.Core.Editing;

/// <summary>並べ替えや移動など、<see cref="EditSession"/> の上に組む複数件の操作。</summary>
public static class BulkOperations
{
    /// <summary>フォルダの直下を「フォルダ → 名前順」に並べ替える。</summary>
    public static async Task SortFolderAsync(EditSession session, BookmarkNode folder)
    {
        var desired = folder.Children
            .OrderBy(c => c.IsFolder ? 0 : 1)
            .ThenBy(c => string.IsNullOrEmpty(c.Title) ? c.Url : c.Title, StringComparer.CurrentCultureIgnoreCase)
            .ToList();

        for (var i = 0; i < desired.Count; i++)
        {
            if (session.ChildIds(folder.Id)[i] == desired[i].Id) continue;
            await session.MoveAsync(desired[i].Id, folder.Id, i);
        }
    }

    /// <summary>選んだ項目を同じフォルダ内で 1 つ上 (-1) または下 (+1) へ動かす。端に詰まった項目は動かさない。</summary>
    public static async Task NudgeAsync(EditSession session, IEnumerable<BookmarkNode> nodes, int direction)
    {
        if (direction is not (-1 or 1)) throw new ArgumentOutOfRangeException(nameof(direction));

        foreach (var group in nodes.GroupBy(n => session.ParentIdOf(n.Id)))
        {
            var folderId = group.Key;
            var selected = group.Select(n => n.Id).ToList();
            var ordered = selected.OrderBy(id => direction < 0 ? IndexOf(id) : -IndexOf(id)).ToList();
            var stuck = new HashSet<string>();

            foreach (var id in ordered)
            {
                var neighbor = IndexOf(id) + direction;
                if (neighbor < 0 || neighbor >= session.ChildIds(folderId).Count || stuck.Contains(session.ChildIds(folderId)[neighbor]))
                {
                    stuck.Add(id);
                    continue;
                }
                // 下へ動かすときの挿入位置は移動前の並びで数えるため +1
                await session.MoveAsync(id, folderId, direction < 0 ? neighbor : neighbor + 1);
            }

            int IndexOf(string id)
            {
                var list = session.ChildIds(folderId);
                for (var i = 0; i < list.Count; i++) if (list[i] == id) return i;
                return -1;
            }
        }
    }

    public static async Task MoveToFolderAsync(EditSession session, IEnumerable<BookmarkNode> nodes, string folderId)
    {
        foreach (var n in nodes)
            if (session.ParentIdOf(n.Id) != folderId) await session.MoveAsync(n.Id, folderId, null);
    }

    public static async Task RemoveAsync(EditSession session, IEnumerable<BookmarkNode> nodes)
    {
        foreach (var n in nodes) await session.RemoveAsync(n);
    }

    public static async Task ApplyReplaceAsync(EditSession session, IEnumerable<ReplaceChange> changes)
    {
        foreach (var c in changes) await session.UpdateAsync(c.Node, c.NewTitle, c.NewUrl);
    }
}
