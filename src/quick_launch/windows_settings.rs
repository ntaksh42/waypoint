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

/// (表示名, 区分, 開く対象, 検索語, アイコン)。
///
/// アイコンは Windows 標準の Segoe MDL2 Assets のコードポイント。
///
/// 開く対象は `ms-settings:` URI か、`<実行ファイル> <引数>` (最初の
/// 半角スペースで分ける、`shell::open_windows_setting`)。検索語は
/// `ws ` モードでだけ照合する別名で、日本語の画面名を含める。
#[rustfmt::skip]
const TABLE: &[(&str, &str, &str, &str, u16)] = &[
    ("Windows Settings", "Home", "ms-settings:", "設定 ホーム", 0xe713),
    // System
    ("Display", "System", "ms-settings:display", "ディスプレイ 画面 解像度 拡大縮小 monitor resolution scale", 0xe7fb),
    ("Night light", "System", "ms-settings:nightlight", "夜間モード blue light", 0xe706),
    ("Sound", "System", "ms-settings:sound", "サウンド 音 音量 audio speaker volume output input", 0xe7f5),
    ("Volume mixer", "System", "ms-settings:apps-volume", "音量ミキサー サウンド audio", 0xe9e9),
    ("Notifications", "System", "ms-settings:notifications", "通知", 0xea8f),
    ("Focus", "System", "ms-settings:quiethours", "集中モード 応答不可 do not disturb", 0xe708),
    ("Power & battery", "System", "ms-settings:powersleep", "電源 スリープ バッテリー sleep power", 0xea93),
    ("Storage", "System", "ms-settings:storagesense", "ストレージ ディスク 容量 disk cleanup", 0xeda2),
    ("Multitasking", "System", "ms-settings:multitasking", "マルチタスク スナップ snap alt tab", 0xe7c4),
    ("Clipboard", "System", "ms-settings:clipboard", "クリップボード 履歴 history", 0xe77f),
    ("Remote Desktop", "System", "ms-settings:remotedesktop", "リモートデスクトップ rdp", 0xe8af),
    ("About", "System", "ms-settings:about", "バージョン情報 PC名 rename pc version specs", 0xe946),
    ("Activation", "System", "ms-settings:activation", "ライセンス認証 product key", 0xe8d7),
    ("Troubleshoot", "System", "ms-settings:troubleshoot", "トラブルシューティング", 0xe90f),
    ("Recovery", "System", "ms-settings:recovery", "回復 リセット reset", 0xe777),
    // Bluetooth & devices
    ("Bluetooth & devices", "Devices", "ms-settings:bluetooth", "ブルートゥース デバイス", 0xe702),
    ("Printers & scanners", "Devices", "ms-settings:printers", "プリンター スキャナー", 0xe749),
    ("Mouse", "Devices", "ms-settings:mousetouchpad", "マウス ポインター速度 スクロール", 0xe962),
    ("Touchpad", "Devices", "ms-settings:devices-touchpad", "タッチパッド", 0xefa5),
    ("Pen & Windows Ink", "Devices", "ms-settings:pen", "ペン", 0xe76d),
    ("AutoPlay", "Devices", "ms-settings:autoplay", "自動再生", 0xe768),
    ("USB", "Devices", "ms-settings:usb", "usb", 0xe88e),
    ("Cameras", "Devices", "ms-settings:camera", "カメラ webcam", 0xe722),
    // Network & internet
    ("Network & internet", "Network", "ms-settings:network", "ネットワーク インターネット", 0xe774),
    ("Wi-Fi", "Network", "ms-settings:network-wifi", "無線 wifi wireless", 0xe701),
    ("Ethernet", "Network", "ms-settings:network-ethernet", "イーサネット 有線 lan", 0xe839),
    ("VPN", "Network", "ms-settings:network-vpn", "vpn", 0xe705),
    ("Proxy", "Network", "ms-settings:network-proxy", "プロキシ", 0xeda3),
    ("Mobile hotspot", "Network", "ms-settings:network-mobilehotspot", "モバイルホットスポット テザリング tethering", 0xe704),
    ("Airplane mode", "Network", "ms-settings:network-airplanemode", "機内モード", 0xe709),
    ("Advanced network settings", "Network", "ms-settings:network-advancedsettings", "ネットワークの詳細設定 アダプター adapter", 0xe968),
    // Personalization
    ("Personalization", "Personalization", "ms-settings:personalization", "個人用設定", 0xe771),
    ("Background", "Personalization", "ms-settings:personalization-background", "背景 壁紙 wallpaper", 0xe8b9),
    ("Colors", "Personalization", "ms-settings:personalization-colors", "色 ダークモード ライトモード dark mode light mode accent", 0xe790),
    ("Themes", "Personalization", "ms-settings:themes", "テーマ", 0xee56),
    ("Lock screen", "Personalization", "ms-settings:lockscreen", "ロック画面", 0xe72e),
    ("Start", "Personalization", "ms-settings:personalization-start", "スタート メニュー start menu", 0xeca5),
    ("Taskbar", "Personalization", "ms-settings:taskbar", "タスクバー", 0xe90e),
    ("Fonts", "Personalization", "ms-settings:fonts", "フォント", 0xe8d2),
    // Apps
    ("Installed apps", "Apps", "ms-settings:appsfeatures", "インストールされているアプリ アンインストール uninstall programs", 0xe71d),
    ("Default apps", "Apps", "ms-settings:defaultapps", "既定のアプリ 関連付け default browser file association", 0xe7ac),
    ("Optional features", "Apps", "ms-settings:optionalfeatures", "オプション機能", 0xea86),
    ("Startup apps", "Apps", "ms-settings:startupapps", "スタートアップ 自動起動 startup", 0xe7e8),
    // Accounts
    ("Your info", "Accounts", "ms-settings:yourinfo", "ユーザーの情報 アカウント account", 0xe77b),
    ("Sign-in options", "Accounts", "ms-settings:signinoptions", "サインインオプション pin hello password", 0xe928),
    ("Email & accounts", "Accounts", "ms-settings:emailandaccounts", "メールとアカウント", 0xe715),
    ("Other users", "Accounts", "ms-settings:otherusers", "他のユーザー family", 0xe716),
    ("Windows backup", "Accounts", "ms-settings:backup", "バックアップ onedrive", 0xe753),
    // Time & language
    ("Date & time", "Time & language", "ms-settings:dateandtime", "日付と時刻 時計 タイムゾーン clock timezone", 0xec92),
    ("Language & region", "Time & language", "ms-settings:regionlanguage", "言語 地域 language region", 0xf2b7),
    ("Typing", "Time & language", "ms-settings:typing", "入力 タイピング", 0xe765),
    ("Advanced keyboard settings", "Time & language", "ms-settings:keyboard", "キーボード ime 入力方式", 0xf210),
    ("Speech", "Time & language", "ms-settings:speech", "音声認識", 0xe720),
    // Gaming
    ("Game Bar", "Gaming", "ms-settings:gaming-gamebar", "ゲームバー xbox", 0xe7fc),
    ("Game Mode", "Gaming", "ms-settings:gaming-gamemode", "ゲームモード", 0xe967),
    // Accessibility
    ("Accessibility", "Accessibility", "ms-settings:easeofaccess", "アクセシビリティ", 0xe776),
    ("Text size", "Accessibility", "ms-settings:easeofaccess-display", "テキストのサイズ 文字 font size", 0xe8e9),
    ("Mouse pointer and touch", "Accessibility", "ms-settings:easeofaccess-mousepointer", "マウスポインター カーソル cursor", 0xe7c9),
    ("Keyboard accessibility", "Accessibility", "ms-settings:easeofaccess-keyboard", "固定キー sticky keys", 0xeda7),
    ("Narrator", "Accessibility", "ms-settings:easeofaccess-narrator", "ナレーター", 0xed4d),
    ("Magnifier", "Accessibility", "ms-settings:easeofaccess-magnifier", "拡大鏡", 0xe71e),
    ("Color filters", "Accessibility", "ms-settings:easeofaccess-colorfilter", "カラーフィルター", 0xef3c),
    // Privacy & security
    ("Privacy & security", "Privacy", "ms-settings:privacy", "プライバシー セキュリティ", 0xebdb),
    ("Windows Security", "Privacy", "ms-settings:windowsdefender", "セキュリティ defender ウイルス antivirus firewall", 0xea18),
    ("For developers", "Privacy", "ms-settings:developers", "開発者向け developer mode sudo", 0xec7a),
    ("Find my device", "Privacy", "ms-settings:findmydevice", "デバイスの検索", 0xebde),
    ("Location", "Privacy", "ms-settings:privacy-location", "位置情報 gps", 0xe707),
    ("Camera access", "Privacy", "ms-settings:privacy-webcam", "カメラ プライバシー webcam permission", 0xe8b8),
    ("Microphone access", "Privacy", "ms-settings:privacy-microphone", "マイク プライバシー mic permission", 0xe720),
    // Windows Update
    ("Windows Update", "Windows Update", "ms-settings:windowsupdate", "更新 アップデート update", 0xe72c),
    ("Update history", "Windows Update", "ms-settings:windowsupdate-history", "更新の履歴", 0xe81c),
    ("Windows Update advanced options", "Windows Update", "ms-settings:windowsupdate-options", "詳細オプション active hours", 0xe8f3),
    // ms-settings: では開けない旧来の画面
    ("Environment Variables", "Control Panel", "rundll32.exe sysdm.cpl,EditEnvironmentVariables", "環境変数 path env", 0xe756),
    ("Advanced system settings", "Control Panel", "SystemPropertiesAdvanced.exe", "システムの詳細設定 システムのプロパティ 仮想メモリ performance virtual memory", 0xe9d9),
    ("Programs and Features", "Control Panel", "control.exe appwiz.cpl", "プログラムと機能 アンインストール uninstall", 0xed35),
    ("Windows Features", "Control Panel", "optionalfeatures.exe", "windows の機能の有効化 hyper-v wsl", 0xe762),
    ("Network Connections", "Control Panel", "control.exe ncpa.cpl", "ネットワーク接続 アダプター adapter", 0xe703),
    ("Sound Control Panel", "Control Panel", "control.exe mmsys.cpl", "サウンド 録音 再生デバイス recording playback", 0xe767),
    ("Power Options", "Control Panel", "control.exe powercfg.cpl", "電源オプション 電源プラン power plan", 0xe7e8),
    ("Mouse Properties", "Control Panel", "control.exe main.cpl", "マウスのプロパティ", 0xe962),
    ("Internet Options", "Control Panel", "control.exe inetcpl.cpl", "インターネットオプション", 0xe909),
    ("Credential Manager", "Control Panel", "control.exe /name Microsoft.CredentialManager", "資格情報マネージャー パスワード credential password", 0xe8d7),
    ("Control Panel", "Control Panel", "control.exe", "コントロールパネル", 0xf246),
    ("Device Manager", "Administrative Tools", "devmgmt.msc", "デバイスマネージャー ドライバー driver", 0xe772),
    ("Services", "Administrative Tools", "services.msc", "サービス", 0xe9f3),
    ("Disk Management", "Administrative Tools", "diskmgmt.msc", "ディスクの管理 パーティション partition", 0xeda2),
    ("Task Scheduler", "Administrative Tools", "taskschd.msc", "タスクスケジューラ", 0xe787),
    ("Event Viewer", "Administrative Tools", "eventvwr.msc", "イベントビューアー ログ log", 0xe9f9),
    ("Computer Management", "Administrative Tools", "compmgmt.msc", "コンピューターの管理", 0xe7f8),
];

/// 開く対象をキーにすることで、履歴から復元した候補にも同じアイコンを使う。
pub(crate) fn windows_setting_icon(path: &str) -> Option<u16> {
    TABLE
        .iter()
        .find_map(|&(_, _, target, _, icon)| (target == path).then_some(icon))
}

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
            .map(|(name, category, target, _, _)| Entry {
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
            .map(|(name, category, _, keywords, _)| {
                LowerKeys::from_name_and_keywords(name, &format!("{category} {keywords}"))
            })
            .collect()
    });
    static NAME_KEYS: std::sync::LazyLock<Vec<LowerKeys>> =
        std::sync::LazyLock::new(|| LowerKeys::build_for_names(&ENTRIES));
    (&ENTRIES, &KEYWORD_KEYS, &NAME_KEYS)
}
