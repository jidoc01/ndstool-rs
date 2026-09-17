use crate::{banner, crypto, elf, filesystem, header, logo};
use std::{
    fs,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    thread,
};

struct CopyTask {
    path: std::path::PathBuf,
    start: usize,
    end: usize,
}

fn write_tasks(bytes: &[u8], tasks: Vec<CopyTask>, jobs: usize) -> io::Result<()> {
    if jobs <= 1 || tasks.len() <= 1 {
        for task in tasks {
            fs::write(task.path, &bytes[task.start..task.end])?;
        }
        return Ok(());
    }
    let worker_count = jobs.min(tasks.len());
    let chunk_size = tasks.len().div_ceil(worker_count);
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for chunk in tasks.chunks(chunk_size) {
            handles.push(scope.spawn(move || -> io::Result<()> {
                for task in chunk {
                    fs::write(&task.path, &bytes[task.start..task.end])?;
                }
                Ok(())
            }));
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| io::Error::other("parallel extraction worker panicked"))??;
        }
        Ok(())
    })
}

pub(crate) fn extract_entries(
    rom: &Path,
    entries: &[crate::model::Entry],
    root: &Path,
    jobs: usize,
) -> io::Result<()> {
    let bytes = fs::read(rom)?;
    let mut tasks = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = root.join(entry.path.trim_start_matches('/'));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let start = entry.start as usize;
        let end = entry.end as usize;
        if start > end || end > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid FAT range for {}", entry.path),
            ));
        }
        tasks.push(CopyTask { path, start, end });
    }
    write_tasks(&bytes, tasks, jobs)
}

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
    jobs: usize,
) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;
    let bytes = fs::read(rom)?;
    let mut tasks = Vec::new();
    for p in (table_offset as usize..(table_offset + table_size) as usize).step_by(32) {
        if p + 32 > bytes.len() {
            break;
        }
        let id = u32::from_le_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]);
        // ndstool's extractor addresses overlay payloads by overlay ID.  The
        // table's file-id field is metadata used by the builder, but is not
        // what the original dsextract path uses for lookup.
        let file_id = id as usize;
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
        tasks.push(CopyTask {
            path: out_dir.join(format!("overlay_{id:04}.bin")),
            start,
            end,
        });
    }
    write_tasks(&bytes, tasks, jobs)
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
        // ndstool's extracted overlay filenames use the overlay ID as the
        // FAT slot. Preserve the table's file-id field verbatim, but rebuild
        // the payload layout from those filenames so an extract/recreate
        // round trip reproduces the original ROM bytes.
        let file_index = usize::try_from(id).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "overlay file ID is invalid")
        })?;
        if file_index >= 0x10000 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "overlay file ID is too large",
            ));
        }
        *next_file_id = (*next_file_id).max((file_index + 1) as u16);
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
        if fat.len() <= file_index {
            fat.resize(file_index + 1, (0, 0));
        }
        fat[file_index] = (start as u32, cursor as u32);
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
    game_code: Option<&str>,
    maker_code: Option<&str>,
    title: Option<&str>,
    header_template: Option<&Path>,
) -> io::Result<()> {
    let arm9 = elf::load(arm9_path, 0x02000000, 0x02000000)?;
    let arm7 = elf::load(arm7_path, 0x0238_0000, 0x0238_0000)?;
    let secure_boot = arm9.entry == arm9.ram.saturating_add(0x800);
    let arm9_offset = if secure_boot { 0x4000usize } else { 0x200usize };
    let mut rom = vec![0xffu8; arm9_offset + arm9.data.len()];
    let template = header_template.map(fs::read).transpose()?;
    if let Some(template) = template.as_deref() {
        if template.len() < 0x200 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "header template must be at least 512 bytes",
            ));
        }
        let copy_len = arm9_offset.min(template.len());
        rom[..copy_len].copy_from_slice(&template[..copy_len]);
        if copy_len < arm9_offset {
            rom[copy_len..arm9_offset].fill(0);
        }
    } else {
        rom[..arm9_offset].fill(0);
    }
    let template_title = template.as_deref().map(|bytes| {
        String::from_utf8_lossy(&bytes[..12])
            .trim_end_matches('\0')
            .to_string()
    });
    let template_game_code = template
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(&bytes[12..16]).to_string());
    let template_maker_code = template
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(&bytes[16..18]).to_string());
    let title = title
        .map(str::to_owned)
        .or(template_title)
        .unwrap_or_else(|| "NDSTOOL".to_string());
    let game_code = game_code
        .map(str::to_owned)
        .or(template_game_code)
        .unwrap_or_else(|| "####".to_string());
    let maker_code = maker_code
        .map(str::to_owned)
        .or(template_maker_code)
        .unwrap_or_else(|| "01".to_string());
    if title.len() > 12 || game_code.len() != 4 || maker_code.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "title must be at most 12 bytes, game code 4 bytes, maker code 2 bytes",
        ));
    }
    rom[0..title.len()].copy_from_slice(title.as_bytes());
    rom[12..16].copy_from_slice(game_code.as_bytes());
    rom[16..18].copy_from_slice(maker_code.as_bytes());
    let logo_bytes = if let Some(path) = logo_path {
        let logo = fs::read(path)?;
        if logo.len() != 156 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "raw logo must be exactly 156 bytes",
            ));
        }
        logo
    } else if let Some(template) = template.as_deref() {
        template[0xc0..0xc0 + 156].to_vec()
    } else {
        logo::NINTENDO_LOGO.to_vec()
    };
    rom[0xc0..0xc0 + 156].copy_from_slice(&logo_bytes);
    let logo_crc = header::crc16(&rom[0xc0..0x15c]);
    rom[0x15c..0x15e].copy_from_slice(&logo_crc.to_le_bytes());
    put32(&mut rom, 0x20, arm9_offset as u32);
    put32(&mut rom, 0x24, arm9.entry);
    put32(&mut rom, 0x28, arm9.ram);
    put32(&mut rom, 0x2c, arm9.data.len() as u32);
    put32(&mut rom, 0x60, 0x0058_6000);
    put32(&mut rom, 0x64, 0x0018_08F8);
    rom[0x6e..0x70].copy_from_slice(&0x051Eu16.to_le_bytes());
    put32(&mut rom, 0x84, if secure_boot { 0x4000 } else { 0x200 });
    if secure_boot {
        put32(&mut rom, 0x70, arm9.ram.saturating_add(0xA58));
        put32(&mut rom, 0x74, arm7.ram.saturating_add(0x158));
    }
    rom[arm9_offset..arm9_offset + arm9.data.len()].copy_from_slice(&arm9.data);
    if secure_boot && arm9.data.len() >= 0x4000 {
        let gamecode = u32::from_le_bytes([
            game_code.as_bytes()[0],
            game_code.as_bytes()[1],
            game_code.as_bytes()[2],
            game_code.as_bytes()[3],
        ]);
        let secure_crc =
            crypto::encrypted_secure_area_crc(gamecode, &rom[arm9_offset..arm9_offset + 0x4000])?;
        rom[0x6c..0x6e].copy_from_slice(&secure_crc.to_le_bytes());
    }

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
    let application_end = rom.len() as u32;
    put32(&mut rom, 0x80, application_end);
    rom.resize(size, 0xff);
    rom[0x14] = (size.trailing_zeros() as u8).saturating_sub(17);
    let crc = header::crc16(&rom[..0x15e]);
    rom[0x15e..0x160].copy_from_slice(&crc.to_le_bytes());
    fs::write(out, rom)
}
