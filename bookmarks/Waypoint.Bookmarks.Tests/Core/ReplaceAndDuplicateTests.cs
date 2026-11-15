using Waypoint.Bookmarks.Core.Bookmarks;
using Waypoint.Bookmarks.Core.Editing;

namespace Waypoint.Bookmarks.Tests.Core;

public class ReplaceSpecTests
{
    private static BookmarkNode Bookmark(string title, string url) => new("id", title, url, "1", 0);

    [Fact]
    public void Plain_text_replaces_in_title_and_url_and_ignores_unchanged_items()
    {
        var spec = new ReplaceSpec("example.com", "example.org", InTitle: true, InUrl: true, UseRegex: false, IgnoreCase: true);
        var hit = Bookmark("Example.com top", "https://example.com/x");
        var miss = Bookmark("other", "https://other.test/");

        var changes = spec.Plan(new[] { hit, miss });

        var change = Assert.Single(changes);
        Assert.Equal("example.org top", change.NewTitle);
        Assert.Equal("https://example.org/x", change.NewUrl);
    }

    [Fact]
    public void Only_the_chosen_fields_change()
    {
        var node = Bookmark("a-site", "https://a-site.test/");

        var titleOnly = new ReplaceSpec("-site", "", true, false, false, true).Plan(new[] { node }).Single();
        var urlOnly = new ReplaceSpec("-site", "", false, true, false, true).Plan(new[] { node }).Single();

        Assert.Equal(("a", "https://a-site.test/"), (titleOnly.NewTitle, titleOnly.NewUrl));
        Assert.Equal(("a-site", "https://a.test/"), (urlOnly.NewTitle, urlOnly.NewUrl));
    }

    [Fact]
    public void Plain_text_treats_special_characters_literally()
    {
        var spec = new ReplaceSpec("a.b", "$1", true, false, UseRegex: false, IgnoreCase: false);

        var changes = spec.Plan(new[] { Bookmark("a.b axb", "https://x.test/") });

        Assert.Equal("$1 axb", changes.Single().NewTitle);
    }

    [Fact]
    public void Regex_supports_groups()
    {
        var spec = new ReplaceSpec(@"(\d+)-(\d+)", "$2/$1", true, false, UseRegex: true, IgnoreCase: false);

        var changes = spec.Plan(new[] { Bookmark("12-34", "https://x.test/") });

        Assert.Equal("34/12", changes.Single().NewTitle);
    }

    [Fact]
    public void Case_sensitivity_is_respected()
    {
        var node = Bookmark("ABC", "https://x.test/");

        Assert.Empty(new ReplaceSpec("abc", "x", true, false, false, IgnoreCase: false).Plan(new[] { node }));
        Assert.Single(new ReplaceSpec("abc", "x", true, false, false, IgnoreCase: true).Plan(new[] { node }));
    }

    [Fact]
    public void A_url_that_would_become_empty_is_left_alone()
    {
        var spec = new ReplaceSpec("https://x.test/", "", InTitle: false, InUrl: true, UseRegex: false, IgnoreCase: true);

        Assert.Empty(spec.Plan(new[] { Bookmark("t", "https://x.test/") }));
    }

    [Fact]
    public void Empty_search_changes_nothing()
    {
        Assert.Empty(new ReplaceSpec("", "x", true, true, false, true).Plan(new[] { Bookmark("a", "https://a.test/") }));
    }

    [Fact]
    public void Invalid_regex_throws()
    {
        var spec = new ReplaceSpec("(", "x", true, true, UseRegex: true, IgnoreCase: true);

        Assert.ThrowsAny<ArgumentException>(() => spec.Plan(new[] { Bookmark("a", "https://a.test/") }));
    }
}

public class DuplicateFinderTests
{
    private static BookmarkTree TreeOf(params (string Title, string Url)[] items)
    {
        var children = items.Select((it, i) => new BookmarkNode($"b{i}", it.Title, it.Url, "1", i)).ToList();
        return new BookmarkTree(new[] { new BookmarkNode("1", "bar", null, "0", 0, children) });
    }

    [Theory]
    [InlineData("https://Example.com/a", "https://example.com/a/")]
    [InlineData("https://example.com/a ", "https://example.com/a")]
    public void Urls_differing_only_in_case_of_host_or_trailing_slash_are_the_same(string x, string y) =>
        Assert.Equal(DuplicateFinder.KeyOf(x), DuplicateFinder.KeyOf(y));

    [Fact]
    public void Different_paths_are_not_duplicates()
    {
        var tree = TreeOf(("a", "https://example.com/a"), ("b", "https://example.com/b"));

        Assert.Empty(DuplicateFinder.FindExtras(tree));
    }

    [Fact]
    public void FindExtras_returns_second_and_later_occurrences_only()
    {
        var tree = TreeOf(("first", "https://example.com/a"), ("second", "https://EXAMPLE.com/a/"), ("third", "https://example.com/a"),
            ("other", "https://example.com/b"));

        Assert.Equal(new[] { "second", "third" }, DuplicateFinder.FindExtras(tree).Select(n => n.Title));
    }

    [Fact]
    public void CountByUrl_counts_each_group()
    {
        var tree = TreeOf(("1", "https://example.com/a"), ("2", "https://example.com/a/"), ("3", "https://example.com/b"));

        var counts = DuplicateFinder.CountByUrl(tree);

        Assert.Equal(2, counts[DuplicateFinder.KeyOf("https://example.com/a")]);
        Assert.Equal(1, counts[DuplicateFinder.KeyOf("https://example.com/b")]);
    }

    [Fact]
    public void Unparseable_urls_fall_back_to_trimmed_text()
    {
        Assert.Equal("not a url", DuplicateFinder.KeyOf(" not a url/ "));
    }
}
