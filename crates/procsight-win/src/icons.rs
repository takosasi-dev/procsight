//! exe のアイコン(担当 B。FR-8・D-33)。SHGetFileInfoW の小さいアイコンを 16×16 の RGBA にする。
//! DrawIconEx で 16×16 の 32 ビットの DIB に描いて読む。アルファを持たないアイコンはマスクから補う。

use crate::util::{pcwstr, wide};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    HDC, SelectObject,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGetFileInfoW};
use windows::Win32::UI::WindowsAndMessaging::{DI_FLAGS, DI_MASK, DI_NORMAL, DestroyIcon, DrawIconEx, HICON};

const SIZE: i32 = 16;
const BYTES: usize = (SIZE * SIZE * 4) as usize;

/// 16 × 16 × 4 バイト(RGBA、上の行から。乗算済みでないアルファ)。取れなければ None
pub fn icon_rgba(path: &str) -> Option<Vec<u8>> {
    let _com = crate::shell::Com::init();
    let p = wide(path);
    let mut sfi = SHFILEINFOW::default();
    // SAFETY: p は 0 で終わる。sfi はこの関数の中の構造体で、その大きさを渡す。返ったアイコンは下で DestroyIcon する
    let r = unsafe {
        SHGetFileInfoW(
            pcwstr(&p),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut sfi),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        )
    };
    if r == 0 || sfi.hIcon.is_invalid() {
        return None;
    }
    let out = render(sfi.hIcon);
    // SAFETY: SHGetFileInfoW が作ったアイコンを 1 回だけ壊す
    unsafe {
        let _ = DestroyIcon(sfi.hIcon);
    }
    out
}

/// 16×16 の DIB に描いて BGRA を読む
fn draw(dc: HDC, icon: HICON, flags: DI_FLAGS) -> Option<Vec<u8>> {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE,
            biHeight: -SIZE, // 上の行から
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: bmi はこの関数の中の構造体。返ったビットマップは最後に DeleteObject で壊し、bits はその間だけ読む
    unsafe {
        let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(dc, bmp.into());
        let ok = DrawIconEx(dc, 0, 0, icon, SIZE, SIZE, 0, None, flags).is_ok() && !bits.is_null();
        let out = ok.then(|| std::slice::from_raw_parts(bits as *const u8, BYTES).to_vec());
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        out
    }
}

fn render(icon: HICON) -> Option<Vec<u8>> {
    // SAFETY: 画面に結び付かない DC を作り、最後に DeleteDC で壊す
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.is_invalid() {
        return None;
    }
    let color = draw(dc, icon, DI_NORMAL);
    let mask = color.as_ref().filter(|c| c.as_chunks::<4>().0.iter().all(|p| p[3] == 0)).and_then(|_| draw(dc, icon, DI_MASK));
    // SAFETY: 上で作った DC を 1 回だけ壊す
    unsafe {
        let _ = DeleteDC(dc);
    }
    Some(to_rgba(&color?, mask.as_deref()))
}

/// BGRA(DrawIconEx はアルファ付きのアイコンを乗算済みで描く)を、乗算済みでない RGBA に。
/// `mask` があれば(アルファの無いアイコン)、マスクの黒い所を不透明にする
fn to_rgba(bgra: &[u8], mask: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(BYTES);
    for (i, p) in bgra.as_chunks::<4>().0.iter().enumerate() {
        let a = match mask {
            Some(m) => if m[i * 4] == 0 && m[i * 4 + 1] == 0 && m[i * 4 + 2] == 0 { 255 } else { 0 },
            None => p[3],
        };
        let un = |c: u8| -> u8 {
            match (mask.is_some(), a) {
                (_, 0) => 0,
                (true, _) | (false, 255) => c,
                (false, a) => ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
            }
        };
        out.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0]), a]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notepad_icon() {
        let p = format!("{}\\System32\\notepad.exe", crate::util::system_root());
        let v = icon_rgba(&p).expect("アイコン");
        assert_eq!(v.len(), BYTES);
        assert!(v.as_chunks::<4>().0.iter().any(|p| p[3] != 0), "全部透明");
    }

    #[test]
    fn conversion() {
        // 乗算済みの半透明の赤 (B,G,R,A) = (0,0,64,128) → (128,0,0,128)
        assert_eq!(&to_rgba(&[0, 0, 64, 128].repeat(256), None)[..4], &[128, 0, 0, 128]);
        // マスクが黒なら不透明、白なら透明
        let m = [[0u8, 0, 0, 0], [255, 255, 255, 0]].concat().repeat(128);
        let v = to_rgba(&[10, 20, 30, 0].repeat(256), Some(&m));
        assert_eq!(&v[..8], &[30, 20, 10, 255, 0, 0, 0, 0]);
    }
}
