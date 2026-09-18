//! Optional incremental linker. The private snapshot is the source of unchanged
//! bytes; metadata is a build-system-style change hint, not a content hash.
//! Changed ranges are journaled before updating the private working snapshot.
//! The user output remains independent; a receipt allows repeated range updates.
use crate::{
    header,
    model::{Entry, Header},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

mod layout;
mod transaction;

use layout::{put32, read32, tables};

const MAGIC: &[u8; 8] = b"NDSRS003";

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn dir(root: &Path) -> PathBuf {
    root.join(".ndstool-rs")
}

fn nonce() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn aligned(n: u64) -> u64 {
    (n + 511) & !511
}

fn u32_offset(n: u64) -> io::Result<u32> {
    u32::try_from(n).map_err(|_| invalid("ROM exceeds the 32-bit offset limit"))
}

#[derive(Clone, PartialEq)]
struct Stamp {
    size: u64,
    modified: u128,
}

fn stamp(meta: &fs::Metadata) -> io::Result<Stamp> {
    Ok(Stamp {
        size: meta.len(),
        modified: meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("input modification time predates Unix epoch"))?
            .as_nanos(),
    })
}

struct Input {
    path: PathBuf,
    stamp: Stamp,
}

struct State {
    snapshot: String,
    high_water: u64,
    entries: Vec<Entry>,
    stamps: BTreeMap<String, Stamp>,
}

