using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Bookmarks.Core.Editing;

/// <summary>Edge から受け取る、ある時点のお気に入りの全体像。</summary>
/// <param name="RawJson">受け取った JSON そのもの。バックアップに使う。</param>
public sealed record BookmarkSnapshot(BookmarkTree Tree, string RawJson);

/// <summary>ブラウザとの接続。お気に入りの読み取り (スナップショット) と書き込み (<see cref="IBookmarkGateway"/>) を持つ。</summary>
public interface IBookmarkConnection : IBookmarkGateway
{
    public bool IsConnected { get; }

    /// <summary>最後に受け取ったスナップショット。まだ無ければ null。</summary>
    public BookmarkSnapshot? LatestSnapshot { get; }

    /// <summary>これまでに送った書き込みの累計。操作がブラウザに届いたかの判定に使う。</summary>
    public int CommandCount { get; }

    /// <summary>接続の確立 (true) と切断 (false)。呼び出しスレッドは不定。</summary>
    public event Action<bool>? ConnectionChanged;

    /// <summary>スナップショットを受け取った。呼び出しスレッドは不定。</summary>
    public event Action<BookmarkSnapshot>? SnapshotReceived;

    public Task RequestSnapshotAsync();

    /// <summary>次のスナップショットが届いたら完了する。書き込みを送る前に呼ぶ。</summary>
    public Task WhenNextSnapshot();
}
