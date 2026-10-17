using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Waypoint.Settings;

/// <summary>Rust 側と共有する config.json を、未知のフィールドを失わずに扱う。</summary>
public sealed class ConfigStore
{
    private const uint WmApp = 0x8000;
    private const uint WmReloadConfig = WmApp + 3;

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern nint FindWindow(string className, string? windowName);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool PostMessage(nint hwnd, uint msg, nuint wParam, nint lParam);

    public JsonObject Root { get; private set; } = new();
    public string ConfigPath { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "waypoint", "config.json");
    private string SharedSettingsPath => Path.Combine(Path.GetDirectoryName(ConfigPath)!, "shared.json");

    public void Load()
    {
        if (File.Exists(ConfigPath))
        {
            Root = JsonNode.Parse(File.ReadAllText(ConfigPath)) as JsonObject
                ?? throw new InvalidDataException("config.json must be a JSON object.");
            EnsureShape();
            ApplySharedSettings();
            return;
        }

        // 常駐部 (config::load) と同じく、中断された保存で config.json だけが
        // 消えた場合は .bak.json から戻す。保存は shared.json も書き換えるので、
        // 既存の共有設定を取り込んでから書き出す。
        Root = TryReadObject(Path.ChangeExtension(ConfigPath, "bak.json")) ?? DefaultConfig();
        EnsureShape();
        ApplySharedSettings();
        Save();
    }

    private void ApplySharedSettings()
    {
        if (TryReadObject(SharedSettingsPath) is { } shared) Root["settings"] = shared;
    }

    private static JsonObject? TryReadObject(string path)
    {
        try { return File.Exists(path) ? JsonNode.Parse(File.ReadAllText(path)) as JsonObject : null; }
        catch (JsonException) { return null; }
    }

    public void Save()
    {
        EnsureShape();
        var options = new JsonSerializerOptions { WriteIndented = true, Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping };
        WriteAtomic(ConfigPath, Root.ToJsonString(options));
        try { WriteAtomic(SharedSettingsPath, Root["settings"]!.ToJsonString(options)); }
        catch (IOException) { }
        var tray = FindWindow("WaypointMessageWindow", null);
        if (tray != 0) _ = PostMessage(tray, WmReloadConfig, 0, 0);
    }

    public JsonObject Settings => RequireObject(Root, "settings");
    public JsonObject Variables => RequireObject(Root, "variables");
    public JsonArray Items => RequireArray(Root, "items");

    private static void WriteAtomic(string path, string contents)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var temp = path + ".tmp";
        File.WriteAllText(temp, contents, new UTF8Encoding(false));
        if (File.Exists(path))
        {
            File.Replace(temp, path, Path.ChangeExtension(path, "bak.json"), ignoreMetadataErrors: true);
        }
        else
        {
            File.Move(temp, path);
        }
    }

    private void EnsureShape()
    {
        _ = RequireObject(Root, "variables");
        _ = RequireObject(Root, "settings");
        _ = RequireArray(Root, "items");
        _ = RequireObject(Settings, "quickLaunch");
    }

    private static JsonObject RequireObject(JsonObject parent, string name)
        => parent[name] as JsonObject ?? (JsonObject)(parent[name] = new JsonObject());
    private static JsonArray RequireArray(JsonObject parent, string name)
        => parent[name] as JsonArray ?? (JsonArray)(parent[name] = new JsonArray());

    private static JsonObject DefaultConfig() => JsonNode.Parse("""
    {"version":1,"variables":{},"settings":{"quickLaunch":{"hotkey":"Alt+Space","includeRecentFolders":true,"includeFrequentFolders":true,"includeOpenWindows":true,"includeBookmarks":true,"includeBrowserHistory":true,"includeApps":true,"includeEverything":true,"searchPaths":false,"editorCommand":"code","visibleResults":12,"monitor":"primary","azureDevops":{"enabled":false,"projects":[]}},"startWithWindows":false},"items":[{"type":"submenu","name":"My Special Folders","items":[{"type":"specialFolder","name":"Desktop","knownFolder":"Desktop"},{"type":"specialFolder","name":"Documents","knownFolder":"Documents"},{"type":"specialFolder","name":"Pictures","knownFolder":"Pictures"},{"type":"specialFolder","name":"Downloads","knownFolder":"Downloads"},{"type":"separator"},{"type":"shell","name":"This PC","target":"shell:MyComputerFolder"},{"type":"shell","name":"Network","target":"shell:NetworkPlacesFolder"},{"type":"shell","name":"All Control Panel Items","target":"shell:ControlPanelFolder"},{"type":"shell","name":"Recycle Bin","target":"shell:RecycleBinFolder"}]},{"type":"separator"},{"type":"folder","name":"Profile","path":"%USERPROFILE%"}]}
    """)!.AsObject();
}
