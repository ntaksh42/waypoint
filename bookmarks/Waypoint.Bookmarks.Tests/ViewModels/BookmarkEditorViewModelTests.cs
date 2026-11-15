using Waypoint.Bookmarks.Core.Diagnostics;
using Waypoint.Bookmarks.Core.Editing;
using Waypoint.Settings.Bookmarks.Services;
using Waypoint.Bookmarks.Tests.Fakes;
using Waypoint.Settings.Bookmarks.ViewModels;

namespace Waypoint.Bookmarks.Tests.ViewModels;

public class BookmarkEditorViewModelTests
{
    private readonly FakeBookmarkGateway _store = new();
    private readonly FakeConnection _connection;
    private readonly FakeDialogs _dialogs = new();
    private readonly FakeBackupService _backups = new();
    private readonly FakeEdgeLauncher _launcher = new();
    private readonly BookmarkEditorViewModel _vm;

    private string _work = "";
    private string _home = "";
    private string _google = "";
    private string _docs = "";
    private string _mail = "";

    public BookmarkEditorViewModelTests()
    {
        _work = _store.AddFolder(FakeBookmarkGateway.BarId, "仕事");
        _home = _store.AddFolder(FakeBookmarkGateway.OtherId, "趣味");
        _google = _store.AddBookmark(_work, "Google", "https://www.google.com/");
        _docs = _store.AddBookmark(_work, "Docs", "https://docs.example.com/");
        _mail = _store.AddBookmark(_home, "Mail", "https://mail.example.com/");
        _store.AddBookmark(_home, "Google 2", "https://WWW.google.com");

        _connection = new FakeConnection(_store);
        _vm = new BookmarkEditorViewModel(_connection, _dialogs, _backups, _launcher, new ImmediateDispatcher(), NullLog.Instance);
        _connection.Connect();
    }

    private FolderViewModel Folder(string id) => _vm.FolderRoots.SelectMany(r => r.SelfAndDescendants()).First(f => f.Id == id);

    private static void Run(CommandBase command)
    {
        Assert.True(command.CanExecute(null), "コマンドが実行できない状態です");
        command.Execute(null);
    }

    private void Select(params string[] titles)
    {
        foreach (var r in _vm.Rows) r.IsSelected = titles.Contains(r.Title);
    }

    private string[] Titles() => _vm.Rows.Select(r => r.Title).ToArray();

    // ---- 表示 ----

    [Fact]
    public void The_first_snapshot_builds_folders_and_lists_every_bookmark_and_saves_a_backup()
    {
        Assert.True(_vm.IsConnected);
        Assert.Single(_backups.Saved);
        Assert.Equal(4, _vm.Rows.Count);
        Assert.Equal("All bookmarks", _vm.FolderRoots.Single().Title);
        Assert.Same(_vm.FolderRoots.Single(), _vm.SelectedFolder);
    }

    [Fact]
    public void Selecting_a_folder_shows_only_its_bookmarks()
    {
        Folder(_work).IsSelected = true;

        Assert.Equal(new[] { "Google", "Docs" }, Titles());
    }

    [Fact]
    public void Subfolder_inclusion_can_be_turned_off()
    {
        var sub = _store.AddFolder(_work, "sub");
        _store.AddBookmark(sub, "Deep", "https://deep.example.com/");
        _connection.Publish();
        Folder(_work).IsSelected = true;
        Assert.Contains("Deep", Titles());

        _vm.IncludeSubFolders = false;

        Assert.DoesNotContain("Deep", Titles());
    }

    [Fact]
    public void Search_matches_all_words_against_title_url_and_folder()
    {
        _vm.SearchText = "google";
        Assert.Equal(new[] { "Google", "Google 2" }, Titles());

        _vm.SearchText = "google 趣味";
        Assert.Equal(new[] { "Google 2" }, Titles());

        _vm.SearchText = "example.com mail";
        Assert.Equal(new[] { "Mail" }, Titles());
    }

    [Fact]
    public void Duplicates_only_shows_every_copy_and_marks_them()
    {
        _vm.DuplicatesOnly = true;

        Assert.Equal(new[] { "Google", "Google 2" }, Titles());
        Assert.All(_vm.Rows, r => Assert.Equal("Dup", r.DuplicateMark));
    }

