using System.IO;
using System.Collections.ObjectModel;
using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Diagnostics;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Settings.Bookmarks.Services;

namespace Waypoint.Settings.Bookmarks.ViewModels;

/// <summary>メイン画面の状態と操作。WPF の型には依存しない (UI スレッドへの受け渡しは <see cref="IUiDispatcher"/> 経由)。</summary>
internal sealed partial class BookmarkEditorViewModel : ViewModelBase
{
    private const string AppTitle = "Edit Bookmarks";
    private const int MaxUndoEntries = 30;
    private const int MaxOpenUrls = 10;
    private static readonly TimeSpan _snapshotWait = TimeSpan.FromSeconds(3);

    private readonly IBookmarkConnection _connection;
    private readonly IDialogService _dialogs;
    private readonly IBackupService _backups;
    private readonly IEdgeLauncher _launcher;
    private readonly IUiDispatcher _ui;
    private readonly ILog _log;

    private readonly HashSet<string> _expandedFolderIds = new();
    private readonly List<UndoEntry> _undoStack = new();
    private readonly List<CommandBase> _commands = new();

    private BookmarkTree _tree = BookmarkTree.Empty;
    private bool _hasReceivedSnapshot;
    private bool _isConnected;
    private bool _isBusy;
    private bool _isRebuilding;
    private bool _selectionUpdatePending;
    private int _selectedRowCount;
    private string? _scopeFolderId;
    private FolderViewModel? _selectedFolder;
    private string _searchText = "";
    private bool _duplicatesOnly;
    private bool _includeSubFolders = true;
    private string _statusText = "";

    public BookmarkEditorViewModel(IBookmarkConnection connection, IDialogService dialogs, IBackupService backups,
        IEdgeLauncher launcher, IUiDispatcher ui, ILog log)
    {
        _connection = connection;
        _dialogs = dialogs;
        _backups = backups;
        _launcher = launcher;
        _ui = ui;
        _log = log;

        LaunchEdgeCommand = Add(new RelayCommand(LaunchEdge, () => !IsConnected));
        RefreshCommand = AddAsync(() => RunExclusiveAsync(connection.RequestSnapshotAsync), () => CanEdit);
        UndoCommand = AddAsync(UndoAsync, () => CanEdit && _undoStack.Count > 0);
        NewFolderCommand = AddAsync(NewFolderAsync, () => CanEdit);
        RenameFolderCommand = AddAsync(RenameFolderAsync, () => CanEdit && SelectedFolder?.Node is { IsRoot: false });
        DeleteFolderCommand = AddAsync(DeleteFolderAsync, () => CanEdit && SelectedFolder?.Node is { IsRoot: false });
        SortFolderCommand = AddAsync(SortFolderAsync, () => CanEdit && SelectedFolder?.Node is not null);
        MoveRowsCommand = AddAsync(MoveSelectedRowsAsync, () => CanEdit && _selectedRowCount > 0);
        ReplaceCommand = AddAsync(ReplaceAsync, () => CanEdit && Rows.Count > 0);
        DeleteRowsCommand = AddAsync(DeleteSelectedRowsAsync, () => CanEdit && _selectedRowCount > 0);
        MoveUpCommand = AddAsync(() => NudgeAsync(-1), () => CanEdit && _selectedRowCount > 0);
        MoveDownCommand = AddAsync(() => NudgeAsync(1), () => CanEdit && _selectedRowCount > 0);
        RemoveDuplicatesCommand = AddAsync(RemoveDuplicatesAsync, () => CanEdit);
        OpenUrlsCommand = Add(new RelayCommand(OpenSelectedUrls, () => _selectedRowCount > 0));
        OpenBackupFolderCommand = Add(new RelayCommand(() => dialogs.OpenFolder(backups.Directory)));
        ClearFilterCommand = Add(new RelayCommand(ClearFilter));

        connection.ConnectionChanged += connected => ui.Post(() => OnConnectionChanged(connected));
        connection.SnapshotReceived += snapshot => ui.Post(() => OnSnapshot(snapshot));
        UpdateStatus();
    }

    // ---- 画面に出す状態 ----

    public ObservableCollection<FolderViewModel> FolderRoots { get; } = new();
    public ObservableCollection<RowViewModel> Rows { get; } = new();

    public bool IsConnected
    {
        get => _isConnected;
        private set
        {
            if (!SetProperty(ref _isConnected, value)) return;
            Raise(nameof(ConnectionText));
            OnEditabilityChanged();
        }
    }

    public string ConnectionText => IsConnected ? "● Connected to Edge" : "○ Not connected";

    private bool IsBusy
    {
        get => _isBusy;
        set
        {
            if (!SetProperty(ref _isBusy, value)) return;
            OnEditabilityChanged();
        }
    }

    private bool CanEdit => IsConnected && !IsBusy;

    /// <summary>一覧のセル編集を止めるとき true。</summary>
    public bool IsReadOnly => !CanEdit;

    public string StatusText
    {
        get => _statusText;
        private set => SetProperty(ref _statusText, value);
    }

