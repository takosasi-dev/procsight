//! 窓・exe・トレイのアイコンの絵(RGBA)を std だけで描く。build.rs からも include! で使う。
//! exe のアイコンは棒のグラフの印、トレイは CPU 使用率の棒(FR-47)。

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

/// トレイのアイコン(16×16)。CPU 使用率 `pct`(0〜100)を下から伸びる棒で描く
pub fn tray_cpu_rgba(pct: f32) -> Vec<u8> {
    const N: usize = 16;
    let pct = if pct.is_finite() { pct.clamp(0.0, 100.0) } else { 0.0 };
    let mut out = vec![0u8; N * N * 4];
    // 中の高さ 14 画素のうち、何画素を塗るか(0% でも 1 画素は出す)
    let filled = ((pct / 100.0 * 14.0).round() as usize).max(1);
    let bar = if pct >= 80.0 {
        [0xE5, 0x48, 0x4D, 0xFF]
    } else if pct >= 50.0 {
        [0xF0, 0xA0, 0x30, 0xFF]
    } else {
        [0x3C, 0xC8, 0xA0, 0xFF]
    };
    for y in 0..N {
        for x in 0..N {
            let i = (y * N + x) * 4;
            let edge = x == 0 || y == 0 || x == N - 1 || y == N - 1;
            let px = if edge {
                [0x9A, 0xA6, 0xB2, 0xFF]
            } else if (3..13).contains(&x) && N - 1 - y <= filled {
                bar
            } else {
                [0x1E, 0x2A, 0x38, 0xFF]
            };
            out[i..i + 4].copy_from_slice(&px);
        }
    }
    out
}