struct Lock(PathBuf);
impl Lock {
    fn acquire(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir(root))?;
        let path = dir(root).join("build.lock");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("incremental cache is locked: {} ({e})", path.display()),
                )
            })?;
        Ok(Self(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn scan(root: &Path) -> io::Result<BTreeMap<String, Input>> {
    fn visit(p: &Path, prefix: &str, result: &mut BTreeMap<String, Input>) -> io::Result<()> {
        for item in fs::read_dir(p)? {
            let item = item?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| invalid("non-UTF8 input filename"))?;
            if name.starts_with('.') {
                continue;
            }
            if name.len() > 127 || name.contains(['/', '\\']) {
                return Err(invalid("invalid NDS filename"));
            }
            let ty = item.file_type()?;
            if ty.is_symlink() {
                return Err(invalid("incremental inputs must not be symlinks"));
            }
            let rel = format!("{prefix}/{name}");
            if ty.is_dir() {
                visit(&item.path(), &rel, result)?;
            } else if ty.is_file() {
                result.insert(
                    rel,
                    Input {
                        path: item.path(),
                        stamp: stamp(&item.metadata()?)?,
                    },
                );
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    visit(root, "", &mut result)?;
    Ok(result)
}

fn safe_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('\\')
        && Path::new(&path[1..])
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !path[1..].is_empty()
}

fn encode(state: &State) -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&(state.snapshot.len() as u32).to_le_bytes());
    b.extend_from_slice(state.snapshot.as_bytes());
    b.extend_from_slice(&state.high_water.to_le_bytes());
    b.extend_from_slice(&(state.entries.len() as u32).to_le_bytes());
    for e in &state.entries {
        let s = &state.stamps[&e.path];
        for v in [e.id, e.start, e.end, e.path.len() as u32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(e.path.as_bytes());
        b.extend_from_slice(&s.size.to_le_bytes());
        b.extend_from_slice(&s.modified.to_le_bytes());
    }
    b
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if n > self.0.len() {
            return Err(invalid("truncated incremental state"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn number(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn text(&mut self) -> io::Result<String> {
        let n = self.number()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| invalid("invalid state string"))
    }
}

fn load(root: &Path) -> io::Result<State> {
    let data = fs::read(dir(root).join("layout.bin"))?;
    let mut r = Reader(&data);
    if r.take(8)? != MAGIC {
        return Err(invalid(
            "old/invalid incremental state; extract again with --incremental",
        ));
    }
    let snapshot = r.text()?;
    if !snapshot.starts_with("snapshot-")
        || !snapshot.ends_with(".nds")
        || snapshot.contains(['/', '\\', ':'])
    {
        return Err(invalid("invalid snapshot name"));
    }
    let high_water = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
    let count = r.number()? as usize;
    if count > 65535 {
        return Err(invalid("too many state entries"));
    }
    let mut entries = Vec::new();
    let mut stamps = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for _ in 0..count {
        let id = r.number()?;
        let start = r.number()?;
        let end = r.number()?;
        let path = r.text()?;
        let size = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let modified = u128::from_le_bytes(r.take(16)?.try_into().unwrap());
        if !safe_path(&path)
            || start > end
            || size != u64::from(end - start)
            || !ids.insert(id)
            || stamps
                .insert(path.clone(), Stamp { size, modified })
                .is_some()
        {
            return Err(invalid("invalid or duplicate state entry"));
        }
        entries.push(Entry {
            id,
            start,
            end,
            path,
        });
    }
    if !r.0.is_empty() {
        return Err(invalid("trailing state data"));
    }
    Ok(State {
        snapshot,
        high_water,
        entries,
        stamps,
    })
}

/// Replace using a sibling backup on platforms where rename cannot overwrite.
/// The previous file is restored if publication fails.
fn publish(temp: &Path, target: &Path) -> io::Result<()> {
    let backup = target.with_extension(format!("backup-{}", nonce()));
    let had = target.exists();
    if had {
        fs::rename(target, &backup)?;
    }
    if let Err(e) = fs::rename(temp, target) {
        if had {
            let _ = fs::rename(&backup, target);
        }
        return Err(e);
    }
    if had {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

fn save(root: &Path, state: &State) -> io::Result<()> {
    let temporary = dir(root).join(format!("state-{}.tmp", nonce()));
    let mut file = File::create(&temporary)?;
    file.write_all(&encode(state))?;
    file.sync_all()?;
    drop(file);
    publish(&temporary, &dir(root).join("layout.bin"))
}

fn range(file: &mut File, start: u64, size: usize) -> io::Result<Vec<u8>> {
    let mut b = vec![0; size];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut b)?;
    Ok(b)
}

fn write_at(file: &mut File, start: u64, b: &[u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(start))?;
    file.write_all(b)
}

fn fill(file: &mut File, start: u64, end: u64) -> io::Result<()> {
    file.seek(SeekFrom::Start(start))?;
    let block = [0xff; 65536];
    let mut remaining = end.saturating_sub(start);
    while remaining > 0 {
        let n = remaining.min(block.len() as u64) as usize;
        file.write_all(&block[..n])?;
        remaining -= n as u64;
    }
    Ok(())
}

// Only known FAT payloads may be displaced. Unclaimed, non-FF bytes might
// contain a trailer or proprietary data and must not be overwritten as slack.
fn opaque_gap(
    source: &mut File,
    start: u64,
    end: u64,
    occupied: &[(u64, u64, Option<u32>)],
    high: u64,
) -> io::Result<bool> {
    let mut pos = start;
    let end = end.min(high);
    let first = occupied.partition_point(|&(_, b, _)| b <= pos);
    for &(a, b, _) in occupied[first..]
        .iter()
        .chain(std::iter::once(&(high, high, None)))
    {
        if pos >= end {
            break;
        }
        if b <= pos {
            continue;
        }
        let gap_end = a.min(end);
        while pos < gap_end {
            let n = (gap_end - pos).min(65536) as usize;
            if range(source, pos, n)?.iter().any(|&b| b != 0xff) {
                return Ok(true);
            }
            pos += n as u64;
        }
        pos = pos.max(b);
    }
    Ok(false)
}

fn validate(file: &mut File, h: &Header, entries: &[Entry]) -> io::Result<Vec<u8>> {
    let len = file.metadata()?.len();
    if h.fat_size % 8 != 0 || h.fat_size > 65536 * 8 || h.fnt_size > 16 * 1024 * 1024 {
        return Err(invalid("invalid filesystem table size"));
    }
    for (offset, size) in [
        (h.fnt_offset, h.fnt_size),
        (h.fat_offset, h.fat_size),
        (h.arm9_offset, h.arm9_size),
        (h.arm7_offset, h.arm7_size),
    ] {
        if u64::from(offset) + u64::from(size) > len {
            return Err(invalid("ROM section exceeds snapshot"));
        }
    }
    let fat = range(file, h.fat_offset as u64, h.fat_size as usize)?;
    for e in entries {
        let p = e.id as usize * 8;
        if p + 8 > fat.len()
            || e.start > e.end
            || e.end as u64 > len
            || read32(&fat, p) != e.start
            || read32(&fat, p + 4) != e.end
        {
            return Err(invalid("state and snapshot FAT do not match"));
        }
    }
    Ok(fat)
}

pub(crate) fn write_state(
    root: &Path,
    rom: &Path,
    h: &Header,
    entries: &[Entry],
) -> io::Result<()> {
    let _lock = Lock::acquire(root)?;
    if dir(root).join("pending").exists() {
        return Err(invalid(
            "unfinished incremental transaction; retry the previous build before extracting",
        ));
    }
    let inputs = scan(root)?;
    if inputs.len() != entries.len() {
        return Err(invalid(
            "use a dedicated -d directory for incremental extraction",
        ));
    }
    let mut file = File::open(rom)?;
    validate(&mut file, h, entries)?;
    // Preserve any opaque trailer (including DSi data): append only beyond the
    // last non-padding byte, declared application size and every FAT endpoint.
    let length = file.metadata()?.len();
    let mut high = length;
    while high > 0 {
        let start = high.saturating_sub(65536);
        let b = range(&mut file, start, (high - start) as usize)?;
        if let Some(i) = b.iter().rposition(|&b| b != 0xff) {
            high = start + i as u64 + 1;
            break;
        }
        high = start;
    }
    let header_bytes = range(&mut file, 0, 0x200)?;
    high = high.max(read32(&header_bytes, 0x80) as u64);
    for e in entries {
        high = high.max(e.end as u64);
    }
    if high > length {
        return Err(invalid("declared used size exceeds ROM"));
    }
    let stamps = entries
        .iter()
        .map(|e| {
            let input = inputs
                .get(&e.path)
                .ok_or_else(|| invalid("extracted path missing"))?;
            if input.stamp.size != (e.end - e.start) as u64 {
                return Err(invalid("extracted size mismatch"));
            }
            Ok((e.path.clone(), input.stamp.clone()))
        })
        .collect::<io::Result<_>>()?;
    let snapshot = format!("snapshot-{}.nds", nonce());
    fs::copy(rom, dir(root).join(&snapshot))?;
    let old = load(root).ok();
    save(
        root,
        &State {
            snapshot,
            high_water: high,
            entries: entries.to_vec(),
            stamps,
        },
    )?;
    if let Some(old) = old {
        let _ = fs::remove_file(dir(root).join(old.snapshot));
    }
    Ok(())
}

pub(crate) fn link(root: &Path, out: &Path) -> io::Result<()> {
    let _lock = Lock::acquire(root)?;
    transaction::recover(root, out)?;
    let state = load(root)?;
    let root_abs = fs::canonicalize(root)?;
    let output_parent = fs::canonicalize(
        out.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    if output_parent.starts_with(&root_abs) {
        return Err(invalid(
            "output ROM must be outside the input data directory",
        ));
    }
    let base = dir(root).join(&state.snapshot);
    let h = header::read(&base)?;
    let mut source = File::open(&base)?;
    let length = source.metadata()?.len();
    if state.high_water > length {
        return Err(invalid("invalid state high water"));
    }
    let mut fat = validate(&mut source, &h, &state.entries)?;
    let inputs = scan(root)?;
    let old: BTreeMap<_, _> = state
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.clone()))
        .collect();
    let mut changed = BTreeMap::new();
    for (path, input) in &inputs {
        // Size + nanosecond mtime is a fast metadata hint, as in build systems.
        // Preserving both while changing bytes requires regenerating the cache.
        if state.stamps.get(path) != Some(&input.stamp) {
            let bytes = fs::read(&input.path)?;
            if stamp(&fs::metadata(&input.path)?)? != input.stamp
                || bytes.len() as u64 != input.stamp.size
            {
                return Err(invalid("input changed while linking; retry"));
            }
            changed.insert(path.clone(), bytes);
        }
    }
    let structural = old.keys().ne(inputs.keys());
    let deleted: Vec<_> = old
        .values()
        .filter(|e| !inputs.contains_key(&e.path))
        .collect();
    let mut result = old
        .iter()
        .filter(|(p, _)| inputs.contains_key(*p))
        .map(|(p, e)| (p.clone(), e.clone()))
        .collect::<BTreeMap<_, _>>();

    // Every FAT payload is an obstacle, including overlays and retired IDs.
    // Preserve unchanged payload offsets. Growth that cannot fit relocates
    // only the changed payload, never a cascade of unrelated following files.
    let mut occupied = Vec::new();
    for (id, b) in fat.chunks_exact(8).enumerate() {
        let start = read32(b, 0) as u64;
        let end = read32(b, 4) as u64;
        if start > end || end > length {
            return Err(invalid("invalid FAT range"));
        }
        if end > start {
            occupied.push((start, end, Some(id as u32)));
        }
    }
    for (start, size) in [
        (0, 0x200),
        (h.arm9_offset, h.arm9_size),
        (h.arm7_offset, h.arm7_size),
        (h.fnt_offset, h.fnt_size),
        (h.fat_offset, h.fat_size),
        (h.arm9_overlay_offset, h.arm9_overlay_size),
        (h.arm7_overlay_offset, h.arm7_overlay_size),
        (
            h.banner_offset,
            if h.unit_code & 2 != 0 {
                h.banner_size
            } else {
                0x840
            },
        ),
        (h.dsi9_offset, h.dsi9_size),
        (h.dsi7_offset, h.dsi7_size),
    ] {
        if start != 0 || size == 0x200 {
            if size != 0 {
                occupied.push((start as u64, start as u64 + size as u64, None));
            }
        }
    }
    occupied.sort_by_key(|r| (r.0, r.1));
    for pair in occupied.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(invalid(
                "overlapping ROM regions cannot be incrementally linked",
            ));
        }
    }
    let mut high = state.high_water;
    let mut physical: Vec<_> = result.values().cloned().collect();
    physical.sort_by_key(|e| (e.start, e.id));
    for e in physical {
        let size = inputs[&e.path].stamp.size;
        let mut start = e.start as u64;
        let mut end = start
            .checked_add(size)
            .ok_or_else(|| invalid("size overflow"))?;
        let first = occupied.partition_point(|&(_, b, _)| b <= start);
        let blocked = occupied[first..]
            .iter()
            .take_while(|&&(a, _, _)| a < end)
            .any(|&(a, b, id)| id != Some(e.id) && start < b && end > a);
        if size > u64::from(e.end - e.start)
            && (e.start == e.end
                || blocked
                || opaque_gap(&mut source, e.end as u64, end, &occupied, state.high_water)?)
        {
            start = aligned(high);
            end = start + size;
        }
        u32_offset(end)?;
        let target = result.get_mut(&e.path).unwrap();
        target.start = start as u32;
        target.end = end as u32;
        high = high.max(end);
    }
    for (path, input) in &inputs {
        if !old.contains_key(path) {
            let start = aligned(high);
            high = start + input.stamp.size;
            result.insert(
                path.clone(),
                Entry {
                    path: path.clone(),
                    id: u32::MAX,
                    start: u32_offset(start)?,
                    end: u32_offset(high)?,
                },
            );
        }
    }
    let mut fnt_write = None;
    let mut fat_offset = h.fat_offset as u64;
    let mut hdr = range(&mut source, 0, 0x200)?;
    let mut remapped = 0;
    if structural {
        let (fnt, new_fat, remaps) = tables(&mut result, &old, &fat)?;
        remapped = remaps;
        fat = new_fat;
        // Table relocation is independent of payload order; append the small
        // tables rather than shifting ARM binaries or opaque DSi sections.
        let offset = aligned(high);
        put32(&mut hdr, 0x40, u32_offset(offset)?);
        put32(&mut hdr, 0x44, fnt.len() as u32);
        high = offset + fnt.len() as u64;
        fat_offset = (high + 3) & !3;
        high = fat_offset + fat.len() as u64;
        put32(&mut hdr, 0x48, u32_offset(fat_offset)?);
        put32(&mut hdr, 0x4c, fat.len() as u32);
        fnt_write = Some((offset, fnt));
    }
    for e in result.values() {
        let p = e.id as usize * 8;
        if p + 8 > fat.len() {
            return Err(invalid("file ID outside FAT"));
        }
        put32(&mut fat, p, e.start);
        put32(&mut fat, p + 4, e.end);
    }
    for (path, e) in &result {
        if let Some(before) = old.get(path) {
            if before.id != e.id {
                put32(&mut fat, before.id as usize * 8, before.start);
                put32(&mut fat, before.id as usize * 8 + 4, before.start);
            }
        }
    }
    for e in &deleted {
        // Keep historical FAT slots reserved; never reuse IDs.
        put32(&mut fat, e.id as usize * 8, e.start);
        put32(&mut fat, e.id as usize * 8 + 4, e.start);
    }
    let mut used = read32(&hdr, 0x80) as u64;
    for e in result.values() {
        used = used.max(e.end as u64);
    }
    if structural {
        used = used.max(high);
    }
    put32(&mut hdr, 0x80, u32_offset(used)?);
    let new_length = if high > length {
        high.checked_next_power_of_two()
            .ok_or_else(|| invalid("capacity overflow"))?
    } else {
        length
    };
    u32_offset(new_length)?;
    if new_length != length {
        hdr[0x14] = (new_length.trailing_zeros() as u8).saturating_sub(17);
    }
    if h.unit_code & 2 != 0 {
        // NTR payload editing does not regenerate DSi digest/signature tables.
        eprintln!("Note: incremental linking preserves DSi sections but does not regenerate DSi authentication data.");
    }
    let crc = header::crc16(&hdr[..0x15e]);
    hdr[0x15e..0x160].copy_from_slice(&crc.to_le_bytes());
    let mut patches = Vec::new();
    let mut moved = 0u64;
    for (path, e) in &result {
        if let Some(bytes) = changed.remove(path) {
            patches.push((e.start as u64, bytes));
        } else if let Some(before) = old.get(path) {
            if before.start != e.start {
                let size = (before.end - before.start) as u64;
                // Read displaced payloads before any in-place write. This
                // makes overlapping moves independent of write direction.
                patches.push((
                    e.start as u64,
                    range(&mut source, before.start as u64, size as usize)?,
                ));
                moved += size;
            }
        }
    }
    if let Some((offset, b)) = &fnt_write {
        patches.push((*offset, b.clone()));
    }
    if structural || range(&mut source, fat_offset, fat.len())? != fat {
        patches.push((fat_offset, fat));
    }
    if range(&mut source, 0, hdr.len())? != hdr {
        patches.push((0, hdr));
    }
    let changed_count = inputs
        .iter()
        .filter(|(p, i)| state.stamps.get(*p) != Some(&i.stamp))
        .count();
    let next = State {
        snapshot: state.snapshot.clone(),
        high_water: high,
        entries: result.into_values().collect(),
        stamps: inputs.into_iter().map(|(p, i)| (p, i.stamp)).collect(),
    };
    drop(source);
    transaction::commit(root, &base, out, &next, patches, new_length)?;
    println!(
        "Incremental: {} changed/new, {} deleted, {} bytes moved, {} IDs reassigned",
        changed_count,
        deleted.len(),
        moved,
        remapped
    );
    Ok(())
}
