using System.Net.Http.Headers;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json.Nodes;

namespace Waypoint.Settings;

public static class AzureClient
{
    private static HttpClient CreateClient(string pat)
    {
        var client = new HttpClient();
        client.DefaultRequestHeaders.Authorization = pat.StartsWith(AzureCredential.BearerPrefix, StringComparison.Ordinal)
            ? new AuthenticationHeaderValue("Bearer", pat[AzureCredential.BearerPrefix.Length..])
            : new AuthenticationHeaderValue("Basic", Convert.ToBase64String(Encoding.UTF8.GetBytes($":{pat}")));
        return client;
    }

    public static async Task<IReadOnlyList<string>> LoadProjectsAsync(string organization, string pat)
    {
        using var client = CreateClient(pat);
        var projects = new List<string>();
        string? continuation = null;
        do
        {
            var url = $"https://dev.azure.com/{Uri.EscapeDataString(organization)}/_apis/projects?api-version=7.1";
            if (continuation is not null) url += $"&continuationToken={Uri.EscapeDataString(continuation)}";
            using var response = await client.GetAsync(url);
            response.EnsureSuccessStatusCode();
            var body = JsonNode.Parse(await response.Content.ReadAsStringAsync())!.AsObject();
            projects.AddRange((body["value"] as JsonArray ?? []).OfType<JsonObject>().Select(project => project["name"]?.GetValue<string>() ?? "").Where(name => name.Length > 0));
            continuation = response.Headers.TryGetValues("x-ms-continuationtoken", out var values) ? values.FirstOrDefault() : null;
        } while (continuation is not null);
        return projects.OrderBy(name => name, StringComparer.OrdinalIgnoreCase).ToList();
    }

    public static async Task<IReadOnlyList<string>> LoadRepositoriesAsync(string organization, string project, string pat)
    {
        using var client = CreateClient(pat);
        var url = $"https://dev.azure.com/{Uri.EscapeDataString(organization)}/{Uri.EscapeDataString(project)}/_apis/git/repositories?api-version=7.1";
        var body = JsonNode.Parse(await client.GetStringAsync(url))!.AsObject();
        return (body["value"] as JsonArray ?? []).OfType<JsonObject>()
            .Select(repository => repository["name"]?.GetValue<string>() ?? "")
            .Where(name => name.Length > 0).OrderBy(name => name, StringComparer.OrdinalIgnoreCase).ToList();
    }

    public static async Task<IReadOnlyList<string>> LoadAreasAsync(string organization, string project, string pat)
    {
        using var client = CreateClient(pat);
        var url = $"https://dev.azure.com/{Uri.EscapeDataString(organization)}/{Uri.EscapeDataString(project)}/_apis/wit/classificationnodes/areas?$depth=14&api-version=7.1";
        var root = JsonNode.Parse(await client.GetStringAsync(url))!.AsObject();
        var names = new List<string>();
        CollectNodes(root, "", names);
        return names;
    }

    public static async Task<int> CountRecentAssignedWorkItemsAsync(string organization, string project, string pat)
    {
        using var client = CreateClient(pat);
        // WIQL REST では @Me / @Today の UI マクロが展開されないため、アカウントの
        // 最近の作業 API を使ってからプロジェクトと期間で絞り込む。
        var url = $"https://dev.azure.com/{Uri.EscapeDataString(organization)}/_apis/work/accountmyworkrecentactivity?api-version=7.1";
        var response = JsonNode.Parse(await client.GetStringAsync(url));
        var entries = response as JsonArray ?? response?["value"] as JsonArray ?? [];
        var cutoff = DateTime.UtcNow.AddDays(-90);
        return entries.OfType<JsonObject>().Count(item =>
            string.Equals(item["teamProject"]?.GetValue<string>(), project, StringComparison.OrdinalIgnoreCase)
            && DateTime.TryParse(item["activityDate"]?.GetValue<string>(), out var date)
            && date.ToUniversalTime() >= cutoff);
    }

    public static async Task<IReadOnlyList<string>> LoadAssignedAreaSuggestionsAsync(string organization, string project, string pat)
    {
        using var client = CreateClient(pat);
        var baseUrl = $"https://dev.azure.com/{Uri.EscapeDataString(organization)}/{Uri.EscapeDataString(project)}";
        using var response = await client.PostAsync($"{baseUrl}/_apis/wit/wiql?$top=200&api-version=7.1", new StringContent("{\"query\":\"Select [System.Id] From WorkItems Where [System.AssignedTo] = @Me\"}", Encoding.UTF8, "application/json"));
        response.EnsureSuccessStatusCode();
        var ids = (JsonNode.Parse(await response.Content.ReadAsStringAsync())?["workItems"] as JsonArray ?? [])
            .OfType<JsonObject>().Select(item => item["id"]?.GetValue<int?>()).Where(id => id is not null).Cast<int>().ToArray();
        if (ids.Length == 0) return [];
        var request = new JsonObject { ["ids"] = new JsonArray(ids.Select(id => (JsonNode?)id).ToArray()), ["fields"] = new JsonArray("System.AreaPath"), ["errorPolicy"] = "omit" };
        using var areasResponse = await client.PostAsync($"{baseUrl}/_apis/wit/workitemsbatch?api-version=7.1", new StringContent(request.ToJsonString(), Encoding.UTF8, "application/json"));
        areasResponse.EnsureSuccessStatusCode();
        return (JsonNode.Parse(await areasResponse.Content.ReadAsStringAsync())?["value"] as JsonArray ?? [])
            .OfType<JsonObject>().Select(item => item["fields"]?["System.AreaPath"]?.GetValue<string>()).Where(path => !string.IsNullOrWhiteSpace(path)).Cast<string>()
            .GroupBy(path => path, StringComparer.OrdinalIgnoreCase).OrderByDescending(group => group.Count()).ThenBy(group => group.Key, StringComparer.OrdinalIgnoreCase).Select(group => $"{group.Key} ({group.Count()})").ToList();
    }

