use crate::{banner, elf, filesystem, header};
use std::{
    fs,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

pub(crate) fn extract_range(rom: &Path, out: &Path, offset: u32, size: u32) -> io::Result<()> {
    let mut f = File::open(rom)?;
    f.seek(SeekFrom::Start(offset as u64))?;
    let mut buf = vec![0; size as usize];
    f.read_exact(&mut buf)?;
    fs::write(out, buf)
}

pub(crate) fn extract_overlays(
    rom: &Path,
    table_offset: u32,
    table_size: u32,
    fat_offset: u32,
    out_dir: &Path,
) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;
    let bytes = fs::read(rom)?;
    for p in (table_offset as usize..(table_offset + table_size) as usize).step_by(32) {
        if p + 32 > bytes.len() {
            break;
        }
        let id = u32::from_le_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]);
        let file_id =
            u32::from_le_bytes([bytes[p + 24], bytes[p + 25], bytes[p + 26], bytes[p + 27]])
                as usize;
        let fat = fat_offset as usize + file_id * 8;
        if fat + 8 > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "overlay file ID is outside FAT",
            ));
        }
        let start = u32::from_le_bytes([bytes[fat], bytes[fat + 1], bytes[fat + 2], bytes[fat + 3]])
            as usize;
        let end = u32::from_le_bytes([
            bytes[fat + 4],
            bytes[fat + 5],
            bytes[fat + 6],
            bytes[fat + 7],
        ]) as usize;
        if start > end || end > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "overlay FAT range is invalid",
            ));
        }
        fs::write(
            out_dir.join(format!("overlay_{id:04}.bin")),
            &bytes[start..end],
        )?;
    }
    Ok(())
}

fn put32(data: &mut [u8], p: usize, v: u32) {
    data[p..p + 4].copy_from_slice(&v.to_le_bytes());
}

fn align(value: usize, boundary: usize) -> usize {
    (value + boundary - 1) & !(boundary - 1)
}

fn write_overlays(
    rom: &mut Vec<u8>,
    overlays: &[elf::Overlay],
    table_offset_field: &mut u32,
    table_size_field: &mut u32,
    next_file_id: &mut u16,
    fat: &mut Vec<(u32, u32)>,
) -> io::Result<()> {
    if overlays.is_empty() {
        return Ok(());
    }
    let table_offset = align(rom.len(), 0x200);
    let table_size = overlays
        .len()
        .checked_mul(32)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "overlay table is too large"))?;
    rom.resize(table_offset + table_size, 0xff);
    let mut cursor = table_offset + table_size;
    for (index, overlay) in overlays.iter().enumerate() {
        let file_id = *next_file_id as u32;
        *next_file_id = next_file_id
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "too many overlay files"))?;
        let start = align(cursor, 0x200);
        rom.resize(start, 0xff);
        rom.extend_from_slice(&overlay.data);
        cursor = start + overlay.data.len();
        fat.push((start as u32, cursor as u32));

        let p = table_offset + index * 32;
        put32(rom, p, index as u32);
        put32(rom, p + 4, overlay.ram);
        put32(rom, p + 8, overlay.load_size);
        put32(rom, p + 12, overlay.bss_size);
        put32(rom, p + 16, overlay.ctors_start);
        put32(rom, p + 20, overlay.ctors_end);
        put32(rom, p + 24, file_id);
        put32(rom, p + 28, 0);
    }
    *table_offset_field = table_offset as u32;
    *table_size_field = table_size as u32;
    Ok(())
}

fn write_external_overlays(
    rom: &mut Vec<u8>,
    table_path: &Path,
    overlay_root: Option<&Path>,
    table_offset_field: &mut u32,
    table_size_field: &mut u32,
    next_file_id: &mut u16,
    fat: &mut Vec<(u32, u32)>,
) -> io::Result<()> {
    let table = fs::read(table_path)?;
    if table.is_empty() || table.len() % 32 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "overlay table size must be a non-zero multiple of 32",
        ));
    }
    let root = overlay_root.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "an overlay directory is required with -y9 or -y7",
        )
    })?;
    let table_offset = align(rom.len(), 0x200);
    rom.resize(table_offset + table.len(), 0xff);
    rom[table_offset..table_offset + table.len()].copy_from_slice(&table);
    let mut cursor = table_offset + table.len();
    for p in (0..table.len()).step_by(32) {
        let id = u32::from_le_bytes(table[p..p + 4].try_into().unwrap());
        let file_id = *next_file_id as u32;
        *next_file_id = next_file_id
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "too many overlay files"))?;
        let input = root.join(format!("overlay_{id:04}.bin"));
        let payload = fs::read(&input).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to read overlay {}: {error}", input.display()),
            )
        })?;
        let start = align(cursor, 0x200);
        rom.resize(start, 0xff);
        rom.extend_from_slice(&payload);
        cursor = start + payload.len();
        fat.push((start as u32, cursor as u32));
        put32(rom, table_offset + p + 24, file_id);
    }
    *table_offset_field = table_offset as u32;
    *table_size_field = table.len() as u32;
    Ok(())
}

