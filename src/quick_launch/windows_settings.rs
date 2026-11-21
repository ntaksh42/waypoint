//! Windows の設定ページの固定候補 (`ws ` プレフィックス、FR-9.29)。
//!
//! 設定アプリのページ (`ms-settings:` URI) と、`ms-settings:` では開けない
//! 旧コントロールパネル・管理ツールを、ビルド時に埋め込んだ静的な表から
//! 検索する。列挙も存在確認もしないので、インデックス構築・表示・
//! キー入力のどの経路にも I/O を足さない。

use super::search::LowerKeys;
use super::{Action, Entry};

/// Windows 設定の検索モードに入るプレフィックス (末尾の半角スペース込み)。
pub const WINDOWS_SETTINGS_PREFIX: &str = "ws ";

/// (表示名, 区分, 開く対象, 検索語)。
///
/// 開く対象は `ms-settings:` URI か、`<実行ファイル> <引数>` (最初の
/// 半角スペースで分ける、`shell::open_windows_setting`)。検索語は
/// `ws ` モードでだけ照合する別名で、日本語の画面名を含める。
#[rustfmt::skip]
const TABLE: &[(&str, &str, &str, &str)] = &[
    ("Windows Settings", "Home", "ms-settings:", "設定 ホーム"),
    // System
    ("Display", "System", "ms-settings:display", "ディスプレイ 画面 解像度 拡大縮小 monitor resolution scale"),
    ("Night light", "System", "ms-settings:nightlight", "夜間モード blue light"),
    ("Sound", "System", "ms-settings:sound", "サウンド 音 音量 audio speaker volume output input"),
    ("Volume mixer", "System", "ms-settings:apps-volume", "音量ミキサー サウンド audio"),
    ("Notifications", "System", "ms-settings:notifications", "通知"),
    ("Focus", "System", "ms-settings:quiethours", "集中モード 応答不可 do not disturb"),
    ("Power & battery", "System", "ms-settings:powersleep", "電源 スリープ バッテリー sleep power"),
    ("Storage", "System", "ms-settings:storagesense", "ストレージ ディスク 容量 disk cleanup"),
    ("Multitasking", "System", "ms-settings:multitasking", "マルチタスク スナップ snap alt tab"),
    ("Clipboard", "System", "ms-settings:clipboard", "クリップボード 履歴 history"),
    ("Remote Desktop", "System", "ms-settings:remotedesktop", "リモートデスクトップ rdp"),
    ("About", "System", "ms-settings:about", "バージョン情報 PC名 rename pc version specs"),
    ("Activation", "System", "ms-settings:activation", "ライセンス認証 product key"),
    ("Troubleshoot", "System", "ms-settings:troubleshoot", "トラブルシューティング"),
    ("Recovery", "System", "ms-settings:recovery", "回復 リセット reset"),
    // Bluetooth & devices
    ("Bluetooth & devices", "Devices", "ms-settings:bluetooth", "ブルートゥース デバイス"),
    ("Printers & scanners", "Devices", "ms-settings:printers", "プリンター スキャナー"),
    ("Mouse", "Devices", "ms-settings:mousetouchpad", "マウス ポインター速度 スクロール"),
    ("Touchpad", "Devices", "ms-settings:devices-touchpad", "タッチパッド"),
    ("Pen & Windows Ink", "Devices", "ms-settings:pen", "ペン"),
    ("AutoPlay", "Devices", "ms-settings:autoplay", "自動再生"),
    ("USB", "Devices", "ms-settings:usb", "usb"),
    ("Cameras", "Devices", "ms-settings:camera", "カメラ webcam"),
    // Network & internet
    ("Network & internet", "Network", "ms-settings:network", "ネットワーク インターネット"),
    ("Wi-Fi", "Network", "ms-settings:network-wifi", "無線 wifi wireless"),
    ("Ethernet", "Network", "ms-settings:network-ethernet", "イーサネット 有線 lan"),
    ("VPN", "Network", "ms-settings:network-vpn", "vpn"),
    ("Proxy", "Network", "ms-settings:network-proxy", "プロキシ"),
    ("Mobile hotspot", "Network", "ms-settings:network-mobilehotspot", "モバイルホットスポット テザリング tethering"),
    ("Airplane mode", "Network", "ms-settings:network-airplanemode", "機内モード"),
    ("Advanced network settings", "Network", "ms-settings:network-advancedsettings", "ネットワークの詳細設定 アダプター adapter"),
    // Personalization
    ("Personalization", "Personalization", "ms-settings:personalization", "個人用設定"),
    ("Background", "Personalization", "ms-settings:personalization-background", "背景 壁紙 wallpaper"),
    ("Colors", "Personalization", "ms-settings:personalization-colors", "色 ダークモード ライトモード dark mode light mode accent"),
    ("Themes", "Personalization", "ms-settings:themes", "テーマ"),
    ("Lock screen", "Personalization", "ms-settings:lockscreen", "ロック画面"),
    ("Start", "Personalization", "ms-settings:personalization-start", "スタート メニュー start menu"),
    ("Taskbar", "Personalization", "ms-settings:taskbar", "タスクバー"),
    ("Fonts", "Personalization", "ms-settings:fonts", "フォント"),
    // Apps
    ("Installed apps", "Apps", "ms-settings:appsfeatures", "インストールされているアプリ アンインストール uninstall programs"),
    ("Default apps", "Apps", "ms-settings:defaultapps", "既定のアプリ 関連付け default browser file association"),
    ("Optional features", "Apps", "ms-settings:optionalfeatures", "オプション機能"),
    ("Startup apps", "Apps", "ms-settings:startupapps", "スタートアップ 自動起動 startup"),
    // Accounts
    ("Your info", "Accounts", "ms-settings:yourinfo", "ユーザーの情報 アカウント account"),
    ("Sign-in options", "Accounts", "ms-settings:signinoptions", "サインインオプション pin hello password"),
    ("Email & accounts", "Accounts", "ms-settings:emailandaccounts", "メールとアカウント"),
    ("Other users", "Accounts", "ms-settings:otherusers", "他のユーザー family"),
    ("Windows backup", "Accounts", "ms-settings:backup", "バックアップ onedrive"),
    // Time & language
    ("Date & time", "Time & language", "ms-settings:dateandtime", "日付と時刻 時計 タイムゾーン clock timezone"),
    ("Language & region", "Time & language", "ms-settings:regionlanguage", "言語 地域 language region"),
    ("Typing", "Time & language", "ms-settings:typing", "入力 タイピング"),
    ("Advanced keyboard settings", "Time & language", "ms-settings:keyboard", "キーボード ime 入力方式"),
    ("Speech", "Time & language", "ms-settings:speech", "音声認識"),
    // Gaming
    ("Game Bar", "Gaming", "ms-settings:gaming-gamebar", "ゲームバー xbox"),
    ("Game Mode", "Gaming", "ms-settings:gaming-gamemode", "ゲームモード"),
    // Accessibility
    ("Accessibility", "Accessibility", "ms-settings:easeofaccess", "アクセシビリティ"),
    ("Text size", "Accessibility", "ms-settings:easeofaccess-display", "テキストのサイズ 文字 font size"),
    ("Mouse pointer and touch", "Accessibility", "ms-settings:easeofaccess-mousepointer", "マウスポインター カーソル cursor"),
    ("Keyboard accessibility", "Accessibility", "ms-settings:easeofaccess-keyboard", "固定キー sticky keys"),
    ("Narrator", "Accessibility", "ms-settings:easeofaccess-narrator", "ナレーター"),
    ("Magnifier", "Accessibility", "ms-settings:easeofaccess-magnifier", "拡大鏡"),
    ("Color filters", "Accessibility", "ms-settings:easeofaccess-colorfilter", "カラーフィルター"),
    // Privacy & security
    ("Privacy & security", "Privacy", "ms-settings:privacy", "プライバシー セキュリティ"),
    ("Windows Security", "Privacy", "ms-settings:windowsdefender", "セキュリティ defender ウイルス antivirus firewall"),
    ("For developers", "Privacy", "ms-settings:developers", "開発者向け developer mode sudo"),
    ("Find my device", "Privacy", "ms-settings:findmydevice", "デバイスの検索"),
    ("Location", "Privacy", "ms-settings:privacy-location", "位置情報 gps"),
    ("Camera access", "Privacy", "ms-settings:privacy-webcam", "カメラ プライバシー webcam permission"),
    ("Microphone access", "Privacy", "ms-settings:privacy-microphone", "マイク プライバシー mic permission"),
    // Windows Update
    ("Windows Update", "Windows Update", "ms-settings:windowsupdate", "更新 アップデート update"),
    ("Update history", "Windows Update", "ms-settings:windowsupdate-history", "更新の履歴"),
    ("Windows Update advanced options", "Windows Update", "ms-settings:windowsupdate-options", "詳細オプション active hours"),
    // ms-settings: では開けない旧来の画面
    ("Environment Variables", "Control Panel", "rundll32.exe sysdm.cpl,EditEnvironmentVariables", "環境変数 path env"),
    ("Advanced system settings", "Control Panel", "SystemPropertiesAdvanced.exe", "システムの詳細設定 システムのプロパティ 仮想メモリ performance virtual memory"),
    ("Programs and Features", "Control Panel", "control.exe appwiz.cpl", "プログラムと機能 アンインストール uninstall"),
    ("Windows Features", "Control Panel", "optionalfeatures.exe", "windows の機能の有効化 hyper-v wsl"),
    ("Network Connections", "Control Panel", "control.exe ncpa.cpl", "ネットワーク接続 アダプター adapter"),
    ("Sound Control Panel", "Control Panel", "control.exe mmsys.cpl", "サウンド 録音 再生デバイス recording playback"),
    ("Power Options", "Control Panel", "control.exe powercfg.cpl", "電源オプション 電源プラン power plan"),
    ("Mouse Properties", "Control Panel", "control.exe main.cpl", "マウスのプロパティ"),
    ("Internet Options", "Control Panel", "control.exe inetcpl.cpl", "インターネットオプション"),
    ("Credential Manager", "Control Panel", "control.exe /name Microsoft.CredentialManager", "資格情報マネージャー パスワード credential password"),
    ("Control Panel", "Control Panel", "control.exe", "コントロールパネル"),
    ("Device Manager", "Administrative Tools", "devmgmt.msc", "デバイスマネージャー ドライバー driver"),
    ("Services", "Administrative Tools", "services.msc", "サービス"),
    ("Disk Management", "Administrative Tools", "diskmgmt.msc", "ディスクの管理 パーティション partition"),
    ("Task Scheduler", "Administrative Tools", "taskschd.msc", "タスクスケジューラ"),
    ("Event Viewer", "Administrative Tools", "eventvwr.msc", "イベントビューアー ログ log"),
    ("Computer Management", "Administrative Tools", "compmgmt.msc", "コンピューターの管理"),
];

