use std::{env, fs, io, path::PathBuf};

mod banner;
mod crypto;
mod elf;
mod filesystem;
mod header;
mod logo;
mod model;
mod rom;

fn main() -> io::Result<()> {
    let a: Vec<String> = env::args().skip(1).collect();
    if a.is_empty() {
        eprintln!("usage: ndstool-rs -i ROM.nds | -l ROM.nds | -x ROM.nds [-d DIR] [-9 FILE] [-7 FILE] [-b FILE] [-o FILE] [-y9 FILE] [-y7 FILE]");
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
        let mut logo = None;
        let mut arm9_overlay_table = None;
        let mut arm7_overlay_table = None;
        let mut overlay_root = None;
        let mut game_code = None;
        let mut maker_code = None;
        let mut title = None;
        let mut header_template = None;
        let mut layout = filesystem::LayoutMode::Stable;
        let mut jobs = 1usize;
        let mut ignore_missing_overlays = false;
        let mut i = 2;
        while i < a.len() {
            let mut step = 2;
            match a[i].as_str() {
                "-9" => arm9 = a.get(i + 1).map(PathBuf::from),
                "-7" => arm7 = a.get(i + 1).map(PathBuf::from),
                "-d" => data = a.get(i + 1).map(PathBuf::from),
                "-t" | "-b" => banner = a.get(i + 1).map(PathBuf::from),
                "-o" => logo = a.get(i + 1).map(PathBuf::from),
                "-y9" => arm9_overlay_table = a.get(i + 1).map(PathBuf::from),
                "-y7" => arm7_overlay_table = a.get(i + 1).map(PathBuf::from),
                "-y" => overlay_root = a.get(i + 1).map(PathBuf::from),
                "-h" => header_template = a.get(i + 1).map(PathBuf::from),
                "--enable-random-layout" => {
                    // Random payload placement is opt-in because it changes
                    // ROM bytes and makes byte-for-byte rebuilds impossible.
                    layout = filesystem::LayoutMode::Random;
                    step = 1;
                }
                "--ignore-missing-overlays" => {
                    // This is deliberately opt-in: the original ndstool
                    // reports missing overlay files as an error.
                    ignore_missing_overlays = true;
                    step = 1;
                }
                "--parallel" => {
                    jobs = std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(1);
                    step = 1;
                }
                "--jobs" => {
                    jobs = a
                        .get(i + 1)
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "--jobs requires a number")
                        })?
                        .parse()
                        .map_err(|_| {
                            io::Error::new(io::ErrorKind::InvalidInput, "--jobs requires a number")
                        })?;
                    if jobs == 0 {
                        jobs = std::thread::available_parallelism()
                            .map(|n| n.get())
                            .unwrap_or(1);
                    }
                }
                "-g" => {
                    game_code = a.get(i + 1).map(String::as_str);
                    if i + 2 < a.len() && !a[i + 2].starts_with('-') {
                        maker_code = Some(a[i + 2].as_str());
                    }
                    if i + 3 < a.len() && !a[i + 3].starts_with('-') {
                        title = Some(a[i + 3].as_str());
                    }
                    i += maker_code.is_some() as usize + title.is_some() as usize;
                }
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown create option: {other}"),
                    ))
                }
            }
            i += step;
        }
        let arm9 = arm9.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "-c requires -9 ARM9.bin")
        })?;
        let arm7 = arm7.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "-c requires -7 ARM7.bin")
        })?;
        rom::create_with_tree(
            &rom,
            &arm9,
            &arm7,
            data.as_deref(),
            banner.as_deref(),
            logo.as_deref(),
            arm9_overlay_table.as_deref(),
            arm7_overlay_table.as_deref(),
            overlay_root.as_deref(),
            game_code,
            maker_code,
            title,
            header_template.as_deref(),
            layout,
            jobs,
            ignore_missing_overlays,
        )?;
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
    let mut arm9_overlay_table = None;
    let mut arm7_overlay_table = None;
    let mut overlays = None;
    let mut jobs = 1usize;
    let mut i = 2;
    while i < a.len() {
        let mut step = 2;
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
            "-y9" => arm9_overlay_table = Some(PathBuf::from(value(i + 1)?)),
            "-y7" => arm7_overlay_table = Some(PathBuf::from(value(i + 1)?)),
            "-y" => overlays = Some(PathBuf::from(value(i + 1)?)),
            "--parallel" => {
                jobs = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1);
                step = 1;
            }
            "--jobs" => {
                let value = value(i + 1)?;
                jobs = value.parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--jobs requires a number")
                })?;
                if jobs == 0 {
                    jobs = std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(1);
                }
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown extract option: {other}"),
                ))
            }
        }
        i += step;
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
        // A zero banner offset means the ROM has no banner. ndstool still
        // creates the requested output file, but leaves it empty.
        let size = if h.banner_offset == 0 {
            0
        } else if h.unit_code & 2 != 0 {
            // DSi-enhanced ROMs carry the banner size in the extended header;
            // a zero value is how ndstool represents a missing banner.
            h.banner_size
        } else {
            0x840
        };
        rom::extract_range(&rom, &path, h.banner_offset, size)?;
    }
    if let Some(path) = logo {
        rom::extract_range(&rom, &path, 0xC0, 156)?;
    }
    if let Some(path) = arm9_overlay_table {
        rom::extract_range(&rom, &path, h.arm9_overlay_offset, h.arm9_overlay_size)?;
    }
    if let Some(path) = arm7_overlay_table {
        rom::extract_range(&rom, &path, h.arm7_overlay_offset, h.arm7_overlay_size)?;
    }
    if let Some(path) = overlays {
        rom::extract_overlays(
            &rom,
            h.arm9_overlay_offset,
            h.arm9_overlay_size,
            h.fat_offset,
            &path,
            jobs,
        )?;
        rom::extract_overlays(
            &rom,
            h.arm7_overlay_offset,
            h.arm7_overlay_size,
            h.fat_offset,
            &path,
            jobs,
        )?;
    }
    rom::extract_entries(&rom, &entries, &root, jobs)?;
    Ok(())
}
