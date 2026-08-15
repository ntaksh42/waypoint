//! Everything (voidtools) との IPC 連携 (FR-9.16)。
//!
//! Everything.dll は使わず、公式 SDK の `everything_ipc.h` が定義する
//! `WM_COPYDATA` プロトコルを直接叩く。waypoint 自身はファイルの索引を
//! 一切持たず、常駐中の Everything へ問い合わせるだけ。Everything が
//! 見つからない・未応答でもエラーにせず 0 件として扱う。
//!
//! クエリは非同期。`query()` は要求を送るだけで、結果は `reply_hwnd` へ
//! 後から `WM_COPYDATA` で届く。呼び出し側 (`quick_launch_window.rs`) が
//! 自分のウィンドウハンドルを `reply_hwnd` にして受け取る。

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW};
use windows::core::w;

/// Everything 常駐プロセスが常に持つ通知ウィンドウのクラス名。
const IPC_WNDCLASS: windows::core::PCWSTR = w!("EVERYTHING_TASKBAR_NOTIFICATION");

/// `WM_COPYDATA` の `dwData`。Unicode クエリ送信を表す。
const IPC_COPYDATA_QUERYW: usize = 2;

/// 検索結果全件を返す指定値。
const IPC_ALLRESULTS: u32 = 0xFFFFFFFF;

/// アイテムがフォルダであることを示すフラグ (`EVERYTHING_IPC_ITEMW::flags`)。
const IPC_FOLDER: u32 = 0x1;

/// Quick Launch が Everything から受け取った 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EverythingResult {
    pub name: String,
    pub path: String,
    pub is_folder: bool,
}

/// Everything の通知ウィンドウを探す。見つからなければ未起動。
fn find_everything_window() -> Option<HWND> {
    unsafe { FindWindowW(IPC_WNDCLASS, None).ok() }
}

/// Everything が常駐しているか。設定 UI のヒント表示などに使う。
pub fn is_running() -> bool {
    find_everything_window().is_some()
}

/// 検索クエリを送る。結果は `reply_hwnd` へ `WM_COPYDATA`
/// (`dwData == reply_message`) で非同期に届く。
///
/// Everything は 1 ウィンドウにつき 1 クエリしか処理しない。新しいクエリを
/// 送ると前のクエリは黙ってキャンセルされる (SDK のコメント通り)。
/// 送信に失敗した場合 (Everything 未起動など) は false を返す。
pub fn query(reply_hwnd: HWND, reply_message: u32, text: &str, max_results: u32) -> bool {
    let Some(everything) = find_everything_window() else {
        return false;
    };

    // EVERYTHING_IPC_QUERYW は pack(1) の可変長構造体:
    // reply_hwnd(4) + reply_copydata_message(4) + search_flags(4) + offset(4)
    // + max_results(4) + 検索文字列 (UTF-16, null 終端)
    let mut search: Vec<u16> = text.encode_utf16().collect();
    search.push(0);

    let mut buffer = Vec::with_capacity(20 + search.len() * 2);
    buffer.extend_from_slice(&(reply_hwnd.0 as u32).to_le_bytes());
    buffer.extend_from_slice(&reply_message.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes()); // search_flags: 既定 (大文字小文字を区別しない)
    buffer.extend_from_slice(&0u32.to_le_bytes()); // offset
    buffer.extend_from_slice(&max_results.to_le_bytes());
    for unit in &search {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }

    let mut copy_data = COPYDATASTRUCT {
        dwData: IPC_COPYDATA_QUERYW,
        cbData: buffer.len() as u32,
        lpData: buffer.as_mut_ptr().cast(),
    };

    unsafe {
        SendMessageW(
            everything,
            windows::Win32::UI::WindowsAndMessaging::WM_COPYDATA,
            None,
            Some(LPARAM(std::ptr::from_mut(&mut copy_data) as isize)),
        );
    }
    true
}