    public string UndoLabel => _undoStack.Count > 0 ? $"Undo ({_undoStack[^1].Label})" : "Undo";

    public FolderViewModel? SelectedFolder
    {
        get => _selectedFolder;
        private set
        {
            if (!SetProperty(ref _selectedFolder, value)) return;
            RefreshCommands();
        }
    }

    public string SearchText
    {
        get => _searchText;
        set
        {
            if (SetProperty(ref _searchText, value)) RebuildRows();
        }
    }

    public bool DuplicatesOnly
    {
        get => _duplicatesOnly;
        set
        {
            if (SetProperty(ref _duplicatesOnly, value)) RebuildRows();
        }
    }

    public bool IncludeSubFolders
    {
        get => _includeSubFolders;
        set
        {
            if (SetProperty(ref _includeSubFolders, value)) RebuildRows();
        }
    }

    // ---- コマンド ----

    public CommandBase LaunchEdgeCommand { get; }
    public CommandBase RefreshCommand { get; }
    public CommandBase UndoCommand { get; }
    public CommandBase NewFolderCommand { get; }
    public CommandBase RenameFolderCommand { get; }
    public CommandBase DeleteFolderCommand { get; }
    public CommandBase SortFolderCommand { get; }
    public CommandBase MoveRowsCommand { get; }
    public CommandBase ReplaceCommand { get; }
    public CommandBase DeleteRowsCommand { get; }
    public CommandBase MoveUpCommand { get; }
    public CommandBase MoveDownCommand { get; }
    public CommandBase RemoveDuplicatesCommand { get; }
    public CommandBase OpenUrlsCommand { get; }
    public CommandBase OpenBackupFolderCommand { get; }
    public CommandBase ClearFilterCommand { get; }

    private T Add<T>(T command) where T : CommandBase
    {
        _commands.Add(command);
        return command;
    }

    private AsyncRelayCommand AddAsync(Func<Task> execute, Func<bool> canExecute) =>
        Add(new AsyncRelayCommand(execute, canExecute, e =>
        {
            _log.Error(e.ToString());
            _dialogs.ShowWarning(AppTitle, e.Message);
        }));

    private void RefreshCommands()
    {
        foreach (var c in _commands) c.NotifyCanExecuteChanged();
        Raise(nameof(UndoLabel));
    }

    private void OnEditabilityChanged()
    {
        Raise(nameof(IsReadOnly));
        RefreshCommands();
    }

    // ---- Edge からの通知 ----

    private void OnConnectionChanged(bool connected)
    {
        IsConnected = connected;
        UpdateStatus();
    }

    private void OnSnapshot(BookmarkSnapshot snapshot)
    {
        _tree = snapshot.Tree;
        if (!_hasReceivedSnapshot)
        {
            _hasReceivedSnapshot = true;
            _backups.Save(snapshot.RawJson);
            foreach (var root in _tree.Roots) _expandedFolderIds.Add(root.Id);
        }
        RebuildFolders();
    }

    // ---- 表示の組み立て ----

    private void RebuildFolders()
    {
        foreach (var f in FolderRoots.SelectMany(r => r.SelfAndDescendants()))
        {
            if (f.Id is null) continue;
            if (f.IsExpanded) _expandedFolderIds.Add(f.Id); else _expandedFolderIds.Remove(f.Id);
        }

        _isRebuilding = true;
        FolderRoots.Clear();
        var all = new FolderViewModel(null, "All bookmarks", _tree.Bookmarks.Count(), null, OnFolderSelected) { IsExpanded = true };
        foreach (var root in _tree.Roots) all.Children.Add(BuildFolder(root, all));
        FolderRoots.Add(all);

        var selected = all.SelfAndDescendants().FirstOrDefault(f => f.Id == _scopeFolderId) ?? all;
        _scopeFolderId = selected.Id;
        SelectedFolder = selected;
        selected.IsSelected = true;
        for (var p = selected.Parent; p is not null; p = p.Parent) p.IsExpanded = true;
        _isRebuilding = false;

        RebuildRows();
    }

    private FolderViewModel BuildFolder(BookmarkNode node, FolderViewModel parent)
    {
        var folder = new FolderViewModel(node, node.Title, node.BookmarkCount, parent, OnFolderSelected)
        {
            IsExpanded = _expandedFolderIds.Contains(node.Id),
        };
        foreach (var child in node.Children.Where(c => c.IsFolder)) folder.Children.Add(BuildFolder(child, folder));
        return folder;
    }

    private void OnFolderSelected(FolderViewModel folder)
    {
        if (_isRebuilding) return;
        _scopeFolderId = folder.Id;
        SelectedFolder = folder;
        RebuildRows();
    }

    private void RebuildRows()
    {
        var keep = Rows.Where(r => r.IsSelected).Select(r => r.Node.Id).ToHashSet();
        var scope = _scopeFolderId is null ? null : _tree.Find(_scopeFolderId);
        IEnumerable<BookmarkNode> source = scope is null ? _tree.Flatten()
            : _includeSubFolders ? scope.Descendants()
            : scope.Children;

        var urlCounts = DuplicateFinder.CountByUrl(_tree);
        var words = _searchText.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);

