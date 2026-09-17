use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

mod banner;
mod crypto;
mod elf;
mod filesystem;
mod header;
mod model;
mod rom;
fn put32(data: &mut [u8], p: usize, v: u32) {
    data[p..p + 4].copy_from_slice(&v.to_le_bytes());
}

fn create_basic_rom(
    out: &Path,
    arm9_path: &Path,
    arm7_path: &Path,
    data_root: Option<&Path>,
) -> io::Result<()> {
    let arm9 = fs::read(arm9_path)?;
    let arm7 = fs::read(arm7_path)?;
    let arm9_offset = 0x200u32;
    let arm7_offset = 0x8000u32;
    let end = (arm7_offset as usize + arm7.len()).max(arm9_offset as usize + arm9.len());
    let mut rom = vec![0xffu8; end];
    rom[..0x200].fill(0);
    rom[0..8].copy_from_slice(b"NDSTOOL ");
    rom[12..16].copy_from_slice(b"####");
    rom[16..18].copy_from_slice(b"01");
    put32(&mut rom, 0x20, arm9_offset);
    put32(&mut rom, 0x24, 0x02000000);
    put32(&mut rom, 0x28, 0x02000000);
    put32(&mut rom, 0x2c, arm9.len() as u32);
    put32(&mut rom, 0x30, arm7_offset);
    put32(&mut rom, 0x34, 0x037f8000);
    put32(&mut rom, 0x38, 0x037f8000);
    put32(&mut rom, 0x3c, arm7.len() as u32);
    rom[arm9_offset as usize..arm9_offset as usize + arm9.len()].copy_from_slice(&arm9);
    rom[arm7_offset as usize..arm7_offset as usize + arm7.len()].copy_from_slice(&arm7);
    let mut files = Vec::new();
    if let Some(root) = data_root {
        for item in fs::read_dir(root)? {
            let item = item?;
            if item.file_type()?.is_file() {
                let name = item.file_name().to_string_lossy().into_owned();
                if name.len() > 127 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "data filename is longer than 127 bytes",
                    ));
                }
                files.push((name, fs::read(item.path())?));
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let mut cursor = (rom.len() + 0x1ff) & !0x1ff;
    let mut ranges = Vec::new();
    for (_, data) in &files {
        cursor = (cursor + 0x1ff) & !0x1ff;
        let start = cursor as u32;
        if rom.len() < cursor {
            rom.resize(cursor, 0xff);
        }
        rom.extend_from_slice(data);
        cursor += data.len();
        ranges.push((start, cursor as u32));
    }
    let fnt_offset = if files.is_empty() { 0 } else { cursor as u32 };
    let fnt_size = if files.is_empty() {
        0
    } else {
        8 + files.iter().map(|(n, _)| 1 + n.len()).sum::<usize>() + 1
    };
    if !files.is_empty() {
        let mut fnt = vec![0u8; fnt_size];
        put32(&mut fnt, 0, 8);
        fnt[4..6].copy_from_slice(&0u16.to_le_bytes());
        fnt[6..8].copy_from_slice(&0xf000u16.to_le_bytes());
        let mut p = 8;
        for (name, _) in &files {
            fnt[p] = name.len() as u8;
            p += 1;
            fnt[p..p + name.len()].copy_from_slice(name.as_bytes());
            p += name.len();
        }
        fnt[p] = 0;
        rom.extend_from_slice(&fnt);
        let fat_offset = (rom.len() + 3) & !3;
        rom.resize(fat_offset, 0xff);
        let mut fat = Vec::with_capacity(ranges.len() * 8);
        for (start, end) in ranges {
            fat.extend_from_slice(&start.to_le_bytes());
            fat.extend_from_slice(&end.to_le_bytes());
        }
        rom.extend_from_slice(&fat);
        put32(&mut rom, 0x40, fnt_offset);
        put32(&mut rom, 0x44, fnt_size as u32);
        put32(&mut rom, 0x48, fat_offset as u32);
        put32(&mut rom, 0x4c, fat.len() as u32);
    }
    let size = rom.len().next_power_of_two().max(0x200);
    rom.resize(size, 0xff);
    rom[0x14] = (size.trailing_zeros() as u8).saturating_sub(17);
    let crc = header::crc16(&rom[..0x15e]);
    rom[0x15e..0x160].copy_from_slice(&crc.to_le_bytes());
    fs::write(out, rom)
}

