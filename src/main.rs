use std::{env, fs, io, path::PathBuf};

mod banner;
mod crypto;
mod elf;
mod filesystem;
mod header;
mod model;
mod rom;

fn main() -> io::Result<()> {
    let a: Vec<String> = env::args().skip(1).collect();
    if a.is_empty() {
        eprintln!("usage: ndstool-rs -i ROM.nds | -l ROM.nds | -x ROM.nds [-d DIR] [-9 FILE] [-7 FILE] [-b FILE] [-o FILE]");
        return Ok(());
    };
    let mode = a[0].as_str();
    if !matches!(mode, "-i" | "-l" | "-x" | "-f" | "-c") && !mode.starts_with("-s") {
        eprintln!("unknown action: {mode}");
        return Ok(());
    }
    if a.len() < 2 {
        eprintln!("missing ROM filename");
        return Ok(());
    }
    let rom = PathBuf::from(&a[1]);
    if mode.starts_with("-s") {
        let option = mode.chars().nth(2).unwrap_or('d');
        crypto::transform_secure_area(&rom, option)?;
        println!("Updated secure area: {}", rom.display());
        return Ok(());
    }
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
