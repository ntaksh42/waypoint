using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Settings.Bookmarks.Services;

namespace Waypoint.Bookmarks.Tests.Fakes;

/// <summary>ダイアログの答えを、テストが先に決めておく。</summary>
internal sealed class FakeDialogs : IDialogService
{
    public bool ConfirmAnswer { get; set; } = true;
    public string? AskAnswer { get; set; }
    public Func<BookmarkTree, BookmarkNode?>? PickFolderAnswer { get; set; }
    public ReplaceSpec? ReplaceAnswer { get; set; }

    public List<string> Infos { get; } = new();
    public List<string> Warnings { get; } = new();
    public List<string> OpenedUrls { get; } = new();
    public int ConfirmCount { get; private set; }

    public bool Confirm(string title, string message)
    {
        ConfirmCount++;
        return ConfirmAnswer;
    }

    public void ShowInfo(string title, string message) => Infos.Add(message);
    public void ShowWarning(string title, string message) => Warnings.Add(message);
    public string? Ask(string title, string label, string initial) => AskAnswer;
    public BookmarkNode? PickFolder(string title, BookmarkTree tree) => PickFolderAnswer?.Invoke(tree);
    public ReplaceSpec? AskReplace(IReadOnlyList<BookmarkNode> targets, string scopeLabel) => ReplaceAnswer;
    public void OpenUrl(string url) => OpenedUrls.Add(url);
    public void OpenFolder(string path) { }
}

internal sealed class FakeBackupService : IBackupService
{
    public string Directory => "backups";
    public List<string> Saved { get; } = new();
    public void Save(string json) => Saved.Add(json);
}

internal sealed class FakeEdgeLauncher : IEdgeLauncher
{
    public EdgeLaunchResult Result { get; set; } = EdgeLaunchResult.Launched;
    public int LaunchCount { get; private set; }

    public EdgeLaunchResult Launch()
    {
        LaunchCount++;
        return Result;
    }
}

/// <summary>UI スレッドの代わりに、その場で実行する。</summary>
internal sealed class ImmediateDispatcher : IUiDispatcher
{
    public void Post(Action action) => action();
}