    [Fact]
    public void ClearFilter_resets_search_and_duplicates()
    {
        _vm.SearchText = "x";
        _vm.DuplicatesOnly = true;

        Run(_vm.ClearFilterCommand);

        Assert.Equal("", _vm.SearchText);
        Assert.False(_vm.DuplicatesOnly);
        Assert.Equal(4, _vm.Rows.Count);
    }

    [Fact]
    public void Selection_survives_a_new_snapshot()
    {
        Select("Docs");

        _connection.Publish();

        Assert.Equal(new[] { "Docs" }, _vm.Rows.Where(r => r.IsSelected).Select(r => r.Title));
    }

    [Fact]
    public void Status_reports_counts_and_selection()
    {
        Select("Docs", "Mail");

        Assert.Contains("Showing 4 of 4", _vm.StatusText);
        Assert.Contains("2 selected", _vm.StatusText);
    }

    // ---- 接続 ----

    [Fact]
    public void Edit_commands_are_disabled_while_disconnected()
    {
        Select("Docs");
        Assert.True(_vm.DeleteRowsCommand.CanExecute(null));

        _connection.Disconnect();

        Assert.False(_vm.IsConnected);
        Assert.False(_vm.DeleteRowsCommand.CanExecute(null));
        Assert.False(_vm.NewFolderCommand.CanExecute(null));
        Assert.True(_vm.IsReadOnly);
        Assert.True(_vm.LaunchEdgeCommand.CanExecute(null));
    }

    [Fact]
    public void LaunchEdge_is_only_available_while_disconnected_and_reports_a_running_edge()
    {
        Assert.False(_vm.LaunchEdgeCommand.CanExecute(null));
        _connection.Disconnect();
        _launcher.Result = EdgeLaunchResult.EdgeAlreadyRunning;

        Run(_vm.LaunchEdgeCommand);

        Assert.Equal(1, _launcher.LaunchCount);
        Assert.Single(_dialogs.Infos);
    }

    // ---- 編集と元に戻す ----

    [Fact]
    public void Deleting_selected_rows_asks_first_and_can_be_undone()
    {
        Select("Docs", "Mail");

        Run(_vm.DeleteRowsCommand);

        Assert.Equal(1, _dialogs.ConfirmCount);
        Assert.Equal(new[] { "Google", "Google 2" }, Titles());
        Assert.Equal(2, _backups.Saved.Count);

        Run(_vm.UndoCommand);
        Assert.Equal(new[] { "Google", "Docs", "Mail", "Google 2" }, Titles());
    }

    [Fact]
    public void Declining_the_confirmation_deletes_nothing()
    {
        _dialogs.ConfirmAnswer = false;
        Select("Docs");

        Run(_vm.DeleteRowsCommand);

        Assert.Equal(4, _vm.Rows.Count);
        Assert.Empty(_store.Calls);
        Assert.False(_vm.UndoCommand.CanExecute(null));
    }

    [Fact]
    public void Undo_label_names_the_last_operation()
    {
        Assert.Equal("Undo", _vm.UndoLabel);
        Select("Docs");
        Run(_vm.DeleteRowsCommand);

        Assert.Equal("Undo (Delete 1)", _vm.UndoLabel);
    }

    [Fact]
    public async Task Committing_a_cell_edit_updates_the_bookmark()
    {
        var row = _vm.Rows.First(r => r.Title == "Docs");

        await _vm.CommitEditAsync(row, isTitleColumn: true, "Documents");
        await _vm.CommitEditAsync(_vm.Rows.First(r => r.Title == "Documents"), isTitleColumn: false, " https://docs.example.org/ ");

        var node = _store.ToTree().Find(_docs)!;
        Assert.Equal(("Documents", "https://docs.example.org/"), (node.Title, node.Url));
    }

    [Fact]
    public void An_empty_url_is_rejected_but_an_empty_title_is_allowed()
    {
        Assert.False(_vm.TryValidateEdit(isTitleColumn: false, "  "));
        Assert.Single(_dialogs.Infos);
        Assert.True(_vm.TryValidateEdit(isTitleColumn: true, ""));
        Assert.True(_vm.TryValidateEdit(isTitleColumn: false, "https://x.test/"));
    }

    [Fact]
    public void Moving_selected_rows_goes_to_the_chosen_folder()
    {
        Select("Docs", "Google");
        _dialogs.PickFolderAnswer = tree => tree.Find(_home);

        Run(_vm.MoveRowsCommand);

        Assert.Equal(new[] { "Mail", "Google 2", "Google", "Docs" }.OrderBy(x => x), _store.ToTree().Find(_home)!.Children.Select(c => c.Title).OrderBy(x => x));
        Assert.Empty(_store.ToTree().Find(_work)!.Children);
    }

