using System.Windows.Input;

namespace Waypoint.Settings.Bookmarks.ViewModels;

/// <summary>実行可否を ViewModel が明示的に通知する ICommand の土台。</summary>
internal abstract class CommandBase : ICommand
{
    private readonly Func<bool>? _canExecute;

    protected CommandBase(Func<bool>? canExecute) => _canExecute = canExecute;

    public event EventHandler? CanExecuteChanged;

    public bool CanExecute(object? parameter) => _canExecute?.Invoke() ?? true;

    public abstract void Execute(object? parameter);

    public void NotifyCanExecuteChanged() => CanExecuteChanged?.Invoke(this, EventArgs.Empty);
}

internal sealed class RelayCommand : CommandBase
{
    private readonly Action _execute;

    public RelayCommand(Action execute, Func<bool>? canExecute = null) : base(canExecute) => _execute = execute;

    public override void Execute(object? parameter) => _execute();
}

/// <summary>非同期の処理を実行する。失敗は onError に渡し、アプリを落とさない。</summary>
internal sealed class AsyncRelayCommand : CommandBase
{
    private readonly Func<Task> _execute;
    private readonly Action<Exception> _onError;

    public AsyncRelayCommand(Func<Task> execute, Func<bool>? canExecute, Action<Exception> onError) : base(canExecute)
    {
        _execute = execute;
        _onError = onError;
    }

    public override async void Execute(object? parameter)
    {
        try { await _execute(); }
        catch (Exception e) { _onError(e); }
    }
}
