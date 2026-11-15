namespace Waypoint.Settings.Bookmarks.Services;

/// <summary>UI スレッドへの処理の受け渡し。ViewModel が WPF に依存しないための窓口。</summary>
internal interface IUiDispatcher
{
    public void Post(Action action);
}
