using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Settings.Bookmarks.ViewModels;

/// <summary>右の一覧の 1 行 (お気に入り 1 件)。</summary>
internal sealed class RowViewModel : ViewModelBase
{
    private readonly Action _onSelectionChanged;
    private string _title;
    private string _url;
    private bool _isSelected;

    public RowViewModel(BookmarkNode node, bool isDuplicate, Action onSelectionChanged)
    {
        Node = node;
        _title = node.Title;
        _url = node.Url ?? "";
        FolderPath = node.FolderPath;
        DuplicateMark = isDuplicate ? "Dup" : "";
        _onSelectionChanged = onSelectionChanged;
    }

    public BookmarkNode Node { get; }

    /// <summary>セルの編集中の値。確定すると、Edge から届く新しいスナップショットで作り直される。</summary>
    public string Title
    {
        get => _title;
        set => SetProperty(ref _title, value);
    }

    public string Url
    {
        get => _url;
        set => SetProperty(ref _url, value);
    }

    public string FolderPath { get; }
    public string DuplicateMark { get; }

    public bool IsSelected
    {
        get => _isSelected;
        set
        {
            if (SetProperty(ref _isSelected, value)) _onSelectionChanged();
        }
    }
}
