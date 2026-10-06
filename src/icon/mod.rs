//! Quick Launch とトレイのアイコン取得。

mod async_load;
mod convert;
mod scale;
mod window;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
use windows::core::HSTRING;

pub(crate) use async_load::{WM_ICON_READY, apply_ready, set_notify};
use convert::{icon_to_bitmap, load_bitmap, rgba_to_bitmap, shell_icon};
pub(crate) use window::bitmap_for_window_sized;

thread_local! {
    /// パス -> ビットマップ。Quick Launch の再描画ごとに引き直さない。
    static CACHE: RefCell<HashMap<String, Cached>> = RefCell::new(HashMap::new());
    /// 参照の新しさを測る通し番号。大きいほど最近使った。
    static TICK: Cell<u64> = const { Cell::new(0) };
}

fn next_tick() -> u64 {
    TICK.with(|tick| {
        let next = tick.get() + 1;
        tick.set(next);
        next
    })
}

/// キャッシュの中身。失敗も覚えるが、成功と違って期限を持つ。
enum Cached {
    /// ビットマップと、最後に参照した時点の通し番号。
    Bitmap(isize, Cell<u64>),
    Failed(Instant),
}

/// 取得に失敗した項目を再び引き直すまでの間隔。
///
/// 失敗を覚えないと、到達できないネットワークパスのように 1 回が重い
/// 相手へ再描画のたびに問い合わせて表示が固まる。一方で永久に覚えると、
/// たまたま取れなかっただけの項目 (オフラインだった共有、起動直後で
/// `WM_GETICON` に応答しなかったウィンドウ) がセッション中ずっと
/// アイコン無しのままになる。
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// キャッシュが抱える最大件数。
///
/// GDI オブジェクトはプロセスあたり 10,000 個で頭打ちになり (実測)、
/// そこに達すると以降すべてのビットマップ生成が失敗する。Everything の
/// 検索結果はパスごとに 1 件増えるため、上限に達したら古いものから捨てる
/// (`make_room`)。
const MAX_ENTRIES: usize = 2048;

/// 設定 (歯車) アイコンの在り処。
///
/// `SIID_SETTINGS` は実測で中身が空だったため使えない。
/// shell32.dll の 314 番が単体の歯車で、16px でも形が潰れない。
const SHELL32: &str = "shell32.dll";
const GEAR_INDEX: i32 = 314;

/// トレイの「設定」項目に使う歯車アイコン。
pub(crate) fn bitmap_for_settings_sized(size: i32) -> Option<HBITMAP> {
    bitmap_for_dll_icon(SHELL32, GEAR_INDEX, size)
}

/// 指定パスのアイコンを、メニューの iconSize 設定とは独立に
/// 指定寸法のビットマップとして得る。
///
/// Quick Launch のようにメニューと別の寸法で描くと、`bitmap_for` が
/// 返すビットマップとの寸法差で AlphaBlend が拡大縮小を行いにじむ
/// (要求寸法どおりのビットマップを直接取れば等倍コピーで済む)。
pub(crate) fn bitmap_for_sized(path: &str, size: i32) -> Option<HBITMAP> {
    cached_bitmap(&format!("{size}:{path}"), || load_bitmap(path, size))
}

/// Quick Launch の描画では、キャッシュミスでも外部処理を待たない。
pub(crate) fn bitmap_for_sized_async(path: &str, size: i32) -> Option<HBITMAP> {
    let path = path.to_owned();
    cached_bitmap_async(&format!("async:{size}:{path}"), move || {
        load_bitmap(&path, size)
    })
}

/// DLL に埋め込まれたアイコンをインデックス指定で取得する。
///
/// `SHGetStockIconInfo` は ID によっては中身が空のアイコンを返す
/// (実測: `SIID_SETTINGS` は全ピクセルが透明で、メニューには何も
/// 表示されない)。歯車のように標準 ID から取れないものは、
/// シェルの DLL から直接引く。
fn bitmap_for_dll_icon(dll: &str, index: i32, size: i32) -> Option<HBITMAP> {
    cached_bitmap(&dll_icon_key(dll, index, size), || {
        load_dll_icon(dll, index, size)
    })
}

fn dll_icon_key(dll: &str, index: i32, size: i32) -> String {
    format!("dll-icon:{size}:{dll}:{index}")
}

