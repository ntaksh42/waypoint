using System.Windows;
using Waypoint.Settings.Bookmarks;

namespace Waypoint.Settings;

public partial class App : Application
{
    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        // `--bookmarks` は Edge のお気に入り編集画面 (FR-6.16)。それ以外は従来どおり設定画面
        if (BookmarkEditorStartup.IsRequested(e.Args))
        {
            BookmarkEditorStartup.Run(this);
            return;
        }
        new MainWindow().Show();
    }
}
