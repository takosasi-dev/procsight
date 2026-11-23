//! 窓・exe・トレイのアイコンの絵(RGBA)を std だけで描く。build.rs からも include! で使う。
//! exe のアイコンは棒のグラフの印、トレイは CPU・メモリの使用率の棒(FR-47、v0.4.0 の tray_value)。

/// 角を丸めた四角の中にあるか(半径 r)
fn inside_rounded(x: f32, y: f32, size: f32, r: f32) -> bool {
    let cx = x.clamp(r, size - r);
    let cy = y.clamp(r, size - r);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

/// exe と窓のアイコン。`size` 四方の RGBA(左上から、1 画素 4 バイト)
pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    // 棒の高さ(全体に対する割合)
    let bars = [0.38f32, 0.72, 0.52];
    let pad = s * 0.18;
    let gap = s * 0.06;
    let bar_w = (s - pad * 2.0 - gap * 2.0) / 3.0;
    let base = s - pad;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let i = ((y * size + x) * 4) as usize;
            if !inside_rounded(fx, fy, s, s * 0.2) {
                continue;
            }
            let mut px = [0x1E, 0x2A, 0x38, 0xFF];
            for (k, h) in bars.iter().enumerate() {
                let x0 = pad + k as f32 * (bar_w + gap);
                let top = base - h * (s - pad * 2.0);
                if fx >= x0 && fx < x0 + bar_w && fy >= top && fy < base {
                    px = [0x3C, 0xC8, 0xA0, 0xFF];
                }
            }
            if fy >= base && fy < base + (s / 16.0).max(1.0) && fx >= pad && fx < s - pad {
                px = [0xE6, 0xEC, 0xF2, 0xFF];
            }
            out[i..i + 4].copy_from_slice(&px);
        }
    }
    out
}

/// 0〜100 に丸めた値と、中の高さ 14 画素のうち塗る画素の数(0% でも 1 画素は出す)
fn tray_level(pct: f32) -> (f32, usize) {
    let pct = if pct.is_finite() { pct.clamp(0.0, 100.0) } else { 0.0 };
    (pct, ((pct / 100.0 * 14.0).round() as usize).max(1))
}

/// CPU の棒の色(緑・橙・赤)
fn cpu_color(pct: f32) -> [u8; 4] {
    if pct >= 80.0 {
        [0xE5, 0x48, 0x4D, 0xFF]
    } else if pct >= 50.0 {
        [0xF0, 0xA0, 0x30, 0xFF]
    } else {
        [0x3C, 0xC8, 0xA0, 0xFF]
    }
}

/// メモリの棒の色(紫・橙・赤。CPU と見分けられるように)
fn mem_color(pct: f32) -> [u8; 4] {
    if pct >= 90.0 {
        [0xE5, 0x48, 0x4D, 0xFF]
    } else if pct >= 80.0 {
        [0xF0, 0xA0, 0x30, 0xFF]
    } else {
        [0xA0, 0x7C, 0xF0, 0xFF]
    }
}

/// 16×16 の枠の中に、下から伸びる棒を描く。棒は (左の x, 右の x(含まない), 塗る画素, 色)
fn tray_bars(bars: &[(usize, usize, usize, [u8; 4])]) -> Vec<u8> {
    const N: usize = 16;
    let mut out = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let i = (y * N + x) * 4;
            let edge = x == 0 || y == 0 || x == N - 1 || y == N - 1;
            let bar = bars.iter().find(|b| (b.0..b.1).contains(&x) && N - 1 - y <= b.2);
            let px = if edge {
                [0x9A, 0xA6, 0xB2, 0xFF]
            } else if let Some(b) = bar {
                b.3
            } else {
                [0x1E, 0x2A, 0x38, 0xFF]
            };
            out[i..i + 4].copy_from_slice(&px);
        }
    }
    out
}

/// トレイのアイコン(16×16)。CPU 使用率 `pct`(0〜100)を下から伸びる棒で描く
pub fn tray_cpu_rgba(pct: f32) -> Vec<u8> {
    let (p, filled) = tray_level(pct);
    tray_bars(&[(3, 13, filled, cpu_color(p))])
}

/// トレイのアイコン(16×16)。メモリの使用率 `pct`(0〜100)を CPU と同じ形・紫の棒で描く
pub fn tray_mem_rgba(pct: f32) -> Vec<u8> {
    let (p, filled) = tray_level(pct);
    tray_bars(&[(3, 13, filled, mem_color(p))])
}

/// トレイのアイコン(16×16)。左に CPU、右にメモリの細い棒を 2 本
pub fn tray_both_rgba(cpu: f32, mem: f32) -> Vec<u8> {
    let ((c, cf), (m, mf)) = (tray_level(cpu), tray_level(mem));
    tray_bars(&[(2, 7, cf, cpu_color(c)), (9, 14, mf, mem_color(m))])
}