/// `WM_COPYDATA` で届いた `EVERYTHING_IPC_LISTW` をパースする。
///
/// レイアウトは pack(1) の固定 7 DWORD ヘッダ (28 バイト) の後に
/// `EVERYTHING_IPC_ITEMW` (flags/filename_offset/path_offset の 3 DWORD =
/// 12 バイト) が `numitems` 個続き、その後ろに UTF-16 文字列プールが並ぶ。
/// オフセットは全てリスト構造体の先頭からのバイト位置。
/// 壊れたデータを渡されても panic せず空を返す。
pub fn parse_results(data: &[u8]) -> Vec<EverythingResult> {
    const HEADER_LEN: usize = 28;
    const ITEM_LEN: usize = 12;

    if data.len() < HEADER_LEN {
        return Vec::new();
    }
    let numitems = read_u32(data, 20) as usize;

    let mut results = Vec::with_capacity(numitems);
    for index in 0..numitems {
        let item_offset = HEADER_LEN + index * ITEM_LEN;
        if item_offset + ITEM_LEN > data.len() {
            break;
        }
        let flags = read_u32(data, item_offset);
        let filename_offset = read_u32(data, item_offset + 4) as usize;
        let path_offset = read_u32(data, item_offset + 8) as usize;

        let Some(name) = read_wide_str(data, filename_offset) else {
            continue;
        };
        let Some(dir) = read_wide_str(data, path_offset) else {
            continue;
        };

        let path = if dir.is_empty() {
            name.clone()
        } else {
            format!("{dir}\\{name}")
        };
        results.push(EverythingResult {
            name,
            path,
            is_folder: flags & IPC_FOLDER != 0,
        });
    }
    results
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    data.get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

/// 指定オフセットから null 終端の UTF-16 文字列を読む。
fn read_wide_str(data: &[u8], offset: usize) -> Option<String> {
    if offset >= data.len() || !offset.is_multiple_of(2) {
        return None;
    }
    let units: &[u8] = &data[offset..];
    let mut wide = Vec::new();
    let mut cursor = 0;
    while cursor + 1 < units.len() {
        let unit = u16::from_le_bytes([units[cursor], units[cursor + 1]]);
        if unit == 0 {
            break;
        }
        wide.push(unit);
        cursor += 2;
    }
    Some(String::from_utf16_lossy(&wide))
}

/// `EVERYTHING_IPC_ALLRESULTS` の値。テストや呼び出し側から参照する。
pub const ALL_RESULTS: u32 = IPC_ALLRESULTS;

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用に `EVERYTHING_IPC_LISTW` 相当のバイト列を組み立てる。
    fn build_list(items: &[(u32, &str, &str)]) -> Vec<u8> {
        let mut pool: Vec<u8> = Vec::new();
        let mut entries = Vec::new();
        for (flags, name, dir) in items {
            let name_offset = 28 + items.len() * 12 + pool.len();
            for unit in name.encode_utf16().chain(std::iter::once(0)) {
                pool.extend_from_slice(&unit.to_le_bytes());
            }
            let path_offset = 28 + items.len() * 12 + pool.len();
            for unit in dir.encode_utf16().chain(std::iter::once(0)) {
                pool.extend_from_slice(&unit.to_le_bytes());
            }
            entries.push((*flags, name_offset as u32, path_offset as u32));
        }

        let mut buffer = Vec::new();
        let numitems = items.len() as u32;
        buffer.extend_from_slice(&0u32.to_le_bytes()); // totfolders
        buffer.extend_from_slice(&0u32.to_le_bytes()); // totfiles
        buffer.extend_from_slice(&numitems.to_le_bytes()); // totitems
        buffer.extend_from_slice(&0u32.to_le_bytes()); // numfolders
        buffer.extend_from_slice(&0u32.to_le_bytes()); // numfiles
        buffer.extend_from_slice(&numitems.to_le_bytes()); // numitems
        buffer.extend_from_slice(&0u32.to_le_bytes()); // offset
        for (flags, name_offset, path_offset) in entries {
            buffer.extend_from_slice(&flags.to_le_bytes());
            buffer.extend_from_slice(&name_offset.to_le_bytes());
            buffer.extend_from_slice(&path_offset.to_le_bytes());
        }
        buffer.extend_from_slice(&pool);
        buffer
    }

    #[test]
    fn parses_files_and_folders_with_their_directory() {
        let data = build_list(&[
            (IPC_FOLDER, "src", r"E:\waypoint"),
            (0, "Cargo.toml", r"E:\waypoint"),
        ]);
        let results = parse_results(&data);
        assert_eq!(
            results,
            vec![
                EverythingResult {
                    name: "src".into(),
                    path: r"E:\waypoint\src".into(),
                    is_folder: true,
                },
                EverythingResult {
                    name: "Cargo.toml".into(),
                    path: r"E:\waypoint\Cargo.toml".into(),
                    is_folder: false,
                },
            ]
        );
    }

    #[test]
    fn empty_list_yields_no_results() {
        let data = build_list(&[]);
        assert!(parse_results(&data).is_empty());
    }

    #[test]
    fn truncated_data_does_not_panic() {
        assert!(parse_results(&[1, 2, 3]).is_empty());
        assert!(parse_results(&[]).is_empty());
    }

    #[test]
    fn root_drive_has_no_directory_separator_duplicated() {
        let data = build_list(&[(IPC_FOLDER, "E:", "")]);
        let results = parse_results(&data);
        assert_eq!(results[0].path, "E:");
    }
}
