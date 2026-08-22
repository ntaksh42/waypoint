//! Chrome / Edge のブックマークを読む。
//!
//! `Bookmarks` は Chromium 系ブラウザ共通の JSON 形式なので、
//! パーサーは両ブラウザで共用する。プロファイルは既定 (`Default`) のみ扱う。

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub name: String,
    pub url: String,
    /// フォルダ階層 (例: "お気に入りバー > 仕事")。
    pub breadcrumb: String,
}

#[derive(Deserialize)]
struct BookmarksFile {
    roots: Roots,
}

#[derive(Deserialize)]
struct Roots {
    bookmark_bar: Option<Node>,
    other: Option<Node>,
    synced: Option<Node>,
}

#[derive(Deserialize)]
struct Node {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
    #[serde(default)]
    children: Vec<Node>,
}

/// インストール済みの Chrome / Edge から既定プロファイルのブックマークを集める。
pub fn scan() -> Vec<Bookmark> {
    let mut bookmarks = Vec::new();
    for path in profile_paths() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let file = match serde_json::from_str::<BookmarksFile>(&text) {
            Ok(file) => file,
            Err(e) => {
                // ブラウザが書き込み中に読むと壊れた JSON を掴むことがある。
                // 無言でスキップすると「ブックマークが急に消えた」ように
                // 見えるだけで原因が追えないため、診断用に 1 行残す
                crate::panic_log::record(&format!(
                    "bookmarks: failed to parse {}: {e}",
                    path.display()
                ));
                continue;
            }
        };
        for root in [file.roots.bookmark_bar, file.roots.other, file.roots.synced]
            .into_iter()
            .flatten()
        {
            // ルート自体の名前 ("お気に入りバー" など) はパンくずに出さない
            for child in &root.children {
                collect(child, &mut Vec::new(), &mut bookmarks);
            }
        }
    }
    bookmarks
}

fn collect(node: &Node, parents: &mut Vec<String>, out: &mut Vec<Bookmark>) {
    match node.kind.as_str() {
        "url" => {
            if let Some(url) = &node.url {
                out.push(Bookmark {
                    name: node.name.clone(),
                    url: url.clone(),
                    breadcrumb: parents.join(" > "),
                });
            }
        }
        "folder" => {
            parents.push(node.name.clone());
            for child in &node.children {
                collect(child, parents, out);
            }
            parents.pop();
        }
        _ => {}
    }
}

fn profile_paths() -> Vec<PathBuf> {
    let Some(local) = dirs::data_local_dir() else {
        return Vec::new();
    };
    [
        local.join("Google\\Chrome\\User Data\\Default\\Bookmarks"),
        local.join("Microsoft\\Edge\\User Data\\Default\\Bookmarks"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_nested_folders_into_a_breadcrumb() {
        let file: BookmarksFile = serde_json::from_str(
            r#"{
                "roots": {
                    "bookmark_bar": {
                        "name": "Bookmarks bar",
                        "type": "folder",
                        "children": [
                            {
                                "name": "Work",
                                "type": "folder",
                                "children": [
                                    { "name": "GitHub", "type": "url", "url": "https://github.com/" }
                                ]
                            },
                            { "name": "Example", "type": "url", "url": "https://example.com/" }
                        ]
                    },
                    "other": { "name": "Other", "type": "folder", "children": [] },
                    "synced": { "name": "Synced", "type": "folder", "children": [] }
                }
            }"#,
        )
        .unwrap();

        let mut bookmarks = Vec::new();
        for child in &file.roots.bookmark_bar.as_ref().unwrap().children {
            collect(child, &mut Vec::new(), &mut bookmarks);
        }

        assert_eq!(
            bookmarks,
            vec![
                Bookmark {
                    name: "GitHub".into(),
                    url: "https://github.com/".into(),
                    breadcrumb: "Work".into(),
                },
                Bookmark {
                    name: "Example".into(),
                    url: "https://example.com/".into(),
                    breadcrumb: "".into(),
                },
            ]
        );
    }
}
