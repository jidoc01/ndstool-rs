use crate::model::{Entry, Header};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
fn le16(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}
fn le32(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
}
pub(crate) fn read_entries(path: &Path, h: &Header) -> io::Result<Vec<Entry>> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(h.fnt_offset as u64))?;
    let mut fnt = vec![0; h.fnt_size as usize];
    f.read_exact(&mut fnt)?;
    f.seek(SeekFrom::Start(h.fat_offset as u64))?;
    let mut fat = vec![0; h.fat_size as usize];
    f.read_exact(&mut fat)?;
    fn dir(fnt: &[u8], fat: &[u8], dir_id: u16, prefix: String, out: &mut Vec<Entry>) {
        let p = (dir_id as usize & 0xfff) * 8;
        if p + 8 > fnt.len() {
            return;
        }
        let mut pos = le32(fnt, p) as usize;
        let mut id = le16(fnt, p + 4) as u32;
        while pos < fnt.len() {
            let n = fnt[pos];
            pos += 1;
            if n == 0 {
                break;
            }
            let len = (n & 0x7f) as usize;
            if pos + len > fnt.len() {
                break;
            }
            let name = String::from_utf8_lossy(&fnt[pos..pos + len]).into_owned();
            pos += len;
            if n & 0x80 != 0 {
                if pos + 2 > fnt.len() {
                    break;
                }
                let child = le16(fnt, pos);
                pos += 2;
                dir(fnt, fat, child, format!("{}{}/", prefix, name), out)
            } else {
                let fp = id as usize * 8;
                if fp + 8 <= fat.len() {
                    out.push(Entry {
                        id,
                        path: format!("{}{}", prefix, name),
                        start: le32(fat, fp),
                        end: le32(fat, fp + 4),
                    })
                }
                id += 1
            }
        }
    }
    let mut out = Vec::new();
    dir(&fnt, &fat, 0xf000, "/".into(), &mut out);
    Ok(out)
}

#[derive(Default)]
struct Node {
    files: BTreeMap<String, std::path::PathBuf>,
    dirs: BTreeMap<String, Node>,
}

pub(crate) struct FsImage {
    pub(crate) data: Vec<u8>,
    pub(crate) fnt: Vec<u8>,
    pub(crate) fat: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum LayoutMode {
    #[default]
    Stable,
    Random,
}

fn insert(node: &mut Node, parts: &[&str], path: std::path::PathBuf) {
    if parts.len() == 1 {
        node.files.insert(parts[0].to_string(), path);
        return;
    }
    insert(
        node.dirs.entry(parts[0].to_string()).or_default(),
        &parts[1..],
        path,
    );
}

pub(crate) fn build_image(
    root: &Path,
    base_offset: u32,
    first_file_id: u16,
    layout: LayoutMode,
    jobs: usize,
) -> io::Result<FsImage> {
    fn scan(root: &Path, dir: &mut Node, prefix: &str) -> io::Result<()> {
        for item in std::fs::read_dir(root)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = item.path();
            if item.file_type()?.is_dir() {
                scan(&path, dir.dirs.entry(name).or_default(), prefix)?;
            } else if item.file_type()?.is_file() {
                // Keep paths during the directory scan. The actual reads are
                // performed after FNT IDs are assigned, which lets us read
                // independent files concurrently without changing their IDs.
                insert(dir, &[&name], path);
            }
        }
        let _ = prefix;
        Ok(())
    }
    let mut root_node = Node::default();
    scan(root, &mut root_node, "")?;
    let mut names = Vec::new();
    let mut infos: Vec<(u32, u16, u16)> = Vec::new();
    let mut ordered = Vec::new();
    fn encode(
        node: &Node,
        id: u16,
        parent: u16,
        file_id: &mut u16,
        next_dir: &mut u16,
        names: &mut Vec<u8>,
        infos: &mut Vec<(u32, u16, u16)>,
        ordered: &mut Vec<(u16, std::path::PathBuf)>,
    ) {
        let idx = (id & 0x0fff) as usize;
        if infos.len() <= idx {
            infos.resize(idx + 1, (0, 0, 0));
        }
        let start = names.len() as u32;
        let first = *file_id;
        let mut files: Vec<_> = node.files.iter().collect();
        // ndstool assigns IDs using byte-wise name ordering. Do not fold case
        // here: changing this order changes every subsequent file ID.
        files.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (name, path) in files {
            names.push(name.len() as u8);
            names.extend_from_slice(name.as_bytes());
            // FNT traversal owns the file ID. It must remain stable even when
            // the physical payload order is randomized below.
            ordered.push((*file_id, path.clone()));
            *file_id += 1;
        }
        let mut child_ids = Vec::new();
        let mut dirs: Vec<_> = node.dirs.iter().collect();
        // Directory IDs affect the FNT tree, so use the same case-sensitive
        // ordering for directories as for files.
        dirs.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (name, _) in &dirs {
            let child = 0xf000 | *next_dir;
            *next_dir += 1;
            child_ids.push(child);
            names.push(0x80 | name.len() as u8);
            names.extend_from_slice(name.as_bytes());
            names.extend_from_slice(&child.to_le_bytes());
        }
        names.push(0);
        infos[idx] = (start, first, parent);
        for (child_id, (_, child_node)) in child_ids.into_iter().zip(dirs) {
            encode(
                child_node, child_id, id, file_id, next_dir, names, infos, ordered,
            );
        }
    }
    let mut file_id = first_file_id;
    let mut next_dir = 1u16;
    encode(
        &root_node,
        0xf000,
        0,
        &mut file_id,
        &mut next_dir,
        &mut names,
        &mut infos,
        &mut ordered,
    );
    let ordered = read_inputs(ordered, jobs)?;
    let table_size = infos.len() * 8;
    let mut fnt = vec![0u8; table_size + names.len()];
    for (i, (start, first, parent)) in infos.iter().enumerate() {
        fnt[i * 8..i * 8 + 4].copy_from_slice(&((table_size as u32) + *start).to_le_bytes());
        fnt[i * 8 + 4..i * 8 + 6].copy_from_slice(&first.to_le_bytes());
        fnt[i * 8 + 6..i * 8 + 8].copy_from_slice(&parent.to_le_bytes());
    }
    fnt[table_size..].copy_from_slice(&names);
    // The root entry stores the total directory count in its parent field;
    // child entries store their actual parent directory ID there.
    fnt[6..8].copy_from_slice(&(infos.len() as u16).to_le_bytes());
    let mut placement = (0..ordered.len()).collect::<Vec<_>>();
    if matches!(layout, LayoutMode::Random) {
        // Only payload order is randomized; FNT names and file IDs are kept
        // stable so the generated filesystem remains semantically identical.
        // This option is intentionally non-reproducible. Leave it disabled
        // when byte-for-byte rebuilds or deterministic builds are required.
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        let mut state = seed | 1;
        for i in (1..placement.len()).rev() {
            // xorshift64* is sufficient for layout randomization and avoids
            // adding a dependency for a non-cryptographic feature.
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let random = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
            placement.swap(i, (random as usize) % (i + 1));
        }
    }
    let mut placements = Vec::with_capacity(ordered.len());
    let mut fat = vec![0u8; ordered.len() * 8];
    let mut cursor = base_offset;
    for index in placement {
        let (file_id, bytes) = &ordered[index];
        let aligned = (cursor + 0x1ff) & !0x1ff;
        cursor = aligned;
        let start = cursor;
        cursor += bytes.len() as u32;
        let local_id = usize::from(*file_id - first_file_id);
        fat[local_id * 8..local_id * 8 + 4].copy_from_slice(&start.to_le_bytes());
        fat[local_id * 8 + 4..local_id * 8 + 8].copy_from_slice(&cursor.to_le_bytes());
        placements.push(((start - base_offset) as usize, bytes.clone()));
    }
    let mut data = vec![0xffu8; (cursor - base_offset) as usize];
    copy_payloads(&mut data, placements, jobs)?;
    Ok(FsImage { data, fnt, fat })
}

