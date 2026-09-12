using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings;

public partial class MainWindow
{
    private HotkeyRecorder? _recorder;

    private void RecordQuickLaunchHotkey_Click(object sender, RoutedEventArgs e) => StartRecording(QuickLaunchHotkey);

    private void StartRecording(TextBox target)
    {
        _recorder?.Dispose();
        _recorder = new HotkeyRecorder();
        _recorder.Captured += hotkey =>
        {
            if (hotkey is not null) { target.Text = hotkey; Changed(); }
            StatusText.Text = hotkey is null ? "Hotkey recording cancelled" : $"Recorded {hotkey}";
        };
        try { _recorder.Start(); StatusText.Text = "Press a key combination (Esc cancels)"; }
        catch (Exception error) { MessageBox.Show(this, error.Message, "Waypoint", MessageBoxButton.OK, MessageBoxImage.Error); }
    }
}
