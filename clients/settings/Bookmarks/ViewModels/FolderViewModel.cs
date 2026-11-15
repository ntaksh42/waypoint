using System.Collections.ObjectModel;
using Waypoint.Bookmarks.Core.Bookmarks;

namespace Waypoint.Settings.Bookmarks.ViewModels;

/// <summary>左のフォルダツリーの 1 行。Node が null のものは「すべてのお気に入り」。</summary>
internal sealed class FolderViewModel : ViewModelBase
{
    private readonly Action<FolderViewModel> _onSelected;
    private bool _isExpanded;
    private bool _isSelected;

    public FolderViewModel(BookmarkNode? node, string title, int bookmarkCount, FolderViewModel? parent,
        Action<FolderViewModel> onSelected)
    {
        Node = node;
        Title = title;
        BookmarkCount = bookmarkCount;
        Parent = parent;
        _onSelected = onSelected;
    }

    public BookmarkNode? Node { get; }
    public string Title { get; }
    public int BookmarkCount { get; }
    public FolderViewModel? Parent { get; }
    public ObservableCollection<FolderViewModel> Children { get; } = new();

    public string? Id => Node?.Id;
    public string DisplayText => $"{(Node is null ? "📚" : "📁")} {Title} ({BookmarkCount})";

    public bool IsExpanded
    {
        get => _isExpanded;
        set => SetProperty(ref _isExpanded, value);
    }

    public bool IsSelected
    {
        get => _isSelected;
        set
        {
            if (!SetProperty(ref _isSelected, value)) return;
            if (value) _onSelected(this);
        }
    }

    public IEnumerable<FolderViewModel> SelfAndDescendants()
    {
        yield return this;
        foreach (var child in Children)
            foreach (var d in child.SelfAndDescendants()) yield return d;
    }
}
