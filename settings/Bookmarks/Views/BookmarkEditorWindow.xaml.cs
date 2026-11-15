using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using Waypoint.Settings.Bookmarks.ViewModels;

namespace Waypoint.Settings.Bookmarks.Views;

/// <summary>メイン画面。表示と操作は ViewModel に任せ、ここには WPF の部品に直接触れる処理 (ドラッグ、フォーカス、セル編集の受け渡し) だけを置く。</summary>
internal sealed partial class BookmarkEditorWindow : Window
{
    private const string RowsDragFormat = "favorites-editor/rows";
    private const string FolderDragFormat = "favorites-editor/folder";

    private readonly BookmarkEditorViewModel _viewModel;

    private Point _dragOrigin;
    private bool _rowDragPending;
    private FolderViewModel? _folderDragCandidate;

    public BookmarkEditorWindow(BookmarkEditorViewModel viewModel)
    {
        InitializeComponent();
        DataContext = _viewModel = viewModel;
        PreviewKeyDown += OnPreviewKeyDown;
    }

    private void OnPreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.F || Keyboard.Modifiers != ModifierKeys.Control) return;
        SearchBox.Focus();
        SearchBox.SelectAll();
        e.Handled = true;
    }

    // ---- セルの編集 ----

    private async void RowList_CellEditEnding(object? sender, DataGridCellEditEndingEventArgs e)
    {
        if (e.EditAction != DataGridEditAction.Commit || e.Row.Item is not RowViewModel row || e.EditingElement is not TextBox box)
            return;

        var isTitleColumn = e.Column.DisplayIndex == 0;
        var text = box.Text;
        if (!_viewModel.TryValidateEdit(isTitleColumn, text))
        {
            e.Cancel = true;
            return;
        }

        // 編集の確定処理が終わってから、読み取り専用の切り替えを伴う書き込みを始める
        await Task.Yield();
        await _viewModel.CommitEditAsync(row, isTitleColumn, text);
    }

    // ---- 一覧からのドラッグ ----

    private void RowList_PreviewMouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        _dragOrigin = e.GetPosition(null);
        _rowDragPending = false;

        var row = FindAncestor<DataGridRow>(e.OriginalSource as DependencyObject);
        if (row is null || FindAncestor<TextBox>(e.OriginalSource as DependencyObject) is not null) return;

        _rowDragPending = true;
        // 複数選択のまま掴めるよう、選択済みの行を押したときは選択を崩さない (離したときに 1 件へ絞る)
        if (row.IsSelected && RowList.SelectedItems.Count > 1 && Keyboard.Modifiers == ModifierKeys.None) e.Handled = true;
    }

    private void RowList_PreviewMouseLeftButtonUp(object sender, MouseButtonEventArgs e)
    {
        if (!_rowDragPending) return;
        _rowDragPending = false;

        var row = FindAncestor<DataGridRow>(e.OriginalSource as DependencyObject);
        if (row is null || !row.IsSelected || RowList.SelectedItems.Count <= 1 || Keyboard.Modifiers != ModifierKeys.None) return;

        var item = row.Item;
        RowList.SelectedItems.Clear();
        RowList.SelectedItems.Add(item);
    }

    private void RowList_PreviewMouseMove(object sender, MouseEventArgs e)
    {
        if (!_rowDragPending || e.LeftButton != MouseButtonState.Pressed || !_viewModel.CanDragRows() || !MovedFarEnough(e)) return;

        _rowDragPending = false;
        var ids = _viewModel.SelectedRowIds().ToArray();
        if (ids.Length > 0) DragDrop.DoDragDrop(RowList, new DataObject(RowsDragFormat, ids), DragDropEffects.Move);
    }

    // ---- フォルダツリー (ドラッグ元 / ドロップ先) ----

    private void FolderTree_PreviewMouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        _dragOrigin = e.GetPosition(null);
        _folderDragCandidate = FolderAt(e.OriginalSource);
    }

    private void FolderTree_PreviewMouseMove(object sender, MouseEventArgs e)
    {
        if (e.LeftButton != MouseButtonState.Pressed || _folderDragCandidate is not { Id: { } id } folder
            || !_viewModel.CanDragFolder(folder) || !MovedFarEnough(e)) return;

        _folderDragCandidate = null;
        DragDrop.DoDragDrop(FolderTree, new DataObject(FolderDragFormat, id), DragDropEffects.Move);
    }

    private void FolderTree_DragOver(object sender, DragEventArgs e)
    {
        e.Effects = FolderAt(e.OriginalSource) is { } target && CanDrop(e, target) ? DragDropEffects.Move : DragDropEffects.None;
        e.Handled = true;
    }

    private async void FolderTree_Drop(object sender, DragEventArgs e)
    {
        e.Handled = true;
        if (FolderAt(e.OriginalSource) is not { } target || !CanDrop(e, target)) return;

        if (e.Data.GetData(RowsDragFormat) is string[] rowIds)
            await _viewModel.DropRowsAsync(rowIds, target);
        else if (e.Data.GetData(FolderDragFormat) is string folderId)
            await _viewModel.DropFolderAsync(folderId, target);
    }

    private bool CanDrop(DragEventArgs e, FolderViewModel target) =>
        e.Data.GetData(FolderDragFormat) is string folderId ? _viewModel.CanDropFolder(folderId, target)
        : e.Data.GetDataPresent(RowsDragFormat) && _viewModel.CanDropRows(target);

    // ---- 共通 ----

    private static FolderViewModel? FolderAt(object source) =>
        FindAncestor<TreeViewItem>(source as DependencyObject)?.DataContext as FolderViewModel;

    private bool MovedFarEnough(MouseEventArgs e)
    {
        var d = e.GetPosition(null) - _dragOrigin;
        return Math.Abs(d.X) >= SystemParameters.MinimumHorizontalDragDistance
               || Math.Abs(d.Y) >= SystemParameters.MinimumVerticalDragDistance;
    }

    private static T? FindAncestor<T>(DependencyObject? element) where T : DependencyObject
    {
        while (element is not null)
        {
            if (element is T match) return match;
            element = element is Visual or System.Windows.Media.Media3D.Visual3D
                ? VisualTreeHelper.GetParent(element)
                : LogicalTreeHelper.GetParent(element);
        }
        return null;
    }
}
