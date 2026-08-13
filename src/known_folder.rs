//! 既知フォルダ (KNOWNFOLDERID) の名前からパスを解決する。

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Favorites,
    FOLDERID_LocalAppData, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile,
    FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_Public, FOLDERID_RecycleBinFolder,
    FOLDERID_RoamingAppData, FOLDERID_Videos, FOLDERID_Windows, KNOWN_FOLDER_FLAG,
    SHGetKnownFolderPath,
};
use windows::core::GUID;

/// 設定ファイルで使える識別名の一覧。管理画面の選択肢にも使う。
pub const NAMES: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Music",
    "Pictures",
    "Videos",
    "Profile",
    "Favorites",
    "Public",
    "RoamingAppData",
    "LocalAppData",
    "ProgramData",
    "ProgramFiles",
    "Windows",
    "RecycleBin",
];

fn guid_for(name: &str) -> Option<GUID> {
    // 大文字小文字は問わない
    let g = match name.to_ascii_lowercase().as_str() {
        "desktop" => FOLDERID_Desktop,
        "documents" => FOLDERID_Documents,
        "downloads" => FOLDERID_Downloads,
        "music" => FOLDERID_Music,
        "pictures" => FOLDERID_Pictures,
        "videos" => FOLDERID_Videos,
        "profile" => FOLDERID_Profile,
        "favorites" => FOLDERID_Favorites,
        "public" => FOLDERID_Public,
        "roamingappdata" | "appdata" => FOLDERID_RoamingAppData,
        "localappdata" => FOLDERID_LocalAppData,
        "programdata" => FOLDERID_ProgramData,
        "programfiles" => FOLDERID_ProgramFiles,
        "windows" => FOLDERID_Windows,
        "recyclebin" => FOLDERID_RecycleBinFolder,
        _ => return None,
    };
    Some(g)
}

/// 識別名を絶対パスに解決する。未知の名前や解決失敗では None。
pub fn resolve(name: &str) -> Option<String> {
    let guid = guid_for(name)?;
    unsafe {
        let pwstr = SHGetKnownFolderPath(&guid, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let result = pwstr.to_string().ok();
        // SHGetKnownFolderPath の戻り値は呼び出し側が解放する
        CoTaskMemFree(Some(pwstr.0 as *const _));
        result
    }
}