fn copy_payloads(
    data: &mut [u8],
    placements: Vec<(usize, Vec<u8>)>,
    jobs: usize,
) -> io::Result<()> {
    if jobs <= 1 || placements.len() <= 1 {
        for (offset, bytes) in placements {
            data[offset..offset + bytes.len()].copy_from_slice(&bytes);
        }
        return Ok(());
    }

    let worker_count = jobs.min(placements.len());
    let chunk_size = placements.len().div_ceil(worker_count);
    // Raw pointers are deliberately represented as an address while moving
    // the worker closure between threads; the address remains valid because
    // `data` is borrowed for the entire scoped-thread lifetime.
    let data_start = data.as_mut_ptr() as usize;
    let data_len = data.len();
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for chunk in placements.chunks(chunk_size) {
            handles.push(scope.spawn(move || -> io::Result<()> {
                for (offset, bytes) in chunk {
                    if offset.saturating_add(bytes.len()) > data_len {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "payload placement exceeds filesystem image",
                        ));
                    }
                    // Every range was assigned by the sequential layout pass;
                    // alignment makes the ranges disjoint. Each worker writes
                    // only its own range, so the raw pointer does not alias
                    // another worker's mutable slice.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            (data_start as *mut u8).add(*offset),
                            bytes.len(),
                        );
                    }
                }
                Ok(())
            }));
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| io::Error::other("parallel payload worker panicked"))??;
        }
        Ok(())
    })
}

fn read_inputs(
    inputs: Vec<(u16, std::path::PathBuf)>,
    jobs: usize,
) -> io::Result<Vec<(u16, Vec<u8>)>> {
    if jobs <= 1 || inputs.len() <= 1 {
        return inputs
            .into_iter()
            .map(|(id, path)| {
                fs::read(&path).map(|data| (id, data)).map_err(|error| {
                    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
                })
            })
            .collect();
    }

    let worker_count = jobs.min(inputs.len());
    let chunk_size = inputs.len().div_ceil(worker_count);
    let results = thread::scope(|scope| -> io::Result<Vec<(u16, Vec<u8>)>> {
        let mut handles = Vec::new();
        for chunk in inputs.chunks(chunk_size) {
            handles.push(scope.spawn(move || {
                chunk
                    .iter()
                    .map(|(id, path)| {
                        fs::read(path).map(|data| (*id, data)).map_err(|error| {
                            io::Error::new(error.kind(), format!("{}: {error}", path.display()))
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()
            }));
        }
        let mut output = Vec::with_capacity(inputs.len());
        for handle in handles {
            output.extend(
                handle
                    .join()
                    .map_err(|_| io::Error::other("parallel input worker panicked"))??,
            );
        }
        Ok(output)
    })?;
    // Worker completion order is not part of the file format. Restore the
    // FNT order before applying stable or randomized physical placement.
    let mut results = results;
    results.sort_by_key(|(id, _)| *id);
    Ok(results)
}

pub(crate) fn empty_image(first_file_id: u16) -> FsImage {
    let mut fnt = vec![0u8; 8 + 1];
    fnt[0..4].copy_from_slice(&8u32.to_le_bytes());
    fnt[4..6].copy_from_slice(&first_file_id.to_le_bytes());
    fnt[6..8].copy_from_slice(&0u16.to_le_bytes());
    fnt[8] = 0;
    FsImage {
        data: Vec::new(),
        fnt,
        fat: Vec::new(),
    }
}