fn main() -> io::Result<()> {
    let a: Vec<String> = env::args().skip(1).collect();
    if a.is_empty() {
        eprintln!("usage: ndstool-rs -i ROM.nds | -l ROM.nds | -x ROM.nds [-d DIR] [-9 FILE] [-7 FILE] [-b FILE] [-o FILE]");
        return Ok(());
    };
    let mode = a[0].as_str();
    if !matches!(mode, "-i" | "-l" | "-x" | "-f" | "-c") {
        eprintln!("unknown action: {mode}");
        return Ok(());
    }
    if a.len() < 2 {
        eprintln!("missing ROM filename");
        return Ok(());
    }
    let rom = PathBuf::from(&a[1]);
    if mode == "-c" {
        let mut arm9 = None;
        let mut arm7 = None;
        let mut data = None;
        let mut banner = None;
        let mut i = 2;
        while i < a.len() {
            match a[i].as_str() {
                "-9" => arm9 = a.get(i + 1).map(PathBuf::from),
                "-7" => arm7 = a.get(i + 1).map(PathBuf::from),
                "-d" => data = a.get(i + 1).map(PathBuf::from),
                "-t" | "-b" => banner = a.get(i + 1).map(PathBuf::from),
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown create option: {other}"),
                    ))
                }
            }
            i += 2;
        }
        let arm9 = arm9.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "-c requires -9 ARM9.bin")
        })?;
        let arm7 = arm7.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "-c requires -7 ARM7.bin")
        })?;
        if let Some(root) = data.as_deref() {
            rom::create_with_tree(&rom, &arm9, &arm7, Some(root), banner.as_deref())?;
        } else {
            rom::create_with_tree(&rom, &arm9, &arm7, None, banner.as_deref())?;
        }
        println!("Created {}", rom.display());
        return Ok(());
    }
    let h = header::read(&rom)?;
    if mode == "-f" {
        header::fix_crc(&rom)?;
        println!("Fixed header CRC: {}", rom.display());
        return Ok(());
    }
    if mode == "-i" {
        header::print_info(&h);
        return Ok(());
    };
    let entries = filesystem::read_entries(&rom, &h)?;
    if mode == "-l" {
        for e in entries {
            println!(
                "{:5} 0x{:08X} 0x{:08X} {:9} {}",
                e.id,
                e.start,
                e.end,
                e.end - e.start,
                e.path
            )
        }
        return Ok(());
    };
    let mut root = PathBuf::from(".");
    let mut arm9 = None;
    let mut arm7 = None;
    let mut banner = None;
    let mut logo = None;
    let mut overlays = None;
    let mut i = 2;
    while i < a.len() {
        let value = |idx: usize| -> io::Result<&str> {
            a.get(idx)
                .map(String::as_str)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing option value"))
        };
        match a[i].as_str() {
            "-d" => root = PathBuf::from(value(i + 1)?),
            "-9" => arm9 = Some(PathBuf::from(value(i + 1)?)),
            "-7" => arm7 = Some(PathBuf::from(value(i + 1)?)),
            "-b" | "-t" => banner = Some(PathBuf::from(value(i + 1)?)),
            "-o" => logo = Some(PathBuf::from(value(i + 1)?)),
            "-y" => overlays = Some(PathBuf::from(value(i + 1)?)),
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown extract option: {other}"),
                ))
            }
        }
        i += 2;
    }
    if arm9.is_none() && arm7.is_none() && banner.is_none() && logo.is_none() {
        fs::create_dir_all(&root)?;
    }
    if let Some(path) = arm9 {
        rom::extract_range(&rom, &path, h.arm9_offset, h.arm9_size)?;
    }
    if let Some(path) = arm7 {
        rom::extract_range(&rom, &path, h.arm7_offset, h.arm7_size)?;
    }
    if let Some(path) = banner {
        rom::extract_range(&rom, &path, h.banner_offset, 0x840)?;
    }
    if let Some(path) = logo {
        rom::extract_range(&rom, &path, 0xC0, 156)?;
    }
    if let Some(path) = overlays {
        rom::extract_overlays(
            &rom,
            h.arm9_overlay_offset,
            h.arm9_overlay_size,
            h.fat_offset,
            &path,
        )?;
        rom::extract_overlays(
            &rom,
            h.arm7_overlay_offset,
            h.arm7_overlay_size,
            h.fat_offset,
            &path,
        )?;
    }
    for e in entries {
        let dst = root.join(e.path.trim_start_matches('/'));
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p)?;
        }
        rom::extract_range(&rom, &dst, e.start, e.end - e.start)?;
    }
    Ok(())
}