    private static void CollectNodes(JsonObject node, string parent, List<string> names)
    {
        var name = node["name"]?.GetValue<string>() ?? "";
        var path = parent.Length == 0 ? name : $"{parent}\\{name}";
        if (parent.Length > 0) names.Add(path);
        foreach (var child in (node["children"] as JsonArray ?? []).OfType<JsonObject>()) CollectNodes(child, path, names);
    }
}

/// <summary>
/// 入力中の PAT → 保存済みの PAT → `az login` 済みの Azure CLI のトークンの順に
/// 認証情報を決める (FR-9.18.2)。Azure CLI のトークンは <see cref="BearerPrefix"/>
/// 付きで返し、<c>CreateClient</c> が Bearer として送る。PAT は空白を含まないので取り違えない。
/// </summary>
public static class AzureCredential
{
    public const string BearerPrefix = "Bearer ";
    private const string AzureDevOpsResource = "499b84ac-1321-427f-aa17-267ca6975798";

    public static async Task<string> ResolveAsync(string organization, string typedPat)
    {
        if (typedPat.Length > 0) return typedPat;
        try { return CredentialStore.Load(organization); }
        catch (InvalidOperationException patError)
        {
            try { return BearerPrefix + await AzureCliTokenAsync(); }
            catch (Exception cliError) { throw new InvalidOperationException($"{patError.Message} {cliError.Message}"); }
        }
    }

    private static async Task<string> AzureCliTokenAsync()
    {
        // az の実体は az.cmd なので cmd.exe 経由で起動する
        var start = new System.Diagnostics.ProcessStartInfo("cmd.exe", $"/c az account get-access-token --resource {AzureDevOpsResource} --query accessToken --output tsv")
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        using var process = System.Diagnostics.Process.Start(start) ?? throw new InvalidOperationException("Could not run Azure CLI.");
        var output = process.StandardOutput.ReadToEndAsync();
        var error = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        var token = (await output).Trim();
        if (process.ExitCode != 0 || token.Length == 0)
        {
            var reason = (await error).Split('\n').Select(line => line.Trim()).FirstOrDefault(line => line.Length > 0);
            throw new InvalidOperationException(reason is null ? "Azure CLI sign-in is not available. Run `az login`." : $"Azure CLI sign-in is not available: {reason}");
        }
        return token;
    }
}

public static class CredentialStore
{
    private const uint CredTypeGeneric = 1;
    private const uint CredPersistEnterprise = 3;
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct Credential
    {
        public uint Flags; public uint Type; public string TargetName; public string? Comment; public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize; public nint CredentialBlob; public uint Persist; public uint AttributeCount; public nint Attributes; public string? TargetAlias; public string UserName;
    }

    // Rust の keyring::Entry::new("Waypoint", "azure-devops:<org>") と同じ形式。
    // Windows の keyring は TargetName を "{user}.{service}" と組み立てる。
    private static string User(string organization) => $"azure-devops:{organization.Trim().ToLowerInvariant()}";
    private static string Target(string organization) => $"{User(organization)}.Waypoint";
    public static void Save(string organization, string pat)
    {
        if (string.IsNullOrWhiteSpace(organization) || string.IsNullOrWhiteSpace(pat)) throw new InvalidDataException("Organization and PAT are required.");
        var bytes = Encoding.UTF8.GetBytes(pat.Trim());
        var blob = Marshal.AllocCoTaskMem(bytes.Length);
        try
        {
            Marshal.Copy(bytes, 0, blob, bytes.Length);
            var credential = new Credential { Type = CredTypeGeneric, TargetName = Target(organization), CredentialBlobSize = (uint)bytes.Length, CredentialBlob = blob, Persist = CredPersistEnterprise, UserName = User(organization) };
            if (!CredWrite(ref credential, 0)) throw new InvalidOperationException($"Could not save PAT (Win32 {Marshal.GetLastWin32Error()}).");
        }
        finally { Marshal.FreeCoTaskMem(blob); }
    }

    public static string Load(string organization)
    {
        if (!CredRead(Target(organization), CredTypeGeneric, 0, out var pointer)) throw new InvalidOperationException($"No PAT is saved for Azure DevOps organization \"{organization}\".");
        try
        {
            var credential = Marshal.PtrToStructure<Credential>(pointer);
            var bytes = new byte[credential.CredentialBlobSize];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            return Encoding.UTF8.GetString(bytes);
        }
        finally { CredFree(pointer); }
    }

    public static void Delete(string organization)
    {
        if (!CredDelete(Target(organization), CredTypeGeneric, 0) && Marshal.GetLastWin32Error() != 1168) throw new InvalidOperationException($"Could not delete PAT (Win32 {Marshal.GetLastWin32Error()}).");
    }

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredWrite(ref Credential credential, uint flags);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredRead(string target, uint type, uint flags, out nint credential);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredDelete(string target, uint type, uint flags);
    [DllImport("advapi32.dll")] private static extern void CredFree(nint buffer);
}
