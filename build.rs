//! 実行ファイルにマニフェストを埋め込み、設定画面 (C# + WPF) を
//! 常駐部と同じディレクトリへ用意する。
//!
//! マニフェストは Per-Monitor V2 / longPathAware / Visual Styles のために要る。

fn main() {
    println!("cargo:rerun-if-changed=waypoint.manifest");
    println!("cargo:rerun-if-changed=assets/waypoint.ico");

    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("assets/waypoint.ico")
        .compile()
        .expect("アプリアイコンの埋め込みに失敗");

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

    build_settings_app();
}

/// 設定画面 (`settings/`、C# + WPF) をビルドして常駐部の隣へ置く。
///
/// 常駐部は `waypoint-settings.exe` を **自分と同じディレクトリ** からしか
/// 探さない (`tray/actions.rs::open_settings`)。cargo の出力先は
/// `target/debug` と `target/<triple>/release` に分かれるため、ここで都度
/// 出力先を求めて配置しないと、以前そこにあった古い exe をそのまま掴む
/// (実際に v0.3.13 時代の Rust/egui 版が起動する事故が起きた)。
///
/// `dotnet` が無い環境やビルド失敗では警告だけ出して続行する。設定画面が
/// 無くても常駐部自体は動くので、Rust 側のビルドまで巻き込んで止めない。
fn build_settings_app() {
    println!("cargo:rerun-if-env-changed=WAYPOINT_SKIP_SETTINGS_BUILD");
    // clippy / test など exe を使わない経路や CI では明示的に飛ばせる
    if std::env::var_os("WAYPOINT_SKIP_SETTINGS_BUILD").is_some() {
        return;
    }

    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project = manifest_dir
        .join("settings")
        .join("Waypoint.Settings.csproj");
    if !project.exists() {
        return;
    }
    println!("cargo:rerun-if-changed=settings");

    // OUT_DIR は `<target>/<profile>/build/<pkg>-<hash>/out`。
    // 3 つ上がると exe が置かれるディレクトリになる。
    let Some(out_dir) = std::env::var_os("OUT_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    let Some(exe_dir) = out_dir.ancestors().nth(3) else {
        return;
    };

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let configuration = if profile == "release" {
        "Release"
    } else {
        "Debug"
    };

    // release だけ self-contained の単一ファイルにする (配布物と同じ形)。
    // debug で毎回 publish すると 140MB 超の書き出しになり、ビルドが目に見えて
    // 遅くなるので framework-dependent のまま隣へ出す。
    let status = if profile == "release" {
        std::process::Command::new("dotnet")
            .args(["publish", project.to_str().unwrap_or_default()])
            .args([
                "-c",
                configuration,
                "-r",
                "win-x64",
                "--self-contained",
                "true",
            ])
            .args(["-p:PublishSingleFile=true", "-p:DebugType=None"])
            .args(["-o", exe_dir.to_str().unwrap_or_default()])
            .status()
    } else {
        std::process::Command::new("dotnet")
            .args(["build", project.to_str().unwrap_or_default()])
            .args(["-c", configuration])
            .args(["-o", exe_dir.to_str().unwrap_or_default()])
            .status()
    };

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => println!(
            "cargo:warning=設定画面のビルドに失敗しました (exit {status})。\
             waypoint-settings.exe が古いままの可能性があります"
        ),
        Err(error) => println!(
            "cargo:warning=dotnet を実行できませんでした ({error})。\
             設定画面は別途 `dotnet build settings\\Waypoint.Settings.csproj` が必要です"
        ),
    }
}
