//! メニュー項目のラベル整形ロジック。

use windows::Win32::UI::Shell::{
    SHSTOCKICONID, SIID_DOCASSOC, SIID_DOCNOASSOC, SIID_FOLDER, SIID_FOLDEROPEN,
};

/// `&1  名前` の装飾を描画用の文字列へ直す。
///
/// オーナードローでは `&` を自分で解釈しないので、
/// 単独の `&` は落とし、`&&` はリテラルの `&` に戻す。
pub(crate) fn strip_accelerator(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
        } else if chars.clone().next() == Some('&') {
            chars.next();
            out.push('&');
        }
    }
    out
}

/// Recent / Frequent の各サブメニューに割り当てるアイコン。
///
/// 「最近」は時計 (履歴)、「よく使う」は星 (お気に入り) に相当する
/// Windows 標準アイコンが無いため、フォルダ / ファイルの区別に加えて
/// 開いた・閉じたで最近とよく使うを描き分ける。
pub(crate) fn path_menu_icon(name: &str) -> SHSTOCKICONID {
    match (name.starts_with("Recent"), name.ends_with("Folders")) {
        (true, true) => SIID_FOLDEROPEN,
        (false, true) => SIID_FOLDER,
        (true, false) => SIID_DOCASSOC,
        (false, false) => SIID_DOCNOASSOC,
    }
}

/// 上位 9 件に `&1 ` のようなアクセラレータを前置する (FR-2.4) 。
pub(crate) fn decorate(name: &str, numeric: bool, accel: usize) -> String {
    // 項目名の & はリテラルの & として出すためエスケープする。
    // 数字アクセラレータを前置する分岐でこれを忘れると、"R&D" のような
    // 項目名で strip_accelerator が & をアクセラレータ区切りと誤解釈し、
    // 文字が欠けて表示される (実測で確認済み)
    let escaped = name.replace('&', "&&");
    if numeric && (1..=9).contains(&accel) {
        format!("&{accel}  {escaped}")
    } else {
        escaped
    }
}

#[cfg(test)]
mod tests {
    use super::{decorate, path_menu_icon, strip_accelerator};
    use crate::git::with_branch;

    /// Recent / Frequent × フォルダ / ファイルの 4 つが別アイコンになること。
    /// 同じだと In the Works の中で見分けが付かない。
    #[test]
    fn path_menu_icons_are_distinct() {
        let ids: Vec<i32> = [
            "Recent Folders",
            "Frequent Folders",
            "Recent Files",
            "Frequent Files",
        ]
        .iter()
        .map(|name| path_menu_icon(name).0)
        .collect();

        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 4, "同じアイコンが割り当てられている: {ids:?}");
    }

    #[test]
    fn branch_survives_accelerator_decoration() {
        let label = with_branch("waypoint", Some("feature/x"));
        assert_eq!(decorate(&label, true, 1), "&1  waypoint  [feature/x]");
    }

    /// 項目名の & はエスケープされる。アクセラレータ無効時も同じ規則。
    #[test]
    fn ampersand_in_name_is_escaped_without_accelerator() {
        let label = with_branch("R&D", Some("main"));
        assert_eq!(decorate(&label, false, 1), "R&&D  [main]");
    }

    /// 数字アクセラレータを前置する上位 9 件でも & はエスケープされる。
    /// 抜けると strip_accelerator が項目名中の & を区切りと誤解釈し、
    /// 文字が欠けて表示される (実測で確認済み)。
    #[test]
    fn ampersand_in_name_is_escaped_with_numeric_accelerator() {
        assert_eq!(decorate("R&D Docs", true, 3), "&3  R&&D Docs");
    }

    /// オーナードローでは & を自分で解釈しないので描画前に落とす。
    #[test]
    fn accelerator_marker_is_removed_for_drawing() {
        assert_eq!(strip_accelerator("&1  Downloads"), "1  Downloads");
    }

    /// エスケープされた && はリテラルの & に戻す。
    #[test]
    fn escaped_ampersand_becomes_literal() {
        assert_eq!(strip_accelerator("R&&D"), "R&D");
        assert_eq!(strip_accelerator("&1  R&&D"), "1  R&D");
    }
}
