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

pub(crate) fn create_with_tree(
    out: &Path,
    arm9_path: &Path,
    arm7_path: &Path,
    data_root: Option<&Path>,
    banner_path: Option<&Path>,
) -> io::Result<()> {
    let arm9 = elf::load(arm9_path, 0x02000000, 0x02000000)?;
    let arm7 = elf::load(arm7_path, 0x037f8000, 0x037f8000)?;
    let arm9_offset = 0x200usize;
    let arm7_offset = 0x8000usize;
    let mut rom = vec![0xffu8; (arm7_offset + arm7.data.len()).max(arm9_offset + arm9.data.len())];
    rom[..0x200].fill(0);
    rom[0..8].copy_from_slice(b"NDSTOOL ");
    rom[12..16].copy_from_slice(b"####");
    rom[16..18].copy_from_slice(b"01");
    put32(&mut rom, 0x20, arm9_offset as u32);
    put32(&mut rom, 0x24, arm9.entry);
    put32(&mut rom, 0x28, arm9.ram);
    put32(&mut rom, 0x2c, arm9.data.len() as u32);
    put32(&mut rom, 0x30, arm7_offset as u32);
    put32(&mut rom, 0x34, arm7.entry);
    put32(&mut rom, 0x38, arm7.ram);
    put32(&mut rom, 0x3c, arm7.data.len() as u32);
    rom[arm9_offset..arm9_offset + arm9.data.len()].copy_from_slice(&arm9.data);
    rom[arm7_offset..arm7_offset + arm7.data.len()].copy_from_slice(&arm7.data);
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
    let base = ((rom.len() + 0x1ff) & !0x1ff) as u32;
    let Some(root) = data_root else {
        let size = rom.len().next_power_of_two().max(0x200);
        rom.resize(size, 0xff);
        rom[0x14] = (size.trailing_zeros() as u8).saturating_sub(17);
        let crc = header::crc16(&rom[..0x15e]);
        rom[0x15e..0x160].copy_from_slice(&crc.to_le_bytes());
        return fs::write(out, rom);
    };
    let image = filesystem::build_image(root, base)?;
    rom.resize(base as usize, 0xff);
    rom.extend_from_slice(&image.data);
    let fnt_offset = rom.len() as u32;
    rom.extend_from_slice(&image.fnt);
    let fat_offset = ((rom.len() + 3) & !3) as u32;
    rom.resize(fat_offset as usize, 0xff);
    rom.extend_from_slice(&image.fat);
    put32(&mut rom, 0x40, fnt_offset);
    put32(&mut rom, 0x44, image.fnt.len() as u32);
    put32(&mut rom, 0x48, fat_offset);
    put32(&mut rom, 0x4c, image.fat.len() as u32);
    let size = rom.len().next_power_of_two().max(0x200);
    rom.resize(size, 0xff);
    rom[0x14] = (size.trailing_zeros() as u8).saturating_sub(17);
    let crc = header::crc16(&rom[..0x15e]);
    rom[0x15e..0x160].copy_from_slice(&crc.to_le_bytes());
    fs::write(out, rom)
}