    [Fact]
    public void Cancelling_the_folder_picker_moves_nothing()
    {
        Select("Docs");
        _dialogs.PickFolderAnswer = _ => null;

        Run(_vm.MoveRowsCommand);

        Assert.Empty(_store.Calls);
    }

    [Fact]
    public void Replace_applies_the_planned_changes_after_a_backup()
    {
        _dialogs.ReplaceAnswer = new ReplaceSpec("example.com", "example.net", false, true, false, true);

        Run(_vm.ReplaceCommand);

        Assert.Equal("https://docs.example.net/", _store.ToTree().Find(_docs)!.Url);
        Assert.Equal("https://mail.example.net/", _store.ToTree().Find(_mail)!.Url);
        Assert.Equal(2, _backups.Saved.Count);
    }

    [Fact]
    public void Removing_duplicates_keeps_the_first_copy()
    {
        Run(_vm.RemoveDuplicatesCommand);

        Assert.Equal(1, _dialogs.ConfirmCount);
        Assert.Equal(new[] { "Google", "Docs", "Mail" }, _store.ToTree().Bookmarks.Select(b => b.Title));
    }

    [Fact]
    public void Removing_duplicates_tells_the_user_when_there_are_none()
    {
        Select("Google 2");
        Run(_vm.DeleteRowsCommand);
        _dialogs.Infos.Clear();

        Run(_vm.RemoveDuplicatesCommand);

        Assert.Single(_dialogs.Infos);
    }

    [Fact]
    public void Up_and_down_reorder_the_selected_row_within_its_folder()
    {
        Folder(_work).IsSelected = true;
        Select("Docs");

        Run(_vm.MoveUpCommand);
        Assert.Equal(new[] { "Docs", "Google" }, _store.ToTree().Find(_work)!.Children.Select(c => c.Title));

        Run(_vm.MoveDownCommand);
        Assert.Equal(new[] { "Google", "Docs" }, _store.ToTree().Find(_work)!.Children.Select(c => c.Title));
    }

    [Fact]
    public void Open_urls_opens_only_web_addresses()
    {
        _store.AddBookmark(_work, "script", "javascript:alert(1)");
        _connection.Publish();
        Folder(_work).IsSelected = true;
        foreach (var r in _vm.Rows) r.IsSelected = true;

        Run(_vm.OpenUrlsCommand);

        Assert.Equal(new[] { "https://www.google.com/", "https://docs.example.com/" }, _dialogs.OpenedUrls);
    }

    // ---- フォルダ ----

    [Fact]
    public void New_folder_is_created_under_the_selected_folder_and_selected()
    {
        Folder(_work).IsSelected = true;
        _dialogs.AskAnswer = "新規";

        Run(_vm.NewFolderCommand);

        var created = _store.ToTree().Find(_work)!.Children.Single(c => c.IsFolder);
        Assert.Equal("新規", created.Title);
        Assert.Equal(created.Id, _vm.SelectedFolder!.Id);
    }

    [Fact]
    public void New_folder_with_a_cancelled_name_does_nothing()
    {
        _dialogs.AskAnswer = null;

        Run(_vm.NewFolderCommand);

        Assert.Empty(_store.Calls);
    }

    [Fact]
    public void Root_folders_cannot_be_renamed_or_deleted_but_others_can()
    {
        Folder(FakeBookmarkGateway.BarId).IsSelected = true;
        Assert.False(_vm.RenameFolderCommand.CanExecute(null));
        Assert.False(_vm.DeleteFolderCommand.CanExecute(null));
        Assert.True(_vm.SortFolderCommand.CanExecute(null));

        Folder(_work).IsSelected = true;
        Assert.True(_vm.RenameFolderCommand.CanExecute(null));
        Assert.True(_vm.DeleteFolderCommand.CanExecute(null));
    }

    [Fact]
    public void Deleting_a_folder_removes_it_with_its_contents_and_undo_brings_it_back()
    {
        Folder(_work).IsSelected = true;

        Run(_vm.DeleteFolderCommand);
        Assert.Null(_store.ToTree().Folders.FirstOrDefault(f => f.Title == "仕事"));

        Run(_vm.UndoCommand);
        var restored = _store.ToTree().Folders.Single(f => f.Title == "仕事");
        Assert.Equal(new[] { "Google", "Docs" }, restored.Children.Select(c => c.Title));
    }