fn load_dll_icon(dll: &str, index: i32, size: i32) -> Option<HBITMAP> {
    unsafe {
        let path = HSTRING::from(dll);
        let mut large = HICON::default();
        let mut small = HICON::default();
        // 大小の両方を取り、描画寸法に近いほうを使う。
        let extracted = ExtractIconExW(&path, index, Some(&mut large), Some(&mut small), 1);
        if extracted == 0 {
            return None;
        }
        let prefer_large = size > 16 && !large.is_invalid();
        let chosen = if prefer_large { large } else { small };
        let bitmap = (!chosen.is_invalid())
            .then(|| icon_to_bitmap(chosen, SIZE { cx: size, cy: size }))
            .flatten();
        if !large.is_invalid() {
            let _ = DestroyIcon(large);
        }
        if !small.is_invalid() {
            let _ = DestroyIcon(small);
        }
        bitmap
    }
}

/// 埋め込み PNG を指定寸法へ縮小して使う。
pub(crate) fn bitmap_for_asset_sized(key: &str, png: &[u8], size: i32) -> Option<HBITMAP> {
    cached_bitmap(&format!("asset-icon:{size}:{key}"), || {
        load_asset(png, size)
    })
}

fn load_asset(png: &[u8], size: i32) -> Option<HBITMAP> {
    let target = SIZE { cx: size, cy: size };
    let image = image::load_from_memory(png).ok()?.into_rgba8();
    let image = image::imageops::resize(
        &image,
        target.cx as u32,
        target.cy as u32,
        image::imageops::FilterType::Lanczos3,
    );
    rgba_to_bitmap(image.as_raw(), target)
}

/// トレイメニューの 16px アイコンを、バックグラウンドで先に読んでおく。
///
/// 初回の右クリックで同期に読むと 25ms ほど固まる (実測: 実行ファイルの
/// シェルアイコンだけで 19ms、歯車 5ms)。結果は `apply_ready` で取り込まれ、
/// メニュー側の `bitmap_for_*` はキャッシュに当たる。間に合わなければ従来どおり
/// その場で読む。UI スレッドから呼ぶ (読み込み用ワーカーはスレッドごとに持つ)。
pub(crate) fn prefetch_menu_icons(exe_path: &str, assets: &[(&'static str, &'static [u8])]) {
    const SIZE_PX: i32 = 16;
    let mut jobs: Vec<(String, async_load::Loader)> = Vec::new();
    jobs.push((
        dll_icon_key(SHELL32, GEAR_INDEX, SIZE_PX),
        Box::new(|| load_dll_icon(SHELL32, GEAR_INDEX, SIZE_PX)),
    ));
    let exe = exe_path.to_owned();
    jobs.push((
        format!("{SIZE_PX}:{exe_path}"),
        Box::new(move || load_bitmap(&exe, SIZE_PX)),
    ));
    for &(key, png) in assets {
        jobs.push((
            format!("asset-icon:{SIZE_PX}:{key}"),
            Box::new(move || load_asset(png, SIZE_PX)),
        ));
    }
    for (key, load) in jobs {
        if cached_value(&key).is_none() {
            async_load::request_boxed(&key, load);
        }
    }
}

/// ブックマーク URL に対応する favicon を、Chrome/Edge の `Favicons` DB
/// から得て Quick Launch 用ビットマップにする。見つからなければ None
/// (呼び出し側が汎用のリンクアイコンへフォールバックする)。
pub(crate) fn bitmap_for_favicon_sized(url: &str, size: i32) -> Option<HBITMAP> {
    let url = url.to_owned();
    cached_bitmap_async(&format!("favicon:{size}:{url}"), move || {
        let png = crate::favicons::lookup(&url)?;
        let image = image::load_from_memory(&png).ok()?.into_rgba8();
        let target = SIZE { cx: size, cy: size };
        let image = if image.width() as i32 == size && image.height() as i32 == size {
            image
        } else {
            image::imageops::resize(
                &image,
                target.cx as u32,
                target.cy as u32,
                image::imageops::FilterType::Lanczos3,
            )
        };
        rgba_to_bitmap(image.as_raw(), target)
    })
}

fn cached_value(key: &str) -> Option<Option<HBITMAP>> {
    // 「まだ引いていない」と「引いて駄目だった」を区別する。前者は
    // load へ進み、後者は期限が切れるまで None を返す
    CACHE
        .with(|cache| match cache.borrow().get(key) {
            Some(Cached::Bitmap(raw, used)) => {
                used.set(next_tick());
                Some(Some(*raw))
            }
            Some(Cached::Failed(at)) if at.elapsed() < RETRY_AFTER => Some(None),
            _ => None,
        })
        .map(|hit| hit.map(|raw| HBITMAP(raw as *mut _)))
}

