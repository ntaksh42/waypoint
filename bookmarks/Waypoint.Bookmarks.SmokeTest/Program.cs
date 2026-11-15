// 実際の Edge に対する結合試験。使い捨てのプロファイルで Edge を起動した状態で実行する (bookmarks/smoke-test.ps1)。
// お気に入りに「__smoke__」フォルダを作り、操作して、消す。普段のプロファイルでは実行しないこと。
using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Diagnostics;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Bookmarks.Edge;

var failures = 0;
void Check(string name, bool ok, string detail = "")
{
    if (!ok) failures++;
    Console.WriteLine($"{(ok ? "PASS" : "FAIL")}: {name} {detail}");
}

using var cts = new CancellationTokenSource();
var bridge = new EdgeBridge(new ConsoleLog());
_ = bridge.RunAsync(cts.Token);

try
{
    var deadline = DateTime.UtcNow.AddSeconds(60);
    while (bridge.LatestSnapshot is null && DateTime.UtcNow < deadline) await Task.Delay(100);
    Check("snapshot received", bridge.LatestSnapshot is not null);
    if (bridge.LatestSnapshot is null) return 1;

    // 試験は「__smoke__」フォルダの作成と削除を行う。お気に入りが入っているプロファイルには書き込まない
    if (bridge.LatestSnapshot.Tree.Bookmarks.Any())
    {
        Console.WriteLine("ABORT: お気に入りが入っているプロファイルにつながったため、試験を中止しました。");
        return 2;
    }

    BookmarkTree Tree() => bridge.LatestSnapshot!.Tree;

    // 操作して、Edge が最新のスナップショットを返すまで待つ
    async Task<BookmarkTree> Run(Func<EditSession, Task> body, List<UndoEntry>? undo = null, string label = "")
    {
        var next = bridge.WhenNextSnapshot();
        var before = bridge.CommandCount;
        var session = new EditSession(bridge, Tree());
        await body(session);
        if (undo is not null && session.ToUndoEntry(label) is { } entry) undo.Add(entry);
        if (bridge.CommandCount != before) await next.WaitAsync(TimeSpan.FromSeconds(10));
        return Tree();
    }
    async Task<BookmarkTree> Undo(UndoEntry entry)
    {
        var next = bridge.WhenNextSnapshot();
        await entry.ApplyAsync();
        await next.WaitAsync(TimeSpan.FromSeconds(10));
        return Tree();
    }
    string Order(BookmarkTree tree, string folderId) => string.Join(",", tree.Find(folderId)!.Children.Select(c => c.Title));

    var other = Tree().Roots[1].Id;
    string folder = "", dest = "";
    string idC = "", idA = "", idB = "";

    var tree = await Run(async s =>
    {
        folder = await s.CreateFolderAsync(other, "__smoke__");
        dest = await s.CreateFolderAsync(other, "__smoke_dest__");
    });
    Check("create folders", tree.Find(folder) is { IsFolder: true } && tree.Find(dest) is { IsFolder: true });

    tree = await Run(async _ =>
    {
        idC = await bridge.CreateAsync(folder, "C-site", "https://example.com/c", null);
        idA = await bridge.CreateAsync(folder, "A-site", "https://example.com/a", null);
        idB = await bridge.CreateAsync(folder, "B-site", "https://example.com/b", null);
    });
    Check("create bookmarks", Order(tree, folder) == "C-site,A-site,B-site", Order(tree, folder));

    tree = await Run(s => s.MoveAsync(idC, folder, 3));
    Check("move to end (insert-before index)", Order(tree, folder) == "A-site,B-site,C-site", Order(tree, folder));

    var undo = new List<UndoEntry>();
    tree = await Run(s => s.MoveAsync(idB, folder, 0), undo, "move");
    Check("move to front", Order(tree, folder) == "B-site,A-site,C-site", Order(tree, folder));
    tree = await Run(s => BulkOperations.SortFolderAsync(s, Tree().Find(folder)!), undo, "sort");
    Check("sort", Order(tree, folder) == "A-site,B-site,C-site", Order(tree, folder));
    foreach (var entry in Enumerable.Reverse(undo)) tree = await Undo(entry);
    Check("undo sort and move", Order(tree, folder) == "A-site,B-site,C-site", Order(tree, folder));

    undo.Clear();
    tree = await Run(s => BulkOperations.NudgeAsync(s, new[] { Tree().Find(idB)! }, -1), undo, "nudge");
    Check("nudge up", Order(tree, folder) == "B-site,A-site,C-site", Order(tree, folder));
    tree = await Undo(undo[0]);
    Check("undo nudge", Order(tree, folder) == "A-site,B-site,C-site", Order(tree, folder));

    undo.Clear();
    tree = await Run(s => BulkOperations.MoveToFolderAsync(s, new[] { Tree().Find(idC)!, Tree().Find(idB)! }, dest), undo, "move");
    Check("bulk move", Order(tree, folder) == "A-site" && Order(tree, dest) == "C-site,B-site", $"{Order(tree, folder)} | {Order(tree, dest)}");
    tree = await Undo(undo[0]);
    Check("undo bulk move", Order(tree, folder) == "A-site,B-site,C-site" && Order(tree, dest) == "", $"{Order(tree, folder)} | {Order(tree, dest)}");

    undo.Clear();
    var titleSpec = new ReplaceSpec("-site", "", true, false, false, true);
    tree = await Run(s => BulkOperations.ApplyReplaceAsync(s, titleSpec.Plan(Tree().Find(folder)!.Children)), undo, "replace");
    Check("bulk replace title", Order(tree, folder) == "A,B,C", Order(tree, folder));
    tree = await Undo(undo[0]);
    Check("undo replace", Order(tree, folder) == "A-site,B-site,C-site", Order(tree, folder));

    var urlSpec = new ReplaceSpec("example.com", "example.org", false, true, false, true);
    tree = await Run(s => BulkOperations.ApplyReplaceAsync(s, urlSpec.Plan(Tree().Find(folder)!.Children)));
    Check("bulk replace url", tree.Find(folder)!.Children.All(n => n.Url!.Contains("example.org")),
        string.Join(",", tree.Find(folder)!.Children.Select(n => n.Url)));

    undo.Clear();
    tree = await Run(s => s.RemoveAsync(Tree().Find(folder)!), undo, "remove");
    Check("remove folder", tree.Find(folder) is null);
    tree = await Undo(undo[0]);
    var restored = tree.Roots[1].Children.FirstOrDefault(n => n.Title == "__smoke__");
    Check("undo remove folder (recreated with children)", restored?.Children.Count == 3,
        restored is null ? "missing" : string.Join(",", restored.Children.Select(n => n.Title)));

    tree = await Run(async _ => await bridge.CreateAsync(restored!.Id, "dup", "https://EXAMPLE.org/a/", null));
    var extras = DuplicateFinder.FindExtras(tree).Where(n => n.ParentId == restored!.Id).ToList();
    Check("duplicate detection", extras.Count == 1 && extras[0].Title == "dup", $"count={extras.Count}");

    tree = await Run(async s =>
    {
        await s.RemoveAsync(Tree().Find(restored!.Id)!);
        await s.RemoveAsync(Tree().Find(dest)!);
    });
    Check("cleanup", tree.Roots[1].Children.All(n => !n.Title.StartsWith("__smoke")));
}
catch (Exception e)
{
    failures++;
    Console.WriteLine("EXCEPTION: " + e);
}
finally
{
    cts.Cancel();
}

Console.WriteLine(failures == 0 ? "ALL PASS" : $"FAILED: {failures}");
return failures == 0 ? 0 : 1;

internal sealed class ConsoleLog : ILog
{
    public void Info(string message) { }
    public void Error(string message) => Console.WriteLine("ERROR: " + message);
}