/// 候補と、`ws ` モード用 (名前 + 区分 + 検索語) と通常検索用 (名前だけ) の
/// 2 種類の検索キーを返す。
///
/// 通常検索は FR-9.20 の固定候補と同じく名前だけで照合する。検索語まで
/// 通常検索に載せると、別名の fuzzy 一致がフォルダ等の検索結果へ割り込むため。
pub(crate) fn windows_setting_entries()
-> (&'static [Entry], &'static [LowerKeys], &'static [LowerKeys]) {
    static ENTRIES: std::sync::LazyLock<Vec<Entry>> = std::sync::LazyLock::new(|| {
        TABLE
            .iter()
            .map(|(name, category, target, _)| Entry {
                azure: None,
                name: (*name).to_string(),
                breadcrumb: format!("Windows Settings \u{203A} {category}"),
                path: (*target).to_string(),
                action: Action::OpenWindowsSetting,
                branch: None,
            })
            .collect()
    });
    static KEYWORD_KEYS: std::sync::LazyLock<Vec<LowerKeys>> = std::sync::LazyLock::new(|| {
        TABLE
            .iter()
            .map(|(name, category, _, keywords)| {
                LowerKeys::from_name_and_keywords(name, &format!("{category} {keywords}"))
            })
            .collect()
    });
    static NAME_KEYS: std::sync::LazyLock<Vec<LowerKeys>> =
        std::sync::LazyLock::new(|| LowerKeys::build_for_names(&ENTRIES));
    (&ENTRIES, &KEYWORD_KEYS, &NAME_KEYS)
}
