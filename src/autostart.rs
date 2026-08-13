//! Windows 起動時の自動実行 (FR-8.4) 。
//!
//! HKCU の Run キーに実行ファイルのパスを書く。管理者権限は不要。

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("waypoint");

fn open_run_key(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, access, &mut key) };
    rc.is_ok().then_some(key)
}

/// 自動起動が有効か。
pub fn is_enabled() -> bool {
    let Some(key) = open_run_key(KEY_READ) else {
        return false;
    };
    let mut size = 0u32;
    let rc = unsafe { RegQueryValueExW(key, VALUE_NAME, None, None, None, Some(&mut size)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    rc.is_ok()
}

/// 自動起動を設定する。
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    let Some(key) = open_run_key(KEY_WRITE) else {
        return Err(std::io::Error::other("Run キーを開けない"));
    };

    let result = if enabled {
        let exe = std::env::current_exe()?;
        // パスに空白が含まれても解釈されるよう引用符で囲む
        let quoted = HSTRING::from(format!("\"{}\"", exe.display()));
        let bytes = unsafe {
            std::slice::from_raw_parts(
                quoted.as_ptr() as *const u8,
                // 終端 NUL を含めた長さ
                (quoted.len() + 1) * 2,
            )
        };
        unsafe { RegSetValueExW(key, VALUE_NAME, None, REG_SZ, Some(bytes)) }
    } else {
        let rc = unsafe { RegDeleteValueW(key, VALUE_NAME) };
        // 元から無い場合も成功扱いにする
        if rc.is_err() && is_enabled() {
            rc
        } else {
            windows::Win32::Foundation::WIN32_ERROR(0)
        }
    };

    unsafe {
        let _ = RegCloseKey(key);
    }
    result.ok().map_err(std::io::Error::other)
}
