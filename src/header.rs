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
    let mut b = [0u8; 0x220];
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
        dsi_flags: b[0x1e],
        dsi9_offset: le32(&b, 0x1c0),
        dsi9_ram: le32(&b, 0x1c8),
        dsi9_size: le32(&b, 0x1cc),
        dsi7_offset: le32(&b, 0x1d0),
        dsi7_ram: le32(&b, 0x1d8),
        dsi7_size: le32(&b, 0x1dc),
        banner_size: le32(&b, 0x208),
        total_rom_size: le32(&b, 0x210),
        region_flags: le32(&b, 0x1a0),
        access_control: le32(&b, 0x1a4),
        scfg_ext_mask: le32(&b, 0x1a8),
    })
}
pub(crate) fn print_info(h: &Header) {
    println!("Title       : {}\nGame code   : {}\nMaker code  : {}\nUnit code   : 0x{:02X}\nDevice cap  : 0x{:02X}\nARM9        : 0x{:08X} ({} bytes, entry 0x{:08X}, RAM 0x{:08X})\nARM7        : 0x{:08X} ({} bytes, entry 0x{:08X}, RAM 0x{:08X})\nFNT         : 0x{:08X} ({} bytes)\nFAT         : 0x{:08X} ({} bytes)\nBanner      : 0x{:08X}", h.title,h.game_code,h.maker_code,h.unit_code,h.device_capacity,h.arm9_offset,h.arm9_size,h.arm9_entry,h.arm9_ram,h.arm7_offset,h.arm7_size,h.arm7_entry,h.arm7_ram,h.fnt_offset,h.fnt_size,h.fat_offset,h.fat_size,h.banner_offset);
    if h.unit_code & 2 != 0 || h.dsi9_offset != 0 || h.dsi7_offset != 0 {
        println!("DSi flags   : 0x{:02X}\nDSi9        : 0x{:08X} ({} bytes, RAM 0x{:08X})\nDSi7        : 0x{:08X} ({} bytes, RAM 0x{:08X})\nBanner size : 0x{:08X}\nTotal ROM   : 0x{:08X}\nRegion flags: 0x{:08X}\nAccess      : 0x{:08X}\nSCFG mask   : 0x{:08X}", h.dsi_flags, h.dsi9_offset, h.dsi9_size, h.dsi9_ram, h.dsi7_offset, h.dsi7_size, h.dsi7_ram, h.banner_size, h.total_rom_size, h.region_flags, h.access_control, h.scfg_ext_mask);
    }
}
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for &byte in data {
        crc = (crc >> 8) ^ crc16_byte(byte ^ (crc as u8));
    }
    crc
}

fn crc16_byte(index: u8) -> u16 {
    let mut value = index as u16;
    for _ in 0..8 {
        value = if value & 1 != 0 {
            (value >> 1) ^ 0xA001
        } else {
            value >> 1
        };
    }
    value
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

#[cfg(test)]
mod tests {
    use super::crc16;

    #[test]
    fn crc16_matches_ndstool_algorithm() {
        assert_eq!(crc16(b"123456789"), 0x4B37);
    }
}
