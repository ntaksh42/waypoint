using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;

namespace Waypoint.Bookmarks.Tests.Fakes;

/// <summary>書き込みのたびに最新のスナップショットを即座に配る、Edge の代役。</summary>
internal sealed class FakeConnection : IBookmarkConnection
{
    private readonly List<TaskCompletionSource> _waiters = new();

    public FakeConnection(FakeBookmarkGateway store) => Store = store;

    public FakeBookmarkGateway Store { get; }
    public bool IsConnected { get; private set; }
    public BookmarkSnapshot? LatestSnapshot { get; private set; }
    public int CommandCount { get; private set; }

    public event Action<bool>? ConnectionChanged;
    public event Action<BookmarkSnapshot>? SnapshotReceived;

    /// <summary>Edge がつながり、最初のスナップショットが届いた状態にする。</summary>
    public void Connect()
    {
        IsConnected = true;
        ConnectionChanged?.Invoke(true);
        Publish();
    }

    public void Disconnect()
    {
        IsConnected = false;
        ConnectionChanged?.Invoke(false);
    }

    public void Publish()
    {
        LatestSnapshot = Store.ToSnapshot();
        var done = _waiters.ToList();
        _waiters.Clear();
        foreach (var w in done) w.TrySetResult();
        SnapshotReceived?.Invoke(LatestSnapshot);
    }

    public Task RequestSnapshotAsync()
    {
        Publish();
        return Task.CompletedTask;
    }

    public Task WhenNextSnapshot()
    {
        var waiter = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        _waiters.Add(waiter);
        return waiter.Task;
    }

    public async Task UpdateAsync(string id, string title, string? url) => await Write(() => Store.UpdateAsync(id, title, url));

    public async Task MoveAsync(string id, string parentId, int? index) => await Write(() => Store.MoveAsync(id, parentId, index));

    public async Task RemoveAsync(string id, bool recursive) => await Write(() => Store.RemoveAsync(id, recursive));

    public async Task<string> CreateAsync(string parentId, string title, string? url, int? index)
    {
        var id = "";
        await Write(async () => id = await Store.CreateAsync(parentId, title, url, index));
        return id;
    }

    private async Task Write(Func<Task> action)
    {
        if (!IsConnected) throw new InvalidOperationException("Edge に接続していません");
        await action();
        CommandCount++;
        Publish();
    }
}
