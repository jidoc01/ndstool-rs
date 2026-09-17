use crate::header;
use std::{fs, io, path::Path};

pub(crate) fn load(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    if bytes.len() < 2 || &bytes[..2] != b"BM" {
        return Ok(bytes);
    }
    if bytes.len() < 54 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "BMP header is truncated",
        ));
    }
    let offset = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
    let width = i32::from_le_bytes(bytes[18..22].try_into().unwrap());
    let height_raw = i32::from_le_bytes(bytes[22..26].try_into().unwrap());
    let bpp = u16::from_le_bytes(bytes[28..30].try_into().unwrap());
    if width != 32 || height_raw.abs() != 32 || bpp != 24 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only 32x32 24-bit BMP banners are supported",
        ));
    }
    let top_down = height_raw < 0;
    let row_size = ((width as usize * 3 + 3) / 4) * 4;
    let mut pixels = vec![(0u8, 0u8, 0u8); 32 * 32];
    for y in 0..32 {
        let src_y = if top_down { y } else { 31 - y };
        for x in 0..32 {
            let p = offset + src_y * row_size + x * 3;
            if p + 3 > bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "BMP pixel data is truncated",
                ));
            }
            pixels[y * 32 + x] = (bytes[p + 2], bytes[p + 1], bytes[p]);
        }
    }
    let mut palette = Vec::new();
    for &(r, g, b) in &pixels {
        let color = (r >> 3, g >> 3, b >> 3);
        if !palette.contains(&color) {
            palette.push(color);
            if palette.len() == 16 {
                break;
            }
        }
    }
    while palette.len() < 16 {
        palette.push((0, 0, 0));
    }
    let mut out = vec![0u8; 0x840];
    out[0..2].copy_from_slice(&1u16.to_le_bytes());
    for y in 0..32 {
        for x in (0..32).step_by(2) {
            let mut nibbles = [0u8; 2];
            for (i, nib) in nibbles.iter_mut().enumerate() {
                let (r, g, b) = pixels[y * 32 + x + i];
                *nib = palette
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, &(pr, pg, pb))| {
                        let dr = r as i32 - (pr as i32 * 8);
                        let dg = g as i32 - (pg as i32 * 8);
                        let db = b as i32 - (pb as i32 * 8);
                        dr * dr + dg * dg + db * db
                    })
                    .map(|(idx, _)| idx as u8)
                    .unwrap_or(0);
            }
            let tile = ((y / 8) * 4 + x / 8) * 32 + (y % 8) * 4 + (x % 8) / 2;
            out[32 + tile] = nibbles[0] | (nibbles[1] << 4);
        }
    }
    for (i, &(r, g, b)) in palette.iter().enumerate() {
        let c = (r as u16) | ((g as u16) << 5) | ((b as u16) << 10);
        out[544 + i * 2..546 + i * 2].copy_from_slice(&c.to_le_bytes());
    }
    let crc = header::crc16(&out[32..]);
    out[2..4].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}
