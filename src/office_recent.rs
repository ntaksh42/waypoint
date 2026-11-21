//! Office (Excel / PowerPoint / Word) で最近使ったファイルを読む (`h ` プレフィックス)。
//!
//! Office は MRU を HKCU のレジストリに持つ。`Item N` の値は
//! `[F00000000][T01DD0F3051BB9E40][O00000000]*<パスまたは URL>` の形で、
//! `T` が最終利用の FILETIME (16 進)。Quick Launch の索引構築時だけ読む。

use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, RegCloseKey, RegEnumKeyExW, RegEnumValueW,
    RegOpenKeyExW,
};
use windows::core::{HSTRING, PWSTR};

/// 表示名と、URL を開かせる Office の URI スキーム。
const APPS: [(&str, &str); 3] = [
    ("Excel", "ms-excel"),
    ("PowerPoint", "ms-powerpoint"),
    ("Word", "ms-word"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub name: String,
    /// ローカルパス、または SharePoint / OneDrive の URL。
    pub location: String,
    pub app: &'static str,
    pub scheme: &'static str,
    /// 最終利用。Chromium の `last_visit_time` と同じ単位 (1601 年からのマイクロ秒)。
    pub last_used: i64,
}

/// 全アプリの MRU を集める。順序と重複は呼び出し側が整える。
pub fn scan() -> Vec<Document> {
    let mut documents = Vec::new();
    for (app, scheme) in APPS {
        let base = format!(r"Software\Microsoft\Office\16.0\{app}");
        // サインイン中は `User MRU\<アカウント>` 側に、未サインインなら直下に入る
        let user = format!(r"{base}\User MRU");
        let mut keys = vec![format!(r"{base}\File MRU")];
        keys.extend(
            subkeys(&user)
                .into_iter()
                .map(|account| format!(r"{user}\{account}\File MRU")),
        );
        for key in keys {
            for value in item_values(&key) {
                if let Some((last_used, location)) = parse_item(&value) {
                    documents.push(Document {
                        name: display_name(location),
                        location: location.to_string(),
                        app,
                        scheme,
                        last_used,
                    });
                }
            }
        }
    }
    documents
}

/// `Item N` の値から最終利用とパス (URL) を取り出す。
fn parse_item(value: &str) -> Option<(i64, &str)> {
    let (head, location) = value.split_once('*')?;
    let hex = head.split_once("[T")?.1.split_once(']')?.0;
    let filetime = u64::from_str_radix(hex, 16).ok()?;
    // FILETIME は 100ns 単位。10 で割ると Chromium の時刻と直接比べられる
    (!location.is_empty()).then_some(((filetime / 10) as i64, location))
}

fn display_name(location: &str) -> String {
    let last = location
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(location);
    if crate::shell::is_web_url(location) {
        crate::editor_workspaces::decode(last).unwrap_or_else(|| last.to_string())
    } else {
        last.to_string()
    }
}

struct Key(HKEY);

impl Key {
    fn open(path: &str) -> Option<Self> {
        let mut key = HKEY::default();
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(path),
                None,
                KEY_READ,
                &mut key,
            )
        };
        rc.is_ok().then_some(Self(key))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn subkeys(path: &str) -> Vec<String> {
    let Some(key) = Key::open(path) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for index in 0.. {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        let rc = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if rc != ERROR_SUCCESS {
            break;
        }
        names.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    names
}

/// キー直下の `Item N` (文字列値) を集める。`Item Metadata N` は除く。
fn item_values(path: &str) -> Vec<String> {
    let Some(key) = Key::open(path) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    let mut data = vec![0u16; 4096];
    for index in 0.. {
        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        let mut kind = 0u32;
        let mut bytes = (data.len() * 2) as u32;
        let rc = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        // バッファに収まらない値 (長いメタデータ) は飛ばして次へ進む
        if rc == ERROR_MORE_DATA {
            continue;
        }
        if rc != ERROR_SUCCESS {
            break;
        }
        let name = String::from_utf16_lossy(&name[..name_len as usize]);
        if kind != REG_SZ.0 || !name.starts_with("Item ") || name.starts_with("Item Metadata") {
            continue;
        }
        let text = String::from_utf16_lossy(&data[..bytes as usize / 2]);
        values.push(text.trim_end_matches('\0').to_string());
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_value_yields_chromium_time_and_location() {
        let (last_used, location) =
            parse_item(r"[F00000000][T01DD0F3051BB9E40][O00000000]*C:\docs\Book 1.xlsx").unwrap();
        assert_eq!(last_used, 0x01DD0F3051BB9E40 / 10);
        assert_eq!(location, r"C:\docs\Book 1.xlsx");
        assert!(parse_item("[F00000000][T01DD0F3051BB9E40]*").is_none());
        assert!(parse_item("no marker").is_none());
        assert!(parse_item("[F0][Txyz]*C:\\a.xlsx").is_none());
    }

    #[test]
    fn name_is_the_last_segment_and_urls_are_decoded() {
        assert_eq!(display_name(r"C:\docs\100%25.xlsx"), "100%25.xlsx");
        assert_eq!(
            display_name("https://contoso.sharepoint.com/sites/a/%E8%B3%87%E6%96%99%20v2.pptx"),
            "資料 v2.pptx"
        );
    }
}
