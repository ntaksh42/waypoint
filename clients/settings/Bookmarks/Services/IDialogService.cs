using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;

namespace Waypoint.Settings.Bookmarks.Services;

/// <summary>ユーザーへの確認・入力・通知。ViewModel から画面部品を切り離すための窓口。</summary>
internal interface IDialogService
{
    public bool Confirm(string title, string message);
    public void ShowInfo(string title, string message);
    public void ShowWarning(string title, string message);

    /// <returns>入力された文字列。キャンセルまたは空なら null。</returns>
    public string? Ask(string title, string label, string initial);

    /// <returns>選ばれたフォルダ。キャンセルなら null。</returns>
    public BookmarkNode? PickFolder(string title, BookmarkTree tree);

    /// <returns>置換の指定。キャンセルなら null。</returns>
    public ReplaceSpec? AskReplace(IReadOnlyList<BookmarkNode> targets, string scopeLabel);

    public void OpenUrl(string url);
    public void OpenFolder(string path);
}
