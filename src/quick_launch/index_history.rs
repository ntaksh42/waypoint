//! `h ` の候補 (ブラウザの閲覧履歴 + Office で最近使ったファイル) を組み立てる。

use std::cmp::Reverse;
use std::collections::HashSet;

use crate::browser_history::Visit;
use crate::office_recent::Document;
use crate::url_key::duplicate_key;

use super::{Action, Entry};

pub(super) fn history_entries() -> Vec<Entry> {
    merge(crate::browser_history::scan(), crate::office_recent::scan())
}

/// 最終利用の新しい順に並べ、同じ URL・同じファイルは新しい方だけを残す。
fn merge(visits: Vec<Visit>, documents: Vec<Document>) -> Vec<Entry> {
    let mut timed: Vec<(i64, String, Entry)> = visits
        .into_iter()
        .map(|visit| {
            (
                visit.last_visit,
                duplicate_key(&visit.url),
                Entry {
                    azure: None,
                    name: visit.title,
                    breadcrumb: format!("{} History", visit.browser),
                    path: visit.url.clone(),
                    action: Action::OpenUrl(visit.url),
                    branch: None,
                },
            )
        })
        .collect();
    timed.extend(documents.into_iter().map(|document| {
        // URL はブラウザではなく Office のデスクトップアプリに開かせる
        // (`d.docs.live.net` はブラウザで開けない)
        let (key, action) = if crate::shell::is_web_url(&document.location) {
            (
                duplicate_key(&document.location),
                Action::OpenUrl(format!("{}:ofe|u|{}", document.scheme, document.location)),
            )
        } else {
            (
                document.location.to_lowercase(),
                Action::OpenWithDefaultHandler,
            )
        };
        (
            document.last_used,
            key,
            Entry {
                azure: None,
                name: document.name,
                breadcrumb: format!("{} Recent", document.app),
                path: document.location,
                action,
                branch: None,
            },
        )
    }));
    timed.sort_by_key(|(time, _, _)| Reverse(*time));
    let mut seen = HashSet::new();
    timed
        .into_iter()
        .filter_map(|(_, key, entry)| seen.insert(key).then_some(entry))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visit(url: &str, last_visit: i64) -> Visit {
        Visit {
            title: "page".into(),
            url: url.into(),
            browser: "Edge".into(),
            last_visit,
        }
    }

    fn document(location: &str, last_used: i64) -> Document {
        Document {
            name: "doc".into(),
            location: location.into(),
            app: "Excel",
            scheme: "ms-excel",
            last_used,
        }
    }

    #[test]
    fn documents_are_interleaved_with_visits_by_recency() {
        let entries = merge(
            vec![
                visit("https://a.example/", 30),
                visit("https://b.example/", 10),
            ],
            vec![document(r"C:\docs\Book.xlsx", 20)],
        );
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "https://a.example/",
                r"C:\docs\Book.xlsx",
                "https://b.example/"
            ]
        );
        assert_eq!(entries[1].breadcrumb, "Excel Recent");
        assert!(matches!(entries[1].action, Action::OpenWithDefaultHandler));
    }

    #[test]
    fn same_url_and_same_file_are_folded_to_the_latest() {
        let url = "https://contoso.sharepoint.com/sites/a/Book.xlsx";
        let entries = merge(
            vec![visit("http://CONTOSO.sharepoint.com/sites/a/Book.xlsx/", 5)],
            vec![
                document(url, 40),
                document(url, 50),
                document(r"C:\Docs\Book.xlsx", 10),
                document(r"c:\docs\book.xlsx", 20),
            ],
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, url);
        assert!(matches!(
            &entries[0].action,
            Action::OpenUrl(target) if target == &format!("ms-excel:ofe|u|{url}")
        ));
        assert_eq!(entries[1].path, r"c:\docs\book.xlsx");
    }
}
