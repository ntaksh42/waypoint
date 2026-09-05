//! Azure DevOps 候補のタイトル検索。

/// タイトルと検索語の一致品質。小さいほど強い一致。
///
/// 完全な語句一致、全語の順不同一致、1 文字だけのタイプミスの順に扱う。
/// タイプミス許容は 4 文字以上の英数字トークン同士に限定し、短い入力で
/// 無関係な候補が増えるのを避ける。
pub(crate) fn match_quality(title: &str, query: &str) -> Option<u8> {
    let title = title.to_lowercase();
    let query = query.trim().to_lowercase();
    if query.is_empty() || title.contains(&query) {
        return Some(0);
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
    Some(if used_typo { 2 } else { 1 })
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
