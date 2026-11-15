namespace Waypoint.Bookmarks.Edge.Protocol;

/// <summary>4 バイト長 (リトルエンディアン) + UTF-8 JSON のフレーム。Native Messaging と名前付きパイプで共通。</summary>
public static class Framing
{
    private const int MaxFrameBytes = 64 * 1024 * 1024;

    /// <returns>1 フレーム分の本文。ストリームが閉じたとき、または長さが不正なときは null。</returns>
    public static async Task<byte[]?> ReadFrameAsync(Stream stream, CancellationToken ct = default)
    {
        var head = new byte[4];
        if (!await ReadExactAsync(stream, head, ct)) return null;

        var length = BitConverter.ToInt32(head);
        if (length < 0 || length > MaxFrameBytes) return null;

        var body = new byte[length];
        return await ReadExactAsync(stream, body, ct) ? body : null;
    }

    public static async Task WriteFrameAsync(Stream stream, byte[] body, CancellationToken ct = default)
    {
        var frame = new byte[4 + body.Length];
        BitConverter.GetBytes(body.Length).CopyTo(frame, 0);
        body.CopyTo(frame, 4);
        await stream.WriteAsync(frame, ct);
        await stream.FlushAsync(ct);
    }

    private static async Task<bool> ReadExactAsync(Stream stream, byte[] buffer, CancellationToken ct)
    {
        var offset = 0;
        while (offset < buffer.Length)
        {
            var read = await stream.ReadAsync(buffer.AsMemory(offset), ct);
            if (read <= 0) return false;
            offset += read;
        }
        return true;
    }
}
