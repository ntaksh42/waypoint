using System.Text.Json.Nodes;

namespace Waypoint.Settings;

public sealed class MenuNode(string title, List<int> path, JsonArray items)
{
    public string Title { get; } = title;
    public List<int> Path { get; } = path;
    public JsonArray Items { get; } = items;
    public List<MenuNode> Children { get; } = [];
}

public sealed class ItemRow(JsonObject item, int index)
{
    public JsonObject Item { get; } = item;
    public int Index { get; } = index;
    public List<int>? MenuPath { get; init; }
    public string MenuName { get; init; } = "";
    public string Name => Item["name"]?.GetValue<string>() ?? "(separator)";
    public string Type => Item["type"]?.GetValue<string>() ?? "";
    public string Target => Type switch
    {
        "folder" or "file" => Item["path"]?.GetValue<string>() ?? "",
        "specialFolder" => Item["knownFolder"]?.GetValue<string>() ?? "",
        "shell" => Item["target"]?.GetValue<string>() ?? "",
        "submenu" => $"{(Item["items"] as JsonArray)?.Count ?? 0} item(s)",
        _ => "",
    };
}

public sealed record MenuChoice(string Name, List<int> Path)
{
    public override string ToString() => Name;
}

public sealed class VariableRow(string name, string value)
{
    public string Name { get; set; } = name;
    public string Value { get; set; } = value;
}

public sealed class AzureProjectRow(JsonObject project)
{
    public JsonObject ProjectNode { get; } = project;
    public string Organization => ProjectNode["organization"]?.GetValue<string>() ?? "";
    public string Project => ProjectNode["project"]?.GetValue<string>() ?? "";
    public int Priority => ProjectNode["priority"]?.GetValue<int?>() ?? 0;
    public bool IncludePullRequests => ProjectNode["includePullRequests"]?.GetValue<bool?>() ?? true;
    public bool IncludePipelines => ProjectNode["includePipelines"]?.GetValue<bool?>() ?? true;
    public bool IncludeWorkItems => ProjectNode["includeWorkItems"]?.GetValue<bool?>() ?? true;
    public int RepositoryCount => (ProjectNode["interestRepositories"] as JsonArray)?.Count ?? 0;
}
