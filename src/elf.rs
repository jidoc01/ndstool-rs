use std::{fs, io, path::Path};

pub(crate) struct LoadedImage {
    pub(crate) data: Vec<u8>,
    pub(crate) entry: u32,
    pub(crate) ram: u32,
}
fn u16le(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}
fn u32le(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
}
pub(crate) fn load(path: &Path, default_entry: u32, default_ram: u32) -> io::Result<LoadedImage> {
    let bytes = fs::read(path)?;
    if bytes.len() < 52 || &bytes[..4] != b"\x7fELF" {
        return Ok(LoadedImage {
            data: bytes,
            entry: default_entry,
            ram: default_ram,
        });
    }
    if bytes[4] != 1 || bytes[5] != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "only ELF32 little-endian files are supported",
        ));
    }
    let entry = u32le(&bytes, 24);
    let phoff = u32le(&bytes, 28) as usize;
    let entsz = u16le(&bytes, 42) as usize;
    let count = u16le(&bytes, 44) as usize;
    let mut segs = Vec::new();
    let mut low = u32::MAX;
    let mut high = 0u32;
    for i in 0..count {
        let p = phoff + i * entsz;
        if p + 32 > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ELF program header is truncated",
            ));
        }
        if u32le(&bytes, p) != 1 {
            continue;
        }
        let off = u32le(&bytes, p + 4) as usize;
        let addr = u32le(&bytes, p + 8);
        let filesz = u32le(&bytes, p + 16) as usize;
        let memsz = u32le(&bytes, p + 20);
        if off.checked_add(filesz).is_none() || off + filesz > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ELF segment is outside the file",
            ));
        }
        low = low.min(addr);
        high = high.max(addr.saturating_add(memsz));
        segs.push((off, addr, filesz));
    }
    if segs.is_empty() || high <= low {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ELF contains no loadable segments",
        ));
    }
    let mut data = vec![0; (high - low) as usize];
    for (off, addr, size) in segs {
        let start = (addr - low) as usize;
        data[start..start + size].copy_from_slice(&bytes[off..off + size]);
    }
    Ok(LoadedImage {
        data,
        entry,
        ram: low,
    })
}
