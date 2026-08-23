//! ローマ字とかな/カタカナの相互変換・読みマッチのテスト。

use waypoint::romaji::{kana_name_matches, to_hiragana};

#[test]
fn converts_basic_syllables() {
    assert_eq!(to_hiragana("kaihatsu"), "かいはつ");
    assert_eq!(to_hiragana("purojekuto"), "ぷろじぇくと");
}

#[test]
fn converts_sokuon_double_consonant() {
    assert_eq!(to_hiragana("gakkou"), "がっこう");
}

#[test]
fn matches_hiragana_name_by_romaji() {
    assert!(kana_name_matches("かいはつ", "kaihatsu"));
    assert!(kana_name_matches("かいはつしつ", "kai"));
}

#[test]
fn matches_katakana_name_by_romaji() {
    assert!(kana_name_matches("プロジェクト", "purojekuto"));
    assert!(kana_name_matches("プロジェクト", "puro"));
}

#[test]
fn does_not_match_kanji_names() {
    assert!(!kana_name_matches("開発", "kaihatsu"));
}

#[test]
fn does_not_match_unrelated_reading() {
    assert!(!kana_name_matches("かいはつ", "shiryou"));
}
