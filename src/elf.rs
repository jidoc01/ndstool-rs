use std::{fs, io, path::Path};

const PT_LOAD: u32 = 1;
const PF_OVERLAY: u32 = 0x0020_0000;

pub(crate) struct LoadedImage {
    pub(crate) data: Vec<u8>,
    pub(crate) entry: u32,
    pub(crate) ram: u32,
    pub(crate) overlays: Vec<Overlay>,
}

pub(crate) struct Overlay {
    pub(crate) ram: u32,
    pub(crate) load_size: u32,
    pub(crate) bss_size: u32,
    pub(crate) ctors_start: u32,
    pub(crate) ctors_end: u32,
    pub(crate) data: Vec<u8>,
}

fn u16le(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}

fn u32le(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
}

fn raw(bytes: Vec<u8>, default_entry: u32, default_ram: u32) -> LoadedImage {
    let bytes = if bytes.len() >= 12
        && u32::from_le_bytes(bytes[bytes.len() - 12..bytes.len() - 8].try_into().unwrap())
            == 0xdec0_0621
    {
        bytes[..bytes.len() - 12].to_vec()
    } else {
        bytes
    };
    LoadedImage {
        data: bytes,
        entry: default_entry,
        ram: default_ram,
        overlays: Vec::new(),
    }
}

pub(crate) fn load(path: &Path, default_entry: u32, default_ram: u32) -> io::Result<LoadedImage> {
    let bytes = fs::read(path)?;
    if bytes.len() < 52 || &bytes[..4] != b"\x7fELF" {
        return Ok(raw(bytes, default_entry, default_ram));
    }
    if bytes[4] != 1 || bytes[5] != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "only ELF32 little-endian files are supported",
        ));
    }
    if u16le(&bytes, 16) != 2 || u16le(&bytes, 18) != 40 || u32le(&bytes, 20) != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ELF must be an ARM executable",
        ));
    }
    let entry_vaddr = u32le(&bytes, 24);
    let phoff = u32le(&bytes, 28) as usize;
    let entsz = u16le(&bytes, 42) as usize;
    let count = u16le(&bytes, 44) as usize;
    if entsz < 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ELF program header size is invalid",
        ));
    }

    let mut normal = Vec::new();
    let mut overlay_segments = Vec::new();
    let mut entry = None;
    for i in 0..count {
        let p = phoff
            .checked_add(i.checked_mul(entsz).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "ELF program header overflow")
            })?)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "ELF program header overflow")
            })?;
        if p.checked_add(32).is_none() || p + 32 > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ELF program header is truncated",
            ));
        }
        if u32le(&bytes, p) != PT_LOAD {
            continue;
        }
        let flags = u32le(&bytes, p + 24);
        let file_offset = u32le(&bytes, p + 4) as usize;
        let vaddr = u32le(&bytes, p + 8);
        let paddr = u32le(&bytes, p + 12);
        let filesz = u32le(&bytes, p + 16) as usize;
        let memsz = u32le(&bytes, p + 20);
        let end = file_offset.checked_add(filesz).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "ELF segment size overflow")
        })?;
        if end > bytes.len() || memsz < filesz as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ELF segment is outside the file or has invalid sizes",
            ));
        }
        if flags & PF_OVERLAY != 0 {
            overlay_segments.push((file_offset, vaddr, filesz, memsz));
            continue;
        }
        if entry.is_none()
            && filesz != 0
            && entry_vaddr >= vaddr
            && entry_vaddr - vaddr < filesz as u32
        {
            entry = Some(paddr + (entry_vaddr - vaddr));
        }
        if filesz != 0 {
            normal.push((file_offset, paddr, filesz));
        }
    }
    if normal.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ELF contains no loadable program data",
        ));
    }
    normal.sort_by_key(|(_, addr, _)| *addr);
    let ram = normal[0].1;
    let mut data = Vec::new();
    let mut expected = ram;
    for (offset, address, size) in normal {
        if address != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ELF load segments are not contiguous at 0x{address:08X}"),
            ));
        }
        data.extend_from_slice(&bytes[offset..offset + size]);
        expected = expected.saturating_add(size as u32);
    }

    let mut overlays = Vec::new();
    if let Some((offset, _, size, _)) = overlay_segments.first().copied() {
        if size == 0 || size % 12 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ELF overlay table has an invalid size",
            ));
        }
        for i in 0..(size / 12) {
            let p = offset + i * 12;
            overlays.push(Overlay {
                ram: 0,
                load_size: 0,
                bss_size: 0,
                ctors_start: u32le(&bytes, p),
                ctors_end: u32le(&bytes, p + 4),
                data: Vec::new(),
            });
        }
        for (index, (payload_offset, vaddr, filesz, memsz)) in
            overlay_segments.into_iter().skip(1).enumerate()
        {
            if index >= overlays.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "ELF contains too many overlay segments",
                ));
            }
            overlays[index].ram = vaddr;
            overlays[index].load_size = filesz as u32;
            overlays[index].bss_size = memsz - filesz as u32;
            overlays[index].data = bytes[payload_offset..payload_offset + filesz].to_vec();
        }
    }
    Ok(LoadedImage {
        data,
        entry: entry.unwrap_or(entry_vaddr),
        ram,
        overlays,
    })
}
