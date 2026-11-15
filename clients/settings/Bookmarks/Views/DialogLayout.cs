using System.Windows;
using System.Windows.Controls;

namespace Waypoint.Settings.Bookmarks.Views;

/// <summary>コードで組むダイアログ共通の部品。</summary>
internal static class DialogLayout
{
    public static void Configure(Window dialog, Window owner, string title, double width, double height)
    {
        dialog.Owner = owner;
        dialog.Title = title;
        dialog.Width = width;
        dialog.Height = height;
        dialog.WindowStartupLocation = WindowStartupLocation.CenterOwner;
        dialog.ShowInTaskbar = false;
    }

    /// <summary>右寄せの「OK / キャンセル」。OK を押すと DialogResult が true になる。</summary>
    public static StackPanel Buttons(Window dialog, Button ok)
    {
        var cancel = new Button { Content = "Cancel", Width = 90, Margin = new Thickness(8, 0, 0, 0), IsCancel = true };
        ok.Width = 90;
        ok.IsDefault = true;
        ok.Click += (_, _) => dialog.DialogResult = true;
        return new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Children = { ok, cancel },
        };
    }
}
