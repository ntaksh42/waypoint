namespace Waypoint.Bookmarks.Core.Editing;

/// <summary>ブラウザのお気に入りを書き換える操作。実体は Edge 拡張機能 (chrome.bookmarks) への依頼。</summary>
public interface IBookmarkGateway
{
    /// <param name="url">フォルダでは null。</param>
    public Task UpdateAsync(string id, string title, string? url);

    /// <param name="index">移動前の並びでの挿入位置 (chrome.bookmarks.move と同じ)。null なら末尾。</param>
    public Task MoveAsync(string id, string parentId, int? index);

    /// <param name="recursive">フォルダを中身ごと消すとき true。</param>
    public Task RemoveAsync(string id, bool recursive);

    /// <param name="url">フォルダでは null。</param>
    /// <param name="index">挿入位置。null なら末尾。</param>
    /// <returns>作られた節点の ID。</returns>
    public Task<string> CreateAsync(string parentId, string title, string? url, int? index);
}