        _isRebuilding = true;
        Rows.Clear();
        foreach (var node in source.Where(n => !n.IsFolder))
        {
            var isDuplicate = !string.IsNullOrWhiteSpace(node.Url) && urlCounts.GetValueOrDefault(DuplicateFinder.KeyOf(node.Url)) > 1;
            if (_duplicatesOnly && !isDuplicate) continue;
            if (words.Length > 0 && !Matches(node, words)) continue;
            Rows.Add(new RowViewModel(node, isDuplicate, OnRowSelectionChanged) { IsSelected = keep.Contains(node.Id) });
        }
        _isRebuilding = false;

        _selectedRowCount = Rows.Count(r => r.IsSelected);
        UpdateStatus();
        RefreshCommands();
    }

    private static bool Matches(BookmarkNode node, string[] words)
    {
        var haystack = $"{node.Title}\n{node.Url}\n{node.FolderPath}";
        return words.All(w => haystack.Contains(w, StringComparison.CurrentCultureIgnoreCase));
    }

    /// <summary>複数行の選択が一度に変わると何度も呼ばれるため、画面側の更新は 1 回にまとめる。</summary>
    private void OnRowSelectionChanged()
    {
        if (_isRebuilding || _selectionUpdatePending) return;
        _selectionUpdatePending = true;
        _ui.Post(() =>
        {
            _selectionUpdatePending = false;
            _selectedRowCount = Rows.Count(r => r.IsSelected);
            UpdateStatus();
            RefreshCommands();
        });
    }

    private void UpdateStatus()
    {
        if (!_hasReceivedSnapshot)
        {
            StatusText = IsConnected ? "Loading bookmarks…" : "Open Edge with the Waypoint Tab Bridge extension enabled.";
            return;
        }
        var filtering = _searchText.Trim().Length > 0 || _duplicatesOnly;
        StatusText = $"Showing {Rows.Count} of {_tree.Bookmarks.Count()} · {_selectedRowCount} selected"
                     + (filtering ? " (filtered)" : "")
                     + (IsConnected ? "" : " · Disconnected from Edge");
    }

    private void ClearFilter()
    {
        SearchText = "";
        DuplicatesOnly = false;
    }

    // ---- 実行の枠組み ----

    private List<RowViewModel> SelectedRows() => Rows.Where(r => r.IsSelected).ToList();

    /// <summary>同時に 1 つの操作だけ実行する。失敗は通知し、書き込みがあれば最新のスナップショットが届くまで待つ。</summary>
    private async Task RunExclusiveAsync(Func<Task> action)
    {
        if (IsBusy) return;
        IsBusy = true;
        var commandsBefore = _connection.CommandCount;
        var nextSnapshot = _connection.WhenNextSnapshot();
        try
        {
            await action();
        }
        catch (Exception e)
        {
            _log.Error(e.ToString());
            _dialogs.ShowWarning(AppTitle, e.Message);
        }
        finally
        {
            // 次の操作が古い並びを見ないよう、Edge から最新のスナップショットが届くまで待つ
            if (_connection.CommandCount != commandsBefore)
            {
                try { await nextSnapshot.WaitAsync(_snapshotWait); }
                catch (TimeoutException) { }
            }
            IsBusy = false;
        }
    }

    /// <summary>お気に入りを書き換える操作。成功した分は、元に戻せるよう記録する。</summary>
    private Task EditAsync(string label, Func<EditSession, Task> body, bool backupFirst = false) => RunExclusiveAsync(async () =>
    {
        if (backupFirst && _connection.LatestSnapshot is { } snapshot) _backups.Save(snapshot.RawJson);

        var session = new EditSession(_connection, _tree);
        try
        {
            await body(session);
        }
        finally
        {
            if (session.ToUndoEntry(label) is { } entry)
            {
                _undoStack.Add(entry);
                if (_undoStack.Count > MaxUndoEntries) _undoStack.RemoveAt(0);
            }
        }
    });

    private Task UndoAsync()
    {
        if (_undoStack.Count == 0) return Task.CompletedTask;
        var entry = _undoStack[^1];
        _undoStack.RemoveAt(_undoStack.Count - 1);
        return RunExclusiveAsync(async () =>
        {
            await entry.ApplyAsync();
            _log.Info($"undo: {entry.Label}");
        });
    }

    // ---- Edge の起動 ----

    private void LaunchEdge()
    {
        try
        {
            switch (_launcher.Launch())
            {
                case EdgeLaunchResult.EdgeAlreadyRunning:
                    _dialogs.ShowInfo("Launch Edge",
                        "Edge is already running, so the extension cannot be loaded with it." + Environment.NewLine +
                        "Close every Edge window and try again." + Environment.NewLine +
                        "(If the extension is already loaded in Edge, you do not need this.)");
                    break;
                case EdgeLaunchResult.ExtensionNotFound:
                    _dialogs.ShowWarning(AppTitle, "The browser extension folder was not found.");
                    break;
            }
        }
        catch (FileNotFoundException e)
        {
            _dialogs.ShowWarning(AppTitle, e.Message);
        }
    }
}
