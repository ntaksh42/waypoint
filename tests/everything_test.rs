//! Everything (voidtools) IPC 応答のパースのテスト (FR-9.16)。

use waypoint::everything::{EverythingResult, IPC_FOLDER, parse_results};

/// テスト用に `EVERYTHING_IPC_LISTW` 相当のバイト列を組み立てる。
fn build_list(items: &[(u32, &str, &str)]) -> Vec<u8> {
    let mut pool: Vec<u8> = Vec::new();
    let mut entries = Vec::new();
    for (flags, name, dir) in items {
        let name_offset = 28 + items.len() * 12 + pool.len();
        for unit in name.encode_utf16().chain(std::iter::once(0)) {
            pool.extend_from_slice(&unit.to_le_bytes());
        }
        let path_offset = 28 + items.len() * 12 + pool.len();
        for unit in dir.encode_utf16().chain(std::iter::once(0)) {
            pool.extend_from_slice(&unit.to_le_bytes());
        }
        entries.push((*flags, name_offset as u32, path_offset as u32));
    }

    let mut buffer = Vec::new();
    let numitems = items.len() as u32;
    buffer.extend_from_slice(&0u32.to_le_bytes()); // totfolders
    buffer.extend_from_slice(&0u32.to_le_bytes()); // totfiles
    buffer.extend_from_slice(&numitems.to_le_bytes()); // totitems
    buffer.extend_from_slice(&0u32.to_le_bytes()); // numfolders
    buffer.extend_from_slice(&0u32.to_le_bytes()); // numfiles
    buffer.extend_from_slice(&numitems.to_le_bytes()); // numitems
    buffer.extend_from_slice(&0u32.to_le_bytes()); // offset
    for (flags, name_offset, path_offset) in entries {
        buffer.extend_from_slice(&flags.to_le_bytes());
        buffer.extend_from_slice(&name_offset.to_le_bytes());
        buffer.extend_from_slice(&path_offset.to_le_bytes());
    }
    buffer.extend_from_slice(&pool);
    buffer
}

#[test]
fn parses_files_and_folders_with_their_directory() {
    let data = build_list(&[
        (IPC_FOLDER, "src", r"E:\waypoint"),
        (0, "Cargo.toml", r"E:\waypoint"),
    ]);
    let results = parse_results(&data);
    assert_eq!(
        results,
        vec![
            EverythingResult {
                name: "src".into(),
                path: r"E:\waypoint\src".into(),
                is_folder: true,
            },
            EverythingResult {
                name: "Cargo.toml".into(),
                path: r"E:\waypoint\Cargo.toml".into(),
                is_folder: false,
            },
        ]
    );
}

#[test]
fn empty_list_yields_no_results() {
    let data = build_list(&[]);
    assert!(parse_results(&data).is_empty());
}

#[test]
fn truncated_data_does_not_panic() {
    assert!(parse_results(&[1, 2, 3]).is_empty());
    assert!(parse_results(&[]).is_empty());
}

#[test]
fn impossible_item_count_does_not_allocate_from_the_untrusted_header() {
    let mut data = vec![0; 28];
    data[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(parse_results(&data).is_empty());
}

#[test]
fn root_drive_has_no_directory_separator_duplicated() {
    let data = build_list(&[(IPC_FOLDER, "E:", "")]);
    let results = parse_results(&data);
    assert_eq!(results[0].path, "E:");
}
