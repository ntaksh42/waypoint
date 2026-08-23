//! ローマ字入力とひらがな/カタカナの相互変換。
//!
//! Quick Launch のクエリ (ローマ字) を項目名 (かな/カナ) に音でマッチさせるため。
//! 対象はかな/カナのみで構成される項目名に限る。漢字は読みが一意に定まらず
//! 自動変換できないため対象外 (AGENTS.md の合意事項)。

/// ローマ字の綴りとひらがなの対応表。長い綴りから先に試すため、
/// 呼び出し側は最長一致で探索すること (`to_hiragana`)。
const ROMAJI_TABLE: &[(&str, &str)] = &[
    ("kya", "きゃ"),
    ("kyu", "きゅ"),
    ("kyo", "きょ"),
    ("sha", "しゃ"),
    ("shu", "しゅ"),
    ("sho", "しょ"),
    ("sya", "しゃ"),
    ("syu", "しゅ"),
    ("syo", "しょ"),
    ("cha", "ちゃ"),
    ("chu", "ちゅ"),
    ("cho", "ちょ"),
    ("tya", "ちゃ"),
    ("tyu", "ちゅ"),
    ("tyo", "ちょ"),
    ("nya", "にゃ"),
    ("nyu", "にゅ"),
    ("nyo", "にょ"),
    ("hya", "ひゃ"),
    ("hyu", "ひゅ"),
    ("hyo", "ひょ"),
    ("mya", "みゃ"),
    ("myu", "みゅ"),
    ("myo", "みょ"),
    ("rya", "りゃ"),
    ("ryu", "りゅ"),
    ("ryo", "りょ"),
    ("gya", "ぎゃ"),
    ("gyu", "ぎゅ"),
    ("gyo", "ぎょ"),
    ("ja", "じゃ"),
    ("ju", "じゅ"),
    ("jo", "じょ"),
    ("jya", "じゃ"),
    ("jyu", "じゅ"),
    ("jyo", "じょ"),
    ("zya", "じゃ"),
    ("zyu", "じゅ"),
    ("zyo", "じょ"),
    ("bya", "びゃ"),
    ("byu", "びゅ"),
    ("byo", "びょ"),
    ("pya", "ぴゃ"),
    ("pyu", "ぴゅ"),
    ("pyo", "ぴょ"),
    // 外来語表記で使う拡張音 (「てぃ」「ふぁ」等は thi / fa のように別綴りで区別)
    ("je", "じぇ"),
    ("she", "しぇ"),
    ("che", "ちぇ"),
    ("thi", "てぃ"),
    ("dhi", "でぃ"),
    ("twu", "とぅ"),
    ("dwu", "どぅ"),
    ("fa", "ふぁ"),
    ("fi", "ふぃ"),
    ("fe", "ふぇ"),
    ("fo", "ふぉ"),
    ("wi", "うぃ"),
    ("we", "うぇ"),
    ("shi", "し"),
    ("chi", "ち"),
    ("tsu", "つ"),
    ("fu", "ふ"),
    ("ji", "じ"),
    ("zi", "じ"),
    ("wo", "を"),
    ("nn", "ん"),
    ("ka", "か"),
    ("ki", "き"),
    ("ku", "く"),
    ("ke", "け"),
    ("ko", "こ"),
    ("sa", "さ"),
    ("si", "し"),
    ("su", "す"),
    ("se", "せ"),
    ("so", "そ"),
    ("ta", "た"),
    ("ti", "ち"),
    ("tu", "つ"),
    ("te", "て"),
    ("to", "と"),
    ("di", "ぢ"),
    ("du", "づ"),
    ("na", "な"),
    ("ni", "に"),
    ("nu", "ぬ"),
    ("ne", "ね"),
    ("no", "の"),
    ("ha", "は"),
    ("hi", "ひ"),
    ("hu", "ふ"),
    ("he", "へ"),
    ("ho", "ほ"),
    ("ma", "ま"),
    ("mi", "み"),
    ("mu", "む"),
    ("me", "め"),
    ("mo", "も"),
    ("ya", "や"),
    ("yu", "ゆ"),
    ("yo", "よ"),
    ("ra", "ら"),
    ("ri", "り"),
    ("ru", "る"),
    ("re", "れ"),
    ("ro", "ろ"),
    ("wa", "わ"),
    ("ga", "が"),
    ("gi", "ぎ"),
    ("gu", "ぐ"),
    ("ge", "げ"),
    ("go", "ご"),
    ("za", "ざ"),
    ("zu", "ず"),
    ("ze", "ぜ"),
    ("zo", "ぞ"),
    ("da", "だ"),
    ("de", "で"),
    ("do", "ど"),
    ("ba", "ば"),
    ("bi", "び"),
    ("bu", "ぶ"),
    ("be", "べ"),
    ("bo", "ぼ"),
    ("pa", "ぱ"),
    ("pi", "ぴ"),
    ("pu", "ぷ"),
    ("pe", "ぺ"),
    ("po", "ぽ"),
    ("a", "あ"),
    ("i", "い"),
    ("u", "う"),
    ("e", "え"),
    ("o", "お"),
    ("n", "ん"),
    ("-", "ー"),
];

/// ローマ字文字列をひらがなへ変換する。変換しきれずローマ字のまま残った
/// 部分 (促音の未確定な子音重複や途中入力) はそのまま素通りさせる。
/// 完全な IME 実装ではなく、検索クエリを大まかに正規化できれば十分。
pub fn to_hiragana(input: &str) -> String {
    let chars: Vec<char> = input.to_lowercase().chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        // 促音: 子音の連続 (nを除く) は「っ」+ 次の綴りへ
        if i + 1 < chars.len()
            && chars[i] == chars[i + 1]
            && !"aeioun".contains(chars[i])
            && chars[i].is_ascii_alphabetic()
        {
            out.push('っ');
            i += 1;
            continue;
        }
        let remaining: String = chars[i..].iter().collect();
        let matched = ROMAJI_TABLE
            .iter()
            .filter(|(romaji, _)| remaining.starts_with(romaji))
            .max_by_key(|(romaji, _)| romaji.len());
        match matched {
            Some((romaji, kana)) => {
                out.push_str(kana);
                i += romaji.chars().count();
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// カタカナをひらがなへ正規化する (Unicode 上はコードポイントが
/// 0x60 ずれているだけなので単純シフトで変換できる)。
fn katakana_to_hiragana(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            'ァ'..='ヶ' => char::from_u32(ch as u32 - 0x60).unwrap_or(ch),
            other => other,
        })
        .collect()
}

/// `name` がひらがな/カタカナ (と長音符・中黒・空白程度) のみで構成されて
/// いるか。漢字や英数字が混じる名前はローマ字読みマッチの対象外にする。
fn is_kana_only(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| matches!(ch, 'ぁ'..='ゖ' | 'ァ'..='ヶ' | 'ー' | '・' | ' ' | '　'))
}

/// 項目名がかな/カナのみのとき、ローマ字クエリを読みに変換して一致するか判定する。
pub fn kana_name_matches(name: &str, term: &str) -> bool {
    if !is_kana_only(name) || term.is_empty() {
        return false;
    }
    // ローマ字綴りに現れない文字 (数字・記号) は音マッチの対象外
    if !term.chars().all(|ch| ch.is_ascii_alphabetic() || ch == '-') {
        return false;
    }
    let name_hiragana = katakana_to_hiragana(name);
    let term_hiragana = to_hiragana(term);
    name_hiragana.contains(&term_hiragana)
}
