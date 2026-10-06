//! Azure DevOps 候補のタイトル検索。

/// タイトルと検索語の一致品質。小さいほど強い一致。
///
/// 項目 ID の完全一致、完全な語句一致、全語の順不同一致、1 文字だけの
/// タイプミスの順に扱う。タイプミス許容は 4 文字以上の英数字トークン同士に
/// 限定し、短い入力で無関係な候補が増えるのを避ける。
pub(crate) fn match_quality(title: &str, query: &str) -> Option<u8> {
    TitleQuery::new(query).quality(title, None)
}

/// 検索語の前処理 (trim・小文字化・語分割・ID 判定) を済ませたもの。
/// 候補ごとに同じ前処理をやり直さないよう、検索 1 回につき 1 つ作る。
pub(crate) struct TitleQuery {
    empty: bool,
    /// 数字だけの入力 (`#` 付き可) の数字部分。
    id: Option<String>,
    /// `#<番号>` 形式 (完全一致だけを求める指定)。
    exact_id: bool,
    lower: String,
    terms: Vec<String>,
}

impl TitleQuery {
    pub(crate) fn new(query: &str) -> Self {
        let trimmed = query.trim();
        let lower = trimmed.to_lowercase();
        let terms = lower.split_whitespace().map(str::to_string).collect();
        Self {
            empty: trimmed.is_empty(),
            id: numeric_query(trimmed).map(str::to_string),
            exact_id: trimmed.starts_with('#'),
            lower,
            terms,
        }
    }

    /// `title_lower` は `title.to_lowercase()` 済みの値。呼び出し側が持って
    /// いれば渡して再確保を省く。
    pub(crate) fn quality(&self, title: &str, title_lower: Option<&str>) -> Option<u8> {
        if self.empty {
            return Some(QUALITY_PHRASE);
        }
        // 数字だけの入力は「PR 番号 / Work Item 番号の指定」とみなし、部分文字列
        // 一致ではなく先頭 ID との突き合わせで判定する。`contains` だけに任せると
        // `12345` が `PR 123456` / `PR 112345` / 本文に番号を含む PR と同点になり、
        // 以降の並びが実質キャッシュ順で決まって正解が先頭に来ない (実測)。
        if let Some(id) = self.id.as_deref() {
            if leading_id(title) == Some(id) {
                return Some(QUALITY_ID);
            }
            // `#<番号>` は完全一致だけを求める明示的な指定。部分一致は拾わない。
            if self.exact_id {
                return None;
            }
            // 入力途中の部分番号を拾うため部分一致は残すが、完全一致より下位に置く。
            // タイプミス許容は通さない — 数字の 1 文字違いは別の項目でしかない。
            return title.contains(id).then_some(QUALITY_TERMS);
        }

        let owned;
        let title = match title_lower {
            Some(lower) => lower,
            None => {
                owned = title.to_lowercase();
                &owned
            }
        };
        if title.contains(&self.lower) {
            return Some(QUALITY_PHRASE);
        }

        let mut used_typo = false;
        for term in &self.terms {
            if title.contains(term.as_str()) {
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

/// クエリが項目 ID の指定 (数字のみ、または `#<数字>`) か。そうなら数字部分を返す。
/// Azure DevOps の ID は int32 なので、収まらない桁数は ID とみなさない
/// (そのまま API へ渡すと全プロジェクトで HTTP 400 になる)。
pub(crate) fn numeric_query(query: &str) -> Option<&str> {
    let query = query.trim();
    let query = query.strip_prefix('#').unwrap_or(query);
    (query.bytes().all(|byte| byte.is_ascii_digit()) && query.parse::<i32>().is_ok())
        .then_some(query)
}

/// `#<番号>` 形式の完全一致指定なら数字部分を返す。
pub(crate) fn exact_id_query(query: &str) -> Option<&str> {
    numeric_query(query).filter(|_| query.trim().starts_with('#'))
}

/// 候補名の先頭に現れる項目 ID。PR は `PR 12345: <title>`、Work Item は
/// `12345: <title>` の形で組み立てられる (`convert.rs`)。Pipeline のように
/// 先頭が ID でない候補では `None`。
fn leading_id(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("PR ").unwrap_or(name);
    let id = rest.split(':').next()?.trim();
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then_some(id)
}

/// クエリが項目 ID の指定 (数字のみ、または `#<数字>`) か。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_query_rejects_ids_outside_int32() {
        assert_eq!(numeric_query("#2147483647"), Some("2147483647"));
        assert_eq!(numeric_query("2147483648"), None);
        assert_eq!(numeric_query("#123456789012"), None);
    }

    #[test]
    fn exact_id_query_requires_the_hash() {
        assert_eq!(exact_id_query(" #123 "), Some("123"));
        assert_eq!(exact_id_query("123"), None);
        assert_eq!(exact_id_query("#12a"), None);
    }

    #[test]
    fn precomputed_lowercase_title_gives_the_same_quality() {
        let title = "PR 123: Fix Cache Handling";
        for query in [
            "",
            "cache fix",
            "fix cahce",
            "zzzz",
            "123",
            "#123",
            "#12",
            "xyz",
        ] {
            let prepared = TitleQuery::new(query);
            assert_eq!(
                prepared.quality(title, Some(&title.to_lowercase())),
                prepared.quality(title, None),
                "{query}"
            );
            assert_eq!(prepared.quality(title, None), match_quality(title, query));
        }
        assert_eq!(match_quality(title, "fix cahce"), Some(QUALITY_TYPO));
        assert_eq!(match_quality(title, "cache fix"), Some(QUALITY_TERMS));
    }
}