fn cached_bitmap_async(
    key: &str,
    load: impl FnOnce() -> Option<HBITMAP> + Send + 'static,
) -> Option<HBITMAP> {
    if let Some(hit) = cached_value(key) {
        return hit;
    }
    async_load::request(key, load);
    None
}

fn cached_bitmap(key: &str, load: impl FnOnce() -> Option<HBITMAP>) -> Option<HBITMAP> {
    if let Some(hit) = cached_value(key) {
        return hit;
    }
    let bitmap = load();
    store_bitmap(key, bitmap);
    bitmap
}

fn store_bitmap(key: &str, bitmap: Option<HBITMAP>) {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        // 既にビットマップがあるキーは置き換えない。メニューなどへ渡した
        // ハンドルを後から削除すると、表示中のアイコンが消える (先に同期で
        // 読んだ結果と、バックグラウンドの先読みが重なったときに起きる)。
        // 後から来た方が余るので解放する。
        if matches!(cache.get(key), Some(Cached::Bitmap(..))) {
            if let Some(value) = bitmap {
                unsafe {
                    let _ = DeleteObject(value.into());
                }
            }
            return;
        }
        if cache.len() >= MAX_ENTRIES && !cache.contains_key(key) {
            make_room(&mut cache);
        }
        let entry = match bitmap {
            Some(value) => Cached::Bitmap(value.0 as isize, Cell::new(next_tick())),
            None => Cached::Failed(Instant::now()),
        };
        cache.insert(key.to_string(), entry);
    });
}

/// 上限に達したとき、全件ではなく古いものから一部だけ捨てる。
///
/// 全件を捨てると表示中の行のアイコンまで取り直しになる。参照のたびに
/// 通し番号を更新しているので、表示中の行は新しく、先に落ちるのは
/// しばらく見ていないものになる。
fn make_room(cache: &mut HashMap<String, Cached>) {
    // 失敗の記憶は GDI を使わず、期限切れは引き直す対象なので先に捨てる
    cache.retain(|_, entry| !matches!(entry, Cached::Failed(at) if at.elapsed() >= RETRY_AFTER));
    if cache.len() < MAX_ENTRIES {
        return;
    }
    let mut by_use: Vec<(u64, String)> = cache
        .iter()
        .filter_map(|(key, entry)| match entry {
            Cached::Bitmap(_, used) => Some((used.get(), key.clone())),
            Cached::Failed(_) => None,
        })
        .collect();
    by_use.sort_unstable();
    for (_, key) in by_use.into_iter().take(MAX_ENTRIES / 4) {
        if let Some(Cached::Bitmap(raw, _)) = cache.remove(&key) {
            unsafe {
                let _ = DeleteObject(HBITMAP(raw as *mut _).into());
            }
        }
    }
    // 失敗の記憶だけで埋まっている場合は、それらを捨てて空きを作る
    if cache.len() >= MAX_ENTRIES {
        cache.retain(|_, entry| matches!(entry, Cached::Bitmap(..)));
    }
}

/// 値の `HBITMAP` を解放しつつ全件捨てる。
///
/// `HashMap` を空にするだけでは中身の `HBITMAP` は解放されない。
/// テーマ変更・設定再読み込みのたびに全件 GDI リークし、頻繁な
/// 切り替えでプロセスの GDI ハンドル上限に達してアイコンが描けなく
/// なる (実測で確認済み) 。値を読んでから `DeleteObject` する。
fn drop_entries(cache: &mut HashMap<String, Cached>) {
    for (_, entry) in cache.drain() {
        if let Cached::Bitmap(raw, _) = entry {
            unsafe {
                let _ = DeleteObject(HBITMAP(raw as *mut _).into());
            }
        }
    }
}

/// ファイルパスを持たないシェル名前空間項目のアイコンを得る。
pub(crate) fn bitmap_for_shell_sized(target: &str, size: i32) -> Option<HBITMAP> {
    let target = target.to_owned();
    cached_bitmap_async(&format!("shell-namespace:{size}:{target}"), move || {
        let icon = shell_icon(&target, size)?;
        let bitmap = icon_to_bitmap(icon, SIZE { cx: size, cy: size });
        unsafe {
            let _ = DestroyIcon(icon);
        }
        bitmap
    })
}