    [Fact]
    public void Rename_folder_uses_the_entered_name()
    {
        Folder(_work).IsSelected = true;
        _dialogs.AskAnswer = "業務";

        Run(_vm.RenameFolderCommand);

        Assert.Equal("業務", _store.ToTree().Find(_work)!.Title);
    }

    // ---- ドラッグ＆ドロップ ----

    [Fact]
    public async Task Dropping_rows_on_a_folder_moves_them()
    {
        await _vm.DropRowsAsync(new[] { _mail }, Folder(_work));

        Assert.Contains("Mail", _store.ToTree().Find(_work)!.Children.Select(c => c.Title));
    }

    [Fact]
    public void A_folder_cannot_be_dropped_on_itself_or_its_descendant()
    {
        var sub = _store.AddFolder(_work, "sub");
        _connection.Publish();

        Assert.False(_vm.CanDropFolder(_work, Folder(_work)));
        Assert.False(_vm.CanDropFolder(_work, Folder(sub)));
        Assert.True(_vm.CanDropFolder(sub, Folder(_home)));
        Assert.False(_vm.CanDropRows(_vm.FolderRoots.Single()));
        Assert.True(_vm.CanDropRows(Folder(_home)));
    }

    [Fact]
    public void Root_folders_cannot_be_dragged()
    {
        Assert.False(_vm.CanDragFolder(Folder(FakeBookmarkGateway.BarId)));
        Assert.True(_vm.CanDragFolder(Folder(_work)));
    }

    // ---- 失敗 ----

    [Fact]
    public void A_failing_write_is_reported_and_leaves_the_view_usable()
    {
        var failing = new BookmarkEditorViewModel(new FailingConnection(_connection, new BrokenGateway()), _dialogs, _backups, _launcher,
            new ImmediateDispatcher(), NullLog.Instance);
        _connection.Disconnect();
        _connection.Connect();
        foreach (var r in failing.Rows) r.IsSelected = r.Title == "Docs";

        failing.DeleteRowsCommand.Execute(null);

        Assert.Single(_dialogs.Warnings);
        Assert.True(failing.DeleteRowsCommand.CanExecute(null));
        Assert.Equal(4, failing.Rows.Count);
    }

    private sealed class BrokenGateway : IBookmarkGateway
    {
        public Task UpdateAsync(string id, string title, string? url) => throw new InvalidOperationException("boom");
        public Task MoveAsync(string id, string parentId, int? index) => throw new InvalidOperationException("boom");
        public Task RemoveAsync(string id, bool recursive) => throw new InvalidOperationException("boom");
        public Task<string> CreateAsync(string parentId, string title, string? url, int? index) => throw new InvalidOperationException("boom");
    }

    /// <summary>読み取りは本物の接続、書き込みは常に失敗する。</summary>
    private sealed class FailingConnection : IBookmarkConnection
    {
        private readonly FakeConnection _inner;
        private readonly IBookmarkGateway _gateway;

        public FailingConnection(FakeConnection inner, IBookmarkGateway gateway)
        {
            _inner = inner;
            _gateway = gateway;
        }

        public bool IsConnected => _inner.IsConnected;
        public BookmarkSnapshot? LatestSnapshot => _inner.LatestSnapshot;
        public int CommandCount => _inner.CommandCount;

        public event Action<bool>? ConnectionChanged { add => _inner.ConnectionChanged += value; remove => _inner.ConnectionChanged -= value; }
        public event Action<BookmarkSnapshot>? SnapshotReceived { add => _inner.SnapshotReceived += value; remove => _inner.SnapshotReceived -= value; }

        public Task RequestSnapshotAsync() => _inner.RequestSnapshotAsync();
        public Task WhenNextSnapshot() => _inner.WhenNextSnapshot();
        public Task UpdateAsync(string id, string title, string? url) => _gateway.UpdateAsync(id, title, url);
        public Task MoveAsync(string id, string parentId, int? index) => _gateway.MoveAsync(id, parentId, index);
        public Task RemoveAsync(string id, bool recursive) => _gateway.RemoveAsync(id, recursive);
        public Task<string> CreateAsync(string parentId, string title, string? url, int? index) => _gateway.CreateAsync(parentId, title, url, index);
    }
}
