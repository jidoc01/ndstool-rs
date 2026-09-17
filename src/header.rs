use crate::model::Header;
use std::{
    fs,
    fs::File,
    io::{self, Read},
    path::Path,
};

fn le32(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
}
fn text(b: &[u8]) -> String {
    b.iter()
        .take_while(|&&x| x != 0)
        .map(|&x| x as char)
        .collect()
}

pub(crate) fn read(path: &Path) -> io::Result<Header> {
    let mut f = File::open(path)?;
    let mut b = [0u8; 0x200];
    f.read_exact(&mut b)?;
    Ok(Header {
        title: text(&b[0..12]),
        game_code: text(&b[12..16]),
        maker_code: text(&b[16..18]),
        unit_code: b[0x12],
        device_capacity: b[0x14],
        banner_offset: le32(&b, 0x68),
        arm9_offset: le32(&b, 0x20),
        arm9_entry: le32(&b, 0x24),
        arm9_ram: le32(&b, 0x28),
        arm9_size: le32(&b, 0x2c),
        arm7_offset: le32(&b, 0x30),
        arm7_entry: le32(&b, 0x34),
        arm7_ram: le32(&b, 0x38),
        arm7_size: le32(&b, 0x3c),
        fnt_offset: le32(&b, 0x40),
        fnt_size: le32(&b, 0x44),
        fat_offset: le32(&b, 0x48),
        fat_size: le32(&b, 0x4c),
        arm9_overlay_offset: le32(&b, 0x50),
        arm9_overlay_size: le32(&b, 0x54),
        arm7_overlay_offset: le32(&b, 0x58),
        arm7_overlay_size: le32(&b, 0x5c),
    })
}
pub(crate) fn print_info(h: &Header) {
    println!("Title       : {}\nGame code   : {}\nMaker code  : {}\nUnit code   : 0x{:02X}\nDevice cap  : 0x{:02X}\nARM9        : 0x{:08X} ({} bytes, entry 0x{:08X}, RAM 0x{:08X})\nARM7        : 0x{:08X} ({} bytes, entry 0x{:08X}, RAM 0x{:08X})\nFNT         : 0x{:08X} ({} bytes)\nFAT         : 0x{:08X} ({} bytes)\nBanner      : 0x{:08X}", h.title,h.game_code,h.maker_code,h.unit_code,h.device_capacity,h.arm9_offset,h.arm9_size,h.arm9_entry,h.arm9_ram,h.arm7_offset,h.arm7_size,h.arm7_entry,h.arm7_ram,h.fnt_offset,h.fnt_size,h.fat_offset,h.fat_size,h.banner_offset);
}
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}
pub(crate) fn fix_crc(path: &Path) -> io::Result<()> {
    let mut data = fs::read(path)?;
    if data.len() < 0x160 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ROM is smaller than an NDS header",
        ));
    }
    let crc = crc16(&data[..0x15e]);
    data[0x15e..0x160].copy_from_slice(&crc.to_le_bytes());
    fs::write(path, data)
}
