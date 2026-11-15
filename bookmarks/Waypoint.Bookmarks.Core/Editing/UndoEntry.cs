namespace Waypoint.Bookmarks.Core.Editing;

/// <summary>1 回の操作を元に戻すための逆操作の並び。</summary>
/// <param name="Inverses">実行した順。戻すときは後ろから実行する。</param>
public sealed record UndoEntry(string Label, IReadOnlyList<Func<Task>> Inverses)
{
    public async Task ApplyAsync()
    {
        for (var i = Inverses.Count - 1; i >= 0; i--) await Inverses[i]();
    }
}