pub(crate) fn create_with_tree(
    out: &Path,
    arm9_path: &Path,
    arm7_path: &Path,
    data_root: Option<&Path>,
    banner_path: Option<&Path>,
    logo_path: Option<&Path>,
    arm9_overlay_table: Option<&Path>,
    arm7_overlay_table: Option<&Path>,
    overlay_root: Option<&Path>,
) -> io::Result<()> {
    let arm9 = elf::load(arm9_path, 0x02000000, 0x02000000)?;
    let arm7 = elf::load(arm7_path, 0x037f8000, 0x037f8000)?;
    let arm9_offset = 0x200usize;
    let mut rom = vec![0xffu8; arm9_offset + arm9.data.len()];
    rom[..0x200].fill(0);
    rom[0..8].copy_from_slice(b"NDSTOOL ");
    rom[12..16].copy_from_slice(b"####");
    rom[16..18].copy_from_slice(b"01");
    if let Some(path) = logo_path {
        let logo = fs::read(path)?;
        if logo.len() != 156 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "raw logo must be exactly 156 bytes",
            ));
        }
        rom[0xc0..0xc0 + 156].copy_from_slice(&logo);
    }
    put32(&mut rom, 0x20, arm9_offset as u32);
    put32(&mut rom, 0x24, arm9.entry);
    put32(&mut rom, 0x28, arm9.ram);
    put32(&mut rom, 0x2c, arm9.data.len() as u32);
    rom[arm9_offset..arm9_offset + arm9.data.len()].copy_from_slice(&arm9.data);

    let mut arm9_overlay_offset = 0;
    let mut arm9_overlay_size = 0;
    let mut arm7_overlay_offset = 0;
    let mut arm7_overlay_size = 0;
    let mut next_file_id = 0u16;
    let mut overlay_fat = Vec::new();
    write_overlays(
        &mut rom,
        &arm9.overlays,
        &mut arm9_overlay_offset,
        &mut arm9_overlay_size,
        &mut next_file_id,
        &mut overlay_fat,
    )?;
    if arm9_overlay_table.is_some() && !arm9.overlays.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot combine ARM9 ELF overlays and an external overlay table",
        ));
    }
    if let Some(table) = arm9_overlay_table {
        write_external_overlays(
            &mut rom,
            table,
            overlay_root,
            &mut arm9_overlay_offset,
            &mut arm9_overlay_size,
            &mut next_file_id,
            &mut overlay_fat,
        )?;
    }

    let arm7_offset = align(rom.len().max(0x8000), 0x200);
    rom.resize(arm7_offset + arm7.data.len(), 0xff);
    put32(&mut rom, 0x30, arm7_offset as u32);
    put32(&mut rom, 0x34, arm7.entry);
    put32(&mut rom, 0x38, arm7.ram);
    put32(&mut rom, 0x3c, arm7.data.len() as u32);
    rom[arm7_offset..arm7_offset + arm7.data.len()].copy_from_slice(&arm7.data);
    write_overlays(
        &mut rom,
        &arm7.overlays,
        &mut arm7_overlay_offset,
        &mut arm7_overlay_size,
        &mut next_file_id,
        &mut overlay_fat,
    )?;
    if arm7_overlay_table.is_some() && !arm7.overlays.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot combine ARM7 ELF overlays and an external overlay table",
        ));
    }
    if let Some(table) = arm7_overlay_table {
        write_external_overlays(
            &mut rom,
            table,
            overlay_root,
            &mut arm7_overlay_offset,
            &mut arm7_overlay_size,
            &mut next_file_id,
            &mut overlay_fat,
        )?;
    }
    put32(&mut rom, 0x50, arm9_overlay_offset);
    put32(&mut rom, 0x54, arm9_overlay_size);
    put32(&mut rom, 0x58, arm7_overlay_offset);
    put32(&mut rom, 0x5c, arm7_overlay_size);
    if let Some(path) = banner_path {
        let banner = banner::load(path)?;
        if banner.is_empty() || banner.len() > 0x23c0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "banner size is invalid",
            ));
        }
        let offset = (rom.len() + 0x1ff) & !0x1ff;
        rom.resize(offset, 0xff);
        rom.extend_from_slice(&banner);
        put32(&mut rom, 0x68, offset as u32);
    }
    let base = align(rom.len(), 0x200) as u32;
    let image = match data_root {
        Some(root) => filesystem::build_image(root, base, next_file_id)?,
        None => filesystem::empty_image(next_file_id),
    };
    rom.resize(base as usize, 0xff);
    rom.extend_from_slice(&image.data);
    let fnt_offset = rom.len() as u32;
    rom.extend_from_slice(&image.fnt);
    let fat_offset = ((rom.len() + 3) & !3) as u32;
    rom.resize(fat_offset as usize, 0xff);
    for (start, end) in overlay_fat {
        rom.extend_from_slice(&start.to_le_bytes());
        rom.extend_from_slice(&end.to_le_bytes());
    }
    rom.extend_from_slice(&image.fat);
    put32(&mut rom, 0x40, fnt_offset);
    put32(&mut rom, 0x44, image.fnt.len() as u32);
    put32(&mut rom, 0x48, fat_offset);
    put32(
        &mut rom,
        0x4c,
        (image.fat.len() + (next_file_id as usize) * 8) as u32,
    );
    let size = rom.len().next_power_of_two().max(0x200);
    rom.resize(size, 0xff);
    rom[0x14] = (size.trailing_zeros() as u8).saturating_sub(17);
    let crc = header::crc16(&rom[..0x15e]);
    rom[0x15e..0x160].copy_from_slice(&crc.to_le_bytes());
    fs::write(out, rom)
}
