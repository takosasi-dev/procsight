//! .ttc / .ttf の書体の一覧(番号と名前)を std だけで読む。name テーブルの英語の名前を使う。
//! procsight-uiprobe からも #[path] で同じファイルを読む(依存を足さないため)。

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// 1 つの書体(sfnt の先頭の位置)の名前。name テーブルの ID 4(フルネーム)を Windows の英語(0x409)で、無ければ ID 1
fn face_name(b: &[u8], sfnt: usize) -> Option<String> {
    let num_tables = be16(b, sfnt + 4)? as usize;
    let mut name_off = None;
    for t in 0..num_tables {
        let rec = sfnt + 12 + t * 16;
        if b.get(rec..rec + 4)? == b"name" {
            name_off = Some(be32(b, rec + 8)? as usize);
            break;
        }
    }
    let name = name_off?;
    let count = be16(b, name + 2)? as usize;
    let strings = name + be16(b, name + 4)? as usize;
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let r = name + 6 + i * 12;
        let (platform, lang, id) = (be16(b, r)?, be16(b, r + 4)?, be16(b, r + 6)?);
        let (len, off) = (be16(b, r + 8)? as usize, be16(b, r + 10)? as usize);
        if platform != 3 || lang != 0x409 || !(id == 1 || id == 4) {
            continue;
        }
        let raw = b.get(strings + off..strings + off + len)?;
        let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        let s = String::from_utf16_lossy(&units);
        let rank = if id == 4 { 2 } else { 1 };
        if best.as_ref().is_none_or(|(r, _)| rank > *r) {
            best = Some((rank, s));
        }
    }
    best.map(|(_, s)| s)
}

/// .ttc / .ttf / .otf の書体の一覧(番号, 名前)。読めなければ空
pub fn faces(bytes: &[u8]) -> Vec<(u32, String)> {
    if bytes.get(0..4) == Some(b"ttcf") {
        let n = be32(bytes, 8).unwrap_or(0).min(64);
        (0..n)
            .filter_map(|i| {
                let off = be32(bytes, 12 + i as usize * 4)? as usize;
                Some((i, face_name(bytes, off).unwrap_or_default()))
            })
            .collect()
    } else if bytes.len() > 12 {
        vec![(0, face_name(bytes, 0).unwrap_or_default())]
    } else {
        Vec::new()
    }
}
