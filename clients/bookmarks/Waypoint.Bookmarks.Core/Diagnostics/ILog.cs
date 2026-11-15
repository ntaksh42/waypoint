namespace Waypoint.Bookmarks.Core.Diagnostics;

public interface ILog
{
    public void Info(string message);
    public void Error(string message);
}

/// <summary>何も記録しない。</summary>
public sealed class NullLog : ILog
{
    public static NullLog Instance { get; } = new();

    private NullLog() { }

    public void Info(string message) { }
    public void Error(string message) { }
}