/// キャッシュを捨てる。テーマ変更や設定再読み込みで呼ぶ。
pub(crate) fn clear_cache() {
    async_load::clear();
    CACHE.with(|cache| drop_entries(&mut cache.borrow_mut()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap() -> HBITMAP {
        rgba_to_bitmap(&[255, 0, 0, 255], SIZE { cx: 1, cy: 1 }).unwrap()
    }

    fn contains(key: &str) -> bool {
        CACHE.with(|cache| matches!(cache.borrow().get(key), Some(Cached::Bitmap(..))))
    }

    /// 上限を大きく超えて挿入しても、直近に見ていた行 (表示中の 24 件) は
    /// 取り直しにならない。以前は上限到達で全件が消えていた。
    #[test]
    fn eviction_keeps_recently_used_icons() {
        clear_cache();
        let visible: Vec<String> = (0..24).map(|i| format!("visible-{i}")).collect();
        for key in &visible {
            store_bitmap(key, Some(bitmap()));
        }
        for i in 0..MAX_ENTRIES * 3 {
            store_bitmap(&format!("other-{i}"), Some(bitmap()));
            // 描画のたびに表示中の行を引く
            for key in &visible {
                assert!(cached_value(key).is_some(), "{key} was dropped at {i}");
            }
        }
        assert!(CACHE.with(|cache| cache.borrow().len()) <= MAX_ENTRIES);
        clear_cache();
    }

    #[test]
    fn expired_failures_are_dropped_before_any_bitmap() {
        clear_cache();
        store_bitmap("keep", Some(bitmap()));
        CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            for i in 0..MAX_ENTRIES - 1 {
                cache.insert(
                    format!("failed-{i}"),
                    Cached::Failed(Instant::now() - RETRY_AFTER - Duration::from_secs(1)),
                );
            }
        });
        store_bitmap("new", Some(bitmap()));
        assert!(contains("keep"));
        assert!(contains("new"));
        clear_cache();
    }

    #[test]
    fn replacing_a_key_does_not_leak_the_old_bitmap() {
        clear_cache();
        store_bitmap("same", Some(bitmap()));
        store_bitmap("same", Some(bitmap()));
        assert_eq!(CACHE.with(|cache| cache.borrow().len()), 1);
        clear_cache();
    }

    #[test]
    fn storing_an_existing_key_keeps_the_handle_already_handed_out() {
        clear_cache();
        let first = bitmap();
        store_bitmap("same", Some(first));
        store_bitmap("same", Some(bitmap()));
        assert_eq!(cached_value("same"), Some(Some(first)));
        clear_cache();
    }

    #[test]
    fn prefetched_menu_icons_are_picked_up_by_apply_ready() {
        clear_cache();
        let png = include_bytes!("../../assets/menu/reload.png");
        prefetch_menu_icons("", &[("reload", png)]);
        let key = "asset-icon:16:reload";
        let deadline = Instant::now() + Duration::from_secs(5);
        while !contains(key) {
            apply_ready();
            assert!(Instant::now() < deadline, "prefetch did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        }
        clear_cache();
    }

    /// トレイ右クリックメニューの初回表示で同期に読む 3 つのアイコンの所要時間。
    #[test]
    #[ignore = "手動計測用"]
    fn bench_tray_menu_icons_cold() {
        let time = |label: &str, run: &dyn Fn() -> bool| {
            let start = Instant::now();
            let ok = run();
            println!(
                "  {label:<22} {:>8.3} ms (ok={ok})",
                start.elapsed().as_secs_f64() * 1000.0
            );
        };
        time("settings (ExtractIcon)", &|| {
            bitmap_for_settings_sized(16).is_some()
        });
        let exe = std::env::current_exe().unwrap();
        time("exe (SHGetFileInfo)", &|| {
            bitmap_for_sized(exe.to_string_lossy().as_ref(), 16).is_some()
        });
        let png = include_bytes!("../../assets/menu/reload.png");
        time("reload.png (Lanczos3)", &|| {
            bitmap_for_asset_sized("reload", png, 16).is_some()
        });
        let png = include_bytes!("../../assets/menu/close.png");
        time("close.png (Lanczos3)", &|| {
            bitmap_for_asset_sized("close", png, 16).is_some()
        });
        clear_cache();
    }
}
