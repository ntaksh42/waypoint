using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace Waypoint.Settings;

/// <summary>予約済みの Win 修飾キーも含めて記録するための低レベルキーボードフック。</summary>
public sealed class HotkeyRecorder : IDisposable
{
    private const int WhKeyboardLl = 13;
    private const int WmKeyDown = 0x0100;
    private const int WmSysKeyDown = 0x0104;
    private const int WmKeyUp = 0x0101;
    private const int WmSysKeyUp = 0x0105;
    private const int VkEscape = 0x1B;
    private const int VkControl = 0x11;
    private const int VkMenu = 0x12;
    private const int VkShift = 0x10;
    private const int VkLwin = 0x5B;
    private const int VkRwin = 0x5C;
    private HookProc? _callback;
    private nint _hook;
    private readonly HashSet<int> _consumedKeys = [];
    private bool _finished;
    public event Action<string?>? Captured;

    public void Start()
    {
        Stop();
        _finished = false;
        _callback = Callback;
        _hook = SetWindowsHookEx(WhKeyboardLl, _callback, GetModuleHandle(null), 0);
        if (_hook == 0) throw new InvalidOperationException("Could not start hotkey recording.");
    }

    private nint Callback(int code, nint wParam, nint lParam)
    {
        if (code < 0) return CallNextHookEx(_hook, code, wParam, lParam);
        var key = Marshal.ReadInt32(lParam);
        if (wParam == WmKeyUp || wParam == WmSysKeyUp)
        {
            if (!_consumedKeys.Remove(key)) return CallNextHookEx(_hook, code, wParam, lParam);
            if (_finished && _consumedKeys.Count == 0) Stop();
            return 1;
        }
        if (wParam != WmKeyDown && wParam != WmSysKeyDown) return CallNextHookEx(_hook, code, wParam, lParam);
        _consumedKeys.Add(key);
        if (_finished) return 1;
        if (key == VkEscape) { Finish(null); return 1; }
        if (key is VkControl or VkMenu or VkShift or VkLwin or VkRwin) return 1;
        var parts = new List<string>();
        if (Pressed(VkControl)) parts.Add("Ctrl");
        if (Pressed(VkMenu)) parts.Add("Alt");
        if (Pressed(VkShift)) parts.Add("Shift");
        if (Pressed(VkLwin) || Pressed(VkRwin)) parts.Add("Win");
        parts.Add(KeyName(key));
        Finish(string.Join('+', parts));
        return 1;
    }

    private void Finish(string? value)
    {
        _finished = true;
        Application.Current.Dispatcher.BeginInvoke(() => Captured?.Invoke(value));
    }

    private static bool Pressed(int key) => (GetAsyncKeyState(key) & 0x8000) != 0;
    private static string KeyName(int key) => key switch
    {
        >= 0x70 and <= 0x87 => $"F{key - 0x6F}",
        0x20 => "Space", 0x0D => "Enter", 0x09 => "Tab", 0x2E => "Delete", 0x1B => "Esc",
        _ => System.Windows.Input.KeyInterop.KeyFromVirtualKey(key).ToString(),
    };

    public void Stop()
    {
        if (_hook != 0) UnhookWindowsHookEx(_hook);
        _hook = 0;
        _callback = null;
        _consumedKeys.Clear();
    }

    public void Dispose() => Stop();
    private delegate nint HookProc(int code, nint wParam, nint lParam);
    [DllImport("user32.dll", SetLastError = true)] private static extern nint SetWindowsHookEx(int idHook, HookProc callback, nint module, uint threadId);
    [DllImport("user32.dll")] private static extern nint CallNextHookEx(nint hook, int code, nint wParam, nint lParam);
    [DllImport("user32.dll")] private static extern bool UnhookWindowsHookEx(nint hook);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] private static extern nint GetModuleHandle(string? name);
}
