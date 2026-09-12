using System.Text.Json;
using System.Text.Json.Nodes;
using System.Windows;

namespace Waypoint.Settings;

public partial class MainWindow
{
    private async void SavePatAndLoad_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            CredentialStore.Save(AzureConnectionOrganization.Text, AzurePat.Password);
            AzurePat.Clear();
            await LoadAzureProjectsFromServer();
        }
        catch (Exception error) { AzureConnectionStatus.Text = error.Message; }
    }

    private async void LoadAzureProjects_Click(object sender, RoutedEventArgs e)
    {
        try { await LoadAzureProjectsFromServer(); }
        catch (Exception error) { AzureConnectionStatus.Text = error.Message; }
    }

    private async Task LoadAzureProjectsFromServer()
    {
        var organization = AzureConnectionOrganization.Text.Trim();
        if (organization.Length == 0) throw new InvalidDataException("Organization is required.");
        var pat = AzurePat.Password.Length > 0 ? AzurePat.Password : CredentialStore.Load(organization);
        AzureConnectionStatus.Text = "Loading projects...";
        var available = await AzureClient.LoadProjectsAsync(organization, pat);
        AzureConnectionStatus.Text = $"Loaded {available.Count} project(s).";
        var picker = new AzureProjectPickerWindow(available) { Owner = this };
        if (picker.ShowDialog() != true) return;
        foreach (var name in picker.Selected)
        {
            if (_azureProjects.Any(row => row.Organization.Equals(organization, StringComparison.OrdinalIgnoreCase) && row.Project.Equals(name, StringComparison.OrdinalIgnoreCase))) continue;
            var project = NewAzureProject(organization, name);
            Array(AzureSettings(), "projects").Add(project);
            _azureProjects.Add(new AzureProjectRow(project));
        }
        Changed();
        RebuildAzureTree();
    }

    private void DeletePat_Click(object sender, RoutedEventArgs e)
    {
        try { CredentialStore.Delete(AzureConnectionOrganization.Text); AzureConnectionStatus.Text = "PAT removed from Credential Manager."; }
        catch (Exception error) { AzureConnectionStatus.Text = error.Message; }
    }

    private async void LoadRepositories_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            var (organization, project, pat) = AzureProjectConnection();
            AzureConnectionStatus.Text = "Loading repositories...";
            var picker = new MultiSelectWindow("Interest repositories", "Choose repositories to include. Leave everything unchecked to search all repositories.", await AzureClient.LoadRepositoriesAsync(organization, project, pat), LinesToArray(AzureRepositories.Text).Select(value => value!.GetValue<string>())) { Owner = this };
            if (picker.ShowDialog() != true) return;
            AzureRepositories.Text = string.Join(Environment.NewLine, picker.Selected);
            AzureConnectionStatus.Text = "Repositories updated.";
            Changed();
        }
        catch (Exception error) { AzureConnectionStatus.Text = error.Message; }
    }

    private async void LoadAreas_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            var (organization, project, pat) = AzureProjectConnection();
            AzureConnectionStatus.Text = "Loading area paths...";
            var picker = new MultiSelectWindow("Interest areas", "Choose Area Paths to include. Filter searches the whole path; indentation shows the hierarchy.", await AzureClient.LoadAreasAsync(organization, project, pat), LinesToArray(AzureAreas.Text).Select(value => value!.GetValue<string>())) { Owner = this };
            if (picker.ShowDialog() != true) return;
            AzureAreas.Text = string.Join(Environment.NewLine, picker.Selected);
            AzureConnectionStatus.Text = "Area paths updated.";
            Changed();
        }
        catch (Exception error) { AzureConnectionStatus.Text = error.Message; }
    }

    private async void SuggestAreas_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            var (organization, project, pat) = AzureProjectConnection();
            AzureConnectionStatus.Text = "Finding your assigned work items...";
            var suggestions = await AzureClient.LoadAssignedAreaSuggestionsAsync(organization, project, pat);
            var paths = suggestions.Select(value => value[..value.LastIndexOf(" (", StringComparison.Ordinal)]).ToList();
            var picker = new MultiSelectWindow("Suggested interest areas", "Areas are ranked by your assigned Work Items. Check entries to add them to this project.", paths, LinesToArray(AzureAreas.Text).Select(value => value!.GetValue<string>())) { Owner = this };
            if (picker.ShowDialog() != true) return;
            AzureAreas.Text = string.Join(Environment.NewLine, picker.Selected);
            AzureConnectionStatus.Text = "Area paths updated from your suggestions.";
            Changed();
        }
        catch (Exception error) { AzureConnectionStatus.Text = error.Message; }
    }

    private async void SuggestPriorities_Click(object sender, RoutedEventArgs e)
    {
        CommitAzureProject();
        if (_azureProjects.Count == 0) { AzureConnectionStatus.Text = "Add watched projects before suggesting priorities."; return; }
        try
        {
            AzureConnectionStatus.Text = "Checking recent work-item activity...";
            var activity = new List<(AzureProjectRow Project, int Count)>();
            foreach (var project in _azureProjects)
            {
                var pat = AzurePat.Password.Length > 0 && project.Organization.Equals(AzureConnectionOrganization.Text.Trim(), StringComparison.OrdinalIgnoreCase)
                    ? AzurePat.Password : CredentialStore.Load(project.Organization);
                activity.Add((project, await AzureClient.CountRecentAssignedWorkItemsAsync(project.Organization, project.Project, pat)));
            }
            foreach (var (project, priority) in activity.OrderByDescending(value => value.Count).ThenBy(value => value.Project.Project, StringComparer.OrdinalIgnoreCase).Select((value, index) => (value.Project, index)))
                project.ProjectNode["priority"] = priority;
            LoadAzureProjects();
            AzureConnectionStatus.Text = "Priorities suggested from your work items changed in the last 90 days. Save to apply.";
            Changed();
        }
        catch (Exception error) { AzureConnectionStatus.Text = error.Message; }
    }

    private (string Organization, string Project, string Pat) AzureProjectConnection()
    {
        CommitAzureProject();
        var organization = AzureOrganization.Text.Trim();
        var project = AzureProject.Text.Trim();
        if (organization.Length == 0 || project.Length == 0) throw new InvalidDataException("Select a watched project first.");
        var pat = AzurePat.Password.Length > 0 && organization.Equals(AzureConnectionOrganization.Text.Trim(), StringComparison.OrdinalIgnoreCase)
            ? AzurePat.Password : CredentialStore.Load(organization);
        return (organization, project, pat);
    }

    private void ExportAzureProjects_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.SaveFileDialog { FileName = "azure-devops-projects.json", Filter = "JSON files (*.json)|*.json" };
        if (dialog.ShowDialog(this) != true) return;
        File.WriteAllText(dialog.FileName, Array(AzureSettings(), "projects").ToJsonString(new JsonSerializerOptions { WriteIndented = true }));
        AzureConnectionStatus.Text = $"Exported {_azureProjects.Count} project(s).";
    }

    private void ImportAzureProjects_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFileDialog { Filter = "JSON files (*.json)|*.json" };
        if (dialog.ShowDialog(this) != true) return;
        try
        {
            if (JsonNode.Parse(File.ReadAllText(dialog.FileName)) is not JsonArray projects) throw new InvalidDataException("The file must contain a JSON array of projects.");
            AzureSettings()["projects"] = projects;
            LoadAzureProjects();
            AzureConnectionStatus.Text = $"Imported {_azureProjects.Count} project(s).";
            Changed();
        }
        catch (Exception error) { MessageBox.Show(this, error.Message, "Import Azure DevOps projects", MessageBoxButton.OK, MessageBoxImage.Error); }
    }

    private static JsonObject NewAzureProject(string organization, string project) => new()
    {
        ["organization"] = organization, ["project"] = project, ["aliases"] = new JsonArray(), ["priority"] = 0,
        ["includePullRequests"] = true, ["includePipelines"] = true, ["includeWorkItems"] = true,
        ["interestRepositories"] = new JsonArray(), ["interestAreas"] = new JsonArray(), ["interestIterations"] = new JsonArray(),
    };
}
