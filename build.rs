//! 実行ファイルにマニフェストを埋め込む。
//!
//! comctl32 v6 を要求するため。これが無いと native-windows-gui が使う
//! GetWindowSubclass が解決できず、起動時にローダが止まる。

fn main() {
    println!("cargo:rerun-if-changed=waypoint.manifest");

    // MSVC リンカにマニフェストを渡す
    #[cfg(target_env = "msvc")]
    {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("waypoint.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
        // 既定のマニフェストと衝突させない
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
    }
}
