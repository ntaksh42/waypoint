//! Azure DevOps 候補の状態マーカー (タイル右下の小さな図形)。
//!
//! 色だけで状態を分けると、色覚特性やグレースケールで橙・赤・灰が見分け
//! にくい。そのため状態ごとに「色 + 形」を決める。形は円盤の中の塗り方
//! (`Fill`) と記号 (`Symbol`) で表し、`azure_tile::render` の
//! スーパーサンプリングで描く。

use windows::Win32::Foundation::COLORREF;

use super::azure_tile::render;
use super::rgb;
use crate::azure_devops::Kind;
use crate::quick_launch::AzureMeta;

/// 円盤の塗り方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fill {
    /// 全面を塗る。記号は白で抜く。
    Solid,
    /// 縁だけ。記号は円盤と同じ色で中に描く。
    Hollow,
    /// 縁と左半分を塗る (「途中」を表す)。
    Half,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Symbol {
    None,
    Check,
    Cross,
    Slash,
    Bang,
    /// 欠けた輪 + 矢じり。
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Marker {
    pub(super) color: COLORREF,
    pub(super) fill: Fill,
    pub(super) symbol: Symbol,
}

const GREEN: COLORREF = rgb(52, 199, 89);
const BLUE: COLORREF = rgb(10, 132, 255);
const ORANGE: COLORREF = rgb(255, 149, 0);
const RED: COLORREF = rgb(229, 72, 77);
const GRAY: COLORREF = rgb(142, 142, 147);
const AMBER: COLORREF = rgb(216, 169, 0);

fn marker(color: COLORREF, fill: Fill, symbol: Symbol) -> Marker {
    Marker {
        color,
        fill,
        symbol,
    }
}

/// 自分のレビュー待ち (橙の点) を最優先に、種別ごとの状態を 1 つだけ返す。
pub(super) fn marker_for(meta: &AzureMeta) -> Option<Marker> {
    if meta.needs_my_review {
        return Some(marker(ORANGE, Fill::Solid, Symbol::None));
    }
    let status = meta.status.to_ascii_lowercase();
    let spec = match meta.kind {
        Kind::PullRequest => match status.as_str() {
            "completed" => marker(GREEN, Fill::Solid, Symbol::Check),
            "abandoned" => marker(GRAY, Fill::Solid, Symbol::Cross),
            _ if meta.is_draft => marker(GRAY, Fill::Hollow, Symbol::None),
            _ if meta.ready_to_complete => marker(GREEN, Fill::Hollow, Symbol::Check),
            _ => return None,
        },
        Kind::Pipeline => match status.as_str() {
            "succeeded" => marker(GREEN, Fill::Solid, Symbol::Check),
            "failed" => marker(RED, Fill::Solid, Symbol::Cross),
            "inprogress" | "cancelling" => marker(BLUE, Fill::Solid, Symbol::Running),
            "canceled" => marker(GRAY, Fill::Solid, Symbol::Slash),
            "partiallysucceeded" => marker(AMBER, Fill::Solid, Symbol::Bang),
            _ => return None,
        },
        Kind::WorkItem => match status.as_str() {
            "new" | "proposed" | "to do" | "approved" | "design" => {
                marker(GRAY, Fill::Hollow, Symbol::None)
            }
            "active" | "committed" | "doing" | "in progress" => {
                marker(BLUE, Fill::Solid, Symbol::None)
            }
            "resolved" | "in review" | "testing" => marker(ORANGE, Fill::Half, Symbol::None),
            "closed" | "done" => marker(GREEN, Fill::Solid, Symbol::Check),
            "removed" => marker(GRAY, Fill::Solid, Symbol::Cross),
            _ => return None,
        },
        Kind::Project => return None,
    };
    Some(spec)
}

/// 円盤の半径 (タイルの一辺に対する比)。記号を載せる分だけ点より大きくする。
fn radius_ratio(marker: &Marker) -> f32 {
    if marker.symbol == Symbol::None {
        0.19
    } else {
        0.22
    }
}

/// 太さ `half` の線分 (`a`→`b`) に点が含まれるか。
fn near_segment(u: f32, v: f32, a: (f32, f32), b: (f32, f32), half: f32) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((u - a.0) * dx + (v - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (u - a.0 - t * dx).powi(2) + (v - a.1 - t * dy).powi(2) < half * half
}

fn in_triangle(u: f32, v: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (v - p.1) - (q.1 - p.1) * (u - p.0);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// 円盤の中心を原点、半径を 1 とした座標 `(u, v)` (v は下向き) が記号に含まれるか。
fn symbol_contains(symbol: Symbol, u: f32, v: f32) -> bool {
    const STROKE: f32 = 0.17;
    match symbol {
        Symbol::None => false,
        Symbol::Check => {
            near_segment(u, v, (-0.45, 0.0), (-0.1, 0.4), STROKE)
                || near_segment(u, v, (-0.1, 0.4), (0.5, -0.35), STROKE)
        }
        Symbol::Cross => {
            near_segment(u, v, (-0.4, -0.4), (0.4, 0.4), STROKE)
                || near_segment(u, v, (0.4, -0.4), (-0.4, 0.4), STROKE)
        }
        Symbol::Slash => near_segment(u, v, (-0.45, 0.45), (0.45, -0.45), STROKE),
        Symbol::Bang => {
            near_segment(u, v, (0.0, -0.55), (0.0, 0.1), 0.15)
                || u * u + (v - 0.5).powi(2) < 0.17 * 0.17
        }
        Symbol::Running => {
            let distance = (u * u + v * v).sqrt();
            let angle = v.atan2(u);
            // 右上 (-1.4..-0.5 rad) を欠けさせ、その端に時計回りの矢じりを置く
            let on_arc = (distance - 0.5).abs() < 0.15 && !(-1.4..-0.5).contains(&angle);
            on_arc || in_triangle(u, v, (0.48, -0.42), (0.145, -0.83), (0.025, -0.145))
        }
    }
}

/// 右下の状態マーカー。行の地の色 `ring` で縁取り、タイルとの境目を作る。
pub(super) fn marker_pixels(size: usize, marker: &Marker, ring: COLORREF) -> Vec<u32> {
    let side = size as f32;
    let radius = side * radius_ratio(marker);
    let outer = radius + (side * 0.09).max(1.0);
    let center = side - outer;
    render(size, |x, y| {
        let (dx, dy) = (x - center, y - center);
        let distance = (dx * dx + dy * dy).sqrt();
        if distance > outer {
            return None;
        }
        if distance > radius {
            return Some(ring);
        }
        let (u, v) = (dx / radius, dy / radius);
        let filled = match marker.fill {
            Fill::Solid => true,
            Fill::Hollow => distance / radius >= 0.5,
            Fill::Half => distance / radius >= 0.5 || u < 0.0,
        };
        if !filled {
            // 縁だけの円盤は中も地の色で抜き、下のタイルを透かさない
            // (透かすと記号とマーカーが重なって形が読めない)。
            return if symbol_contains(marker.symbol, u, v) {
                Some(marker.color)
            } else {
                Some(ring)
            };
        }
        if marker.fill == Fill::Solid && symbol_contains(marker.symbol, u, v) {
            Some(rgb(255, 255, 255))
        } else {
            Some(marker.color)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(pixel: u32) -> u32 {
        pixel >> 24
    }

    fn solid(symbol: Symbol) -> Marker {
        Marker {
            color: rgb(255, 149, 0),
            fill: Fill::Solid,
            symbol,
        }
    }

    #[test]
    fn marker_sits_in_the_bottom_right_with_a_ring() {
        let size = 22;
        let marker = solid(Symbol::None);
        let ring = rgb(17, 17, 17);
        let pixels = marker_pixels(size, &marker, ring);
        assert_eq!(alpha(pixels[0]), 0, "左上には何も描かない");
        let side = size as f32;
        let center = (side - side * 0.19 - side * 0.09) as usize;
        let pixel = pixels[center * size + center];
        assert_eq!(alpha(pixel), 255);
        assert_eq!((pixel >> 16) & 0xff, 255, "点の中心は点の色");
        let last = pixels[size * size - 1];
        assert_eq!(alpha(last), 0, "角は透明のまま");
    }

    #[test]
    fn symbols_leave_white_ink_inside_the_disc() {
        let size = 44;
        let ring = rgb(17, 17, 17);
        let plain = marker_pixels(size, &solid(Symbol::None), ring);
        for symbol in [
            Symbol::Check,
            Symbol::Cross,
            Symbol::Slash,
            Symbol::Bang,
            Symbol::Running,
        ] {
            let pixels = marker_pixels(size, &solid(symbol), ring);
            let white = pixels.iter().filter(|p| **p == 0xFFFF_FFFF).count();
            assert!(white > 0, "{symbol:?} が描かれていない");
        }
        assert!(!plain.contains(&0xFFFF_FFFF));
    }

    #[test]
    fn symbols_are_distinguishable_from_each_other() {
        let size = 44;
        let ring = rgb(17, 17, 17);
        let all = [
            solid(Symbol::None),
            solid(Symbol::Check),
            solid(Symbol::Cross),
            solid(Symbol::Slash),
            solid(Symbol::Bang),
            solid(Symbol::Running),
            Marker {
                fill: Fill::Hollow,
                ..solid(Symbol::None)
            },
            Marker {
                fill: Fill::Half,
                ..solid(Symbol::None)
            },
            Marker {
                fill: Fill::Hollow,
                ..solid(Symbol::Check)
            },
        ];
        let images: Vec<_> = all.iter().map(|m| marker_pixels(size, m, ring)).collect();
        for (i, a) in images.iter().enumerate() {
            for b in &images[i + 1..] {
                assert_ne!(a, b, "形が同じマーカーがある");
            }
        }
    }

    #[test]
    fn hollow_marker_is_clear_in_the_middle_and_filled_on_the_rim() {
        let size = 44;
        let ring = rgb(17, 17, 17);
        let marker = Marker {
            fill: Fill::Hollow,
            ..solid(Symbol::None)
        };
        let pixels = marker_pixels(size, &marker, ring);
        let side = size as f32;
        let center = (side - side * 0.19 - side * 0.09) as usize;
        let middle = pixels[center * size + center];
        assert_eq!((middle >> 16) & 0xff, 17, "中は地の色");
        let rim_x = center + (side * 0.19 * 0.8) as usize;
        let rim = pixels[center * size + rim_x];
        assert_eq!((rim >> 16) & 0xff, 255, "縁は点の色");
    }
}
