//! Azure DevOps 候補のタイトル検索。

/// タイトルと検索語の一致品質。小さいほど強い一致。
///
/// 項目 ID の完全一致、完全な語句一致、全語の順不同一致、1 文字だけの
/// タイプミスの順に扱う。タイプミス許容は 4 文字以上の英数字トークン同士に
/// 限定し、短い入力で無関係な候補が増えるのを避ける。
pub(crate) fn match_quality(title: &str, query: &str) -> Option<u8> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Some(QUALITY_PHRASE);
    }
    // 数字だけの入力は「PR 番号 / Work Item 番号の指定」とみなし、部分文字列
    // 一致ではなく先頭 ID との突き合わせで判定する。`contains` だけに任せると
    // `12345` が `PR 123456` / `PR 112345` / 本文に番号を含む PR と同点になり、
    // 以降の並びが実質キャッシュ順で決まって正解が先頭に来ない (実測)。
    if let Some(id) = numeric_query(trimmed) {
        if leading_id(title) == Some(id) {
            return Some(QUALITY_ID);
        }
        // 入力途中の部分番号を拾うため部分一致は残すが、完全一致より下位に置く。
        // タイプミス許容は通さない — 数字の 1 文字違いは別の項目でしかない。
        return title.contains(id).then_some(QUALITY_TERMS);
    }

    let title = title.to_lowercase();
    let query = trimmed.to_lowercase();
    if title.contains(&query) {
        return Some(QUALITY_PHRASE);
    }

    let mut used_typo = false;
    for term in query.split_whitespace() {
        if title.contains(term) {
            continue;
        }
        if term.len() < 4
            || !term.is_ascii()
            || !title
                .split(|character: char| !character.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .any(|word| word.is_ascii() && one_edit_apart(word.as_bytes(), term.as_bytes()))
        {
            return None;
        }
        used_typo = true;
    }
    Some(if used_typo {
        QUALITY_TYPO
    } else {
        QUALITY_TERMS
    })
}

/// 項目 ID (`PR 12345` の `12345`) の完全一致。
pub(crate) const QUALITY_ID: u8 = 0;
/// 語句がそのまま含まれる。
const QUALITY_PHRASE: u8 = 1;
/// 全語が順不同で含まれる (数字クエリの部分一致もここ)。
const QUALITY_TERMS: u8 = 2;
/// 1 文字のタイプミスを許して一致した。
const QUALITY_TYPO: u8 = 3;
/// タイトル一致では拾えず、breadcrumb / URL 側の一致で残った候補
/// (`quick_launch::azure_search`)。
pub(crate) const QUALITY_OTHER: u8 = 4;

/// クエリが項目 ID の指定 (数字のみ) か。そうなら数字部分を返す。
fn numeric_query(query: &str) -> Option<&str> {
    let query = query.trim();
    (!query.is_empty() && query.bytes().all(|byte| byte.is_ascii_digit())).then_some(query)
}

/// 候補名の先頭に現れる項目 ID。PR は `PR 12345: <title>`、Work Item は
/// `12345: <title>` の形で組み立てられる (`convert.rs`)。Pipeline のように
/// 先頭が ID でない候補では `None`。
fn leading_id(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("PR ").unwrap_or(name);
    let id = rest.split(':').next()?.trim();
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then_some(id)
}

/// クエリが項目 ID の指定 (数字のみ) か。
pub(crate) fn is_item_id_query(query: &str) -> bool {
    numeric_query(query).is_some()
}

/// 数字だけのクエリに対し、候補名の先頭 ID が完全一致するか。
/// キャッシュ検索が ID 指定を取りこぼしたかの判定に使う。
pub(crate) fn matches_item_id(name: &str, query: &str) -> bool {
    numeric_query(query).is_some_and(|id| leading_id(name) == Some(id))
}

/// ASCII 文字列が挿入・削除・置換・隣接入れ替えのいずれか 1 回以内で一致するか。
fn one_edit_apart(left: &[u8], right: &[u8]) -> bool {
    if left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    if left.len() == right.len() {
        let mut differences = left
            .iter()
            .zip(right)
            .enumerate()
            .filter_map(|(index, (left, right))| (left != right).then_some(index));
        let Some(first) = differences.next() else {
            return true;
        };
        let Some(second) = differences.next() else {
            return true;
        };
        return differences.next().is_none()
            && second == first + 1
            && left[first] == right[second]
            && left[second] == right[first];
    }

    let (longer, shorter) = if left.len() > right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let mut longer_index = 0;
    let mut shorter_index = 0;
    let mut skipped = false;
    while longer_index < longer.len() && shorter_index < shorter.len() {
        if longer[longer_index] == shorter[shorter_index] {
            shorter_index += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
        }
        longer_index += 1;
    }
    true
}
