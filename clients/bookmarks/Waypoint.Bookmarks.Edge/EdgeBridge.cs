using System.IO.Pipes;
using System.Text.Json;
using System.Text.Json.Serialization;
using Waypoint.Bookmarks.Core.Diagnostics;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Bookmarks.Edge.Protocol;

namespace Waypoint.Bookmarks.Edge;

/// <summary>
/// Edge 拡張機能 (Native Messaging ホスト経由) との通信。
/// パイプのサーバーとして待ち受け、書き込みを依頼し、お気に入りのスナップショットを受け取る。
/// </summary>
public sealed class EdgeBridge : IBookmarkConnection
{
    private static readonly JsonSerializerOptions _jsonOptions = new() { DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull };
    private static readonly TimeSpan _commandTimeout = TimeSpan.FromSeconds(10);

    private readonly ILog _log;
    private readonly Dictionary<int, TaskCompletionSource<JsonElement>> _pending = new();
    private readonly List<TaskCompletionSource> _snapshotWaiters = new();
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private NamedPipeServerStream? _current;
    private int _nextRequestId;
    private int _commandCount;

    public EdgeBridge(ILog log) => _log = log;

    public bool IsConnected => _current is not null;
    public BookmarkSnapshot? LatestSnapshot { get; private set; }
    public int CommandCount => _commandCount;

    public event Action<bool>? ConnectionChanged;
    public event Action<BookmarkSnapshot>? SnapshotReceived;

    /// <summary>ホストの接続を待ち続ける。ホストが切れたら、次の接続を待つ。</summary>
    public async Task RunAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            var pipe = new NamedPipeServerStream(ProtocolConstants.PipeName, PipeDirection.InOut, 1,
                PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
            try { await pipe.WaitForConnectionAsync(ct); }
            catch (OperationCanceledException) { pipe.Dispose(); break; }

            _current = pipe;
            _log.Info("host connected");
            ConnectionChanged?.Invoke(true);
            try
            {
                await SendAsync(pipe, new { type = "requestSnapshot" });
                while (await Framing.ReadFrameAsync(pipe, ct) is { } frame)
                    Handle(JsonDocument.Parse(frame).RootElement.Clone());
            }
            catch (Exception e) when (e is IOException or OperationCanceledException or ObjectDisposedException) { }

            _current = null;
            FailPending(new IOException("Edge との接続が切れました"));
            _log.Info("host disconnected");
            ConnectionChanged?.Invoke(false);
            pipe.Dispose();
        }
    }

    public Task RequestSnapshotAsync() =>
        _current is { } pipe ? SendAsync(pipe, new { type = "requestSnapshot" }) : Task.CompletedTask;

    public Task WhenNextSnapshot()
    {
        var waiter = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        lock (_snapshotWaiters) _snapshotWaiters.Add(waiter);
        return waiter.Task;
    }

    // ---- IBookmarkGateway ----

    public Task UpdateAsync(string id, string title, string? url) =>
        CommandAsync("update", new { id, title, url });

    public Task MoveAsync(string id, string parentId, int? index) =>
        CommandAsync("move", new { id, parentId, index });

    public Task RemoveAsync(string id, bool recursive) =>
        CommandAsync("remove", new { id, tree = recursive });

    public async Task<string> CreateAsync(string parentId, string title, string? url, int? index)
    {
        var result = await CommandAsync("create", new { parentId, title, url, index });
        return result.GetProperty("id").GetString()!;
    }

    // ---- 内部 ----

    private async Task<JsonElement> CommandAsync(string op, object args)
    {
        var pipe = _current ?? throw new InvalidOperationException("Edge に接続していません");
        var requestId = Interlocked.Increment(ref _nextRequestId);
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        lock (_pending) _pending[requestId] = completion;
        try
        {
            await SendAsync(pipe, new { type = "command", reqId = requestId, op, args });
            Interlocked.Increment(ref _commandCount);
            return await completion.Task.WaitAsync(_commandTimeout);
        }
        catch (TimeoutException)
        {
            throw new TimeoutException("Edge から応答がありません");
        }
        finally
        {
            lock (_pending) _pending.Remove(requestId);
        }
    }

    private void Handle(JsonElement message)
    {
        switch (message.GetProperty("type").GetString())
        {
            case "snapshot":
                var roots = message.GetProperty("tree");
                var snapshot = new BookmarkSnapshot(SnapshotParser.Parse(roots), roots.GetRawText());
                LatestSnapshot = snapshot;
                _log.Info($"snapshot: roots={snapshot.Tree.Roots.Count}");
                NotifySnapshotWaiters();
                SnapshotReceived?.Invoke(snapshot);
                break;

            case "result":
                var requestId = message.GetProperty("reqId").GetInt32();
                TaskCompletionSource<JsonElement>? completion;
                lock (_pending) _pending.Remove(requestId, out completion);
                if (completion is null) break;
                if (message.GetProperty("ok").GetBoolean()) completion.TrySetResult(message);
                else completion.TrySetException(new InvalidOperationException(
                    message.TryGetProperty("error", out var error) ? error.GetString() : "失敗しました"));
                break;
        }
    }

    private void NotifySnapshotWaiters()
    {
        List<TaskCompletionSource> waiters;
        lock (_snapshotWaiters)
        {
            waiters = _snapshotWaiters.ToList();
            _snapshotWaiters.Clear();
        }
        foreach (var w in waiters) w.TrySetResult();
    }

    private void FailPending(Exception error)
    {
        lock (_pending)
        {
            foreach (var p in _pending.Values) p.TrySetException(error);
            _pending.Clear();
        }
    }

    private async Task SendAsync(Stream stream, object message)
    {
        await _writeLock.WaitAsync();
        try { await Framing.WriteFrameAsync(stream, JsonSerializer.SerializeToUtf8Bytes(message, _jsonOptions)); }
        finally { _writeLock.Release(); }
    }
}
