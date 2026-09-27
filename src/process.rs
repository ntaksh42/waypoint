//! ウィンドウを所有する実行ファイル名を調べる。プロセスの列挙と終了 (`k ` プレフィックス、FR-9.15.2)。

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::core::PWSTR;

/// `k ` プレフィックスの候補 1 件分。
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
}

/// 実行中プロセスのスナップショットを取る (FR-9.15.2)。
///
/// キー入力のたびに呼ぶ想定だが `CreateToolhelp32Snapshot` はローカル API
/// 呼び出しのみで数百プロセス規模でも軽量なため、`Index` へのキャッシュは持たず
/// 都度取得する (kill 直後の一覧を古いままにしないため)。
/// 自分自身 (`waypoint.exe` / `waypoint-settings.exe`) は自己終了事故を
/// 防ぐため常に除外する。
pub fn list_processes() -> Vec<ProcessEntry> {
    let own_pid = std::process::id();
    let mut result = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return result;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32ProcessID != 0 && entry.th32ProcessID != own_pid {
                    let name = wchar_to_string(&entry.szExeFile);
                    if !name.is_empty() && !is_own_executable(&name) {
                        result.push(ProcessEntry {
                            pid: entry.th32ProcessID,
                            name,
                        });
                    }
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    result
}

/// 自分自身の別プロセス (設定画面 `waypoint-settings.exe` 等) も候補から除く。
fn is_own_executable(name: &str) -> bool {
    name.eq_ignore_ascii_case("waypoint.exe") || name.eq_ignore_ascii_case("waypoint-settings.exe")
}

fn wchar_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 指定 PID のプロセスを終了する。
///
/// 管理者権限プロセス等 UIPI で `OpenProcess` / `TerminateProcess` が
/// 失敗しうるが、呼び出し側 (Quick Launch) は結果を問わず一覧を閉じる
/// だけで、失敗を例外化しない (FR-9.4 のネットワークパス確認と同じ扱い)。
pub fn kill(pid: u32) {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) else {
            return;
        };
        let _ = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
    }
}

/// 指定ウィンドウを所有するプロセスの実行ファイル名。
pub fn process_name_of(hwnd: HWND) -> Option<String> {
    let full = process_path_of(hwnd)?;
    // フルパスから実行ファイル名だけ取り出す
    Some(full.rsplit(['\\', '/']).next().unwrap_or(&full).to_string())
}

/// 指定ウィンドウを所有するプロセスの実行ファイルのフルパス。
pub fn process_path_of(hwnd: HWND) -> Option<String> {
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }

    unsafe {
        // 名前を読むだけなので最小の権限で開く。
        // これなら昇格プロセスでも成功することがある。
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        ok.ok()?;

        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_processes_excludes_the_calling_process_itself() {
        let processes = list_processes();
        let own_pid = std::process::id();
        assert!(!processes.iter().any(|process| process.pid == own_pid));
    }

    #[test]
    fn list_processes_excludes_waypoint_executables_by_name() {
        let processes = list_processes();
        assert!(
            !processes
                .iter()
                .any(|process| is_own_executable(&process.name))
        );
    }

    #[test]
    fn is_own_executable_is_case_insensitive() {
        assert!(is_own_executable("waypoint.exe"));
        assert!(is_own_executable("WAYPOINT.EXE"));
        assert!(is_own_executable("waypoint-settings.exe"));
        assert!(!is_own_executable("notepad.exe"));
    }

    #[test]
    fn kill_on_a_nonexistent_pid_does_not_panic() {
        kill(u32::MAX);
    }
}
