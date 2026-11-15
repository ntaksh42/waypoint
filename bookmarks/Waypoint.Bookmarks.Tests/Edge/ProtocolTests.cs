using System.Text;
using System.Text.Json;
using Waypoint.Bookmarks.Edge;
using Waypoint.Bookmarks.Edge.Protocol;

namespace Waypoint.Bookmarks.Tests.Edge;

public class FramingTests
{
    [Fact]
    public async Task A_frame_round_trips_including_japanese_text()
    {
        var body = Encoding.UTF8.GetBytes("{\"title\":\"お気に入り\"}");
        var stream = new MemoryStream();

        await Framing.WriteFrameAsync(stream, body);
        stream.Position = 0;

        Assert.Equal(body, await Framing.ReadFrameAsync(stream));
    }

    [Fact]
    public async Task Several_frames_are_read_in_order_then_null_at_the_end()
    {
        var stream = new MemoryStream();
        await Framing.WriteFrameAsync(stream, new byte[] { 1 });
        await Framing.WriteFrameAsync(stream, new byte[] { 2, 3 });
        stream.Position = 0;

        Assert.Equal(new byte[] { 1 }, await Framing.ReadFrameAsync(stream));
        Assert.Equal(new byte[] { 2, 3 }, await Framing.ReadFrameAsync(stream));
        Assert.Null(await Framing.ReadFrameAsync(stream));
    }

    [Fact]
    public async Task An_empty_frame_is_valid()
    {
        var stream = new MemoryStream();
        await Framing.WriteFrameAsync(stream, Array.Empty<byte>());
        stream.Position = 0;

        Assert.Empty((await Framing.ReadFrameAsync(stream))!);
    }

    [Fact]
    public async Task A_truncated_frame_returns_null()
    {
        var stream = new MemoryStream(new byte[] { 10, 0, 0, 0, 1, 2 });

        Assert.Null(await Framing.ReadFrameAsync(stream));
    }

    [Theory]
    [InlineData(new byte[] { 0xFF, 0xFF, 0xFF, 0xFF })]
    [InlineData(new byte[] { 0, 0, 0, 0x7F })]
    public async Task An_invalid_length_returns_null(byte[] header)
    {
        Assert.Null(await Framing.ReadFrameAsync(new MemoryStream(header)));
    }
}

public class SnapshotParserTests
{
    [Fact]
    public void Nested_json_becomes_a_linked_tree()
    {
        const string json = """
            [
              {"id":"1","title":"お気に入りバー","parentId":"0","index":0,"children":[
                {"id":"10","title":"folder","parentId":"1","index":0,"children":[
                  {"id":"11","title":"site","url":"https://example.com/","parentId":"10","index":0}
                ]}
              ]},
              {"id":"2","title":"その他","parentId":"0","index":1,"children":[]}
            ]
            """;

        var tree = SnapshotParser.Parse(JsonDocument.Parse(json).RootElement);

        Assert.Equal(new[] { "1", "2" }, tree.Roots.Select(r => r.Id));
        var site = tree.Find("11")!;
        Assert.Equal("https://example.com/", site.Url);
        Assert.False(site.IsFolder);
        Assert.Equal("お気に入りバー / folder", site.FolderPath);
        Assert.True(tree.Find("10")!.IsFolder);
        Assert.True(tree.Find("2")!.IsFolder);
    }

    [Fact]
    public void Missing_optional_fields_use_defaults()
    {
        var tree = SnapshotParser.Parse(JsonDocument.Parse("""[{"id":"5"}]""").RootElement);

        var node = tree.Find("5")!;
        Assert.Equal("", node.Title);
        Assert.Equal("", node.ParentId);
        Assert.Equal(0, node.Index);
        Assert.True(node.IsFolder);
    }
}
