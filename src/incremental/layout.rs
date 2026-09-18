use crate::model::Entry;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub(super) fn read32(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}

pub(super) fn put32(b: &mut [u8], p: usize, n: u32) {
    b[p..p + 4].copy_from_slice(&n.to_le_bytes());
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn parent(path: &str) -> &str {
    let p = path.rfind('/').unwrap();
    if p == 0 {
        "/"
    } else {
        &path[..p]
    }
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

/// FNT file IDs are implicit: a directory has one first ID and a consecutive
/// run. Preserve that run whenever possible. If insertion/deletion would
/// break it, allocate a fresh run for this directory only. Old FAT slots stay
/// reserved so an unrelated file never silently inherits a historical ID.
/// Games using hard-coded IDs need their references updated for such edits.
pub(super) fn tables(
    entries: &mut BTreeMap<String, Entry>,
    old: &BTreeMap<String, Entry>,
    previous_fat: &[u8],
) -> io::Result<(Vec<u8>, Vec<u8>, usize)> {
    let mut directories = BTreeSet::from(["/".to_owned()]);
    let mut files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in entries.keys() {
        let mut p = parent(path);
        files.entry(p.to_string()).or_default().push(path.clone());
        while p != "/" {
            directories.insert(p.to_owned());
            p = parent(p);
        }
    }
    if directories.len() > 4096 {
        return Err(invalid("too many NDS directories"));
    }
    let dirs: Vec<_> = directories.into_iter().collect();
    let dir_ids: BTreeMap<_, _> = dirs
        .iter()
        .enumerate()
        .map(|(i, p)| (p.clone(), i))
        .collect();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in dirs.iter().skip(1) {
        children
            .entry(parent(p).to_owned())
            .or_default()
            .push(p.clone());
    }
    let mut next_id = previous_fat.len() / 8;
    let mut first_ids = BTreeMap::new();
    let mut remapped = 0;
    for (directory, paths) in &mut files {
        // Existing order is ID order; append additions within the directory.
        paths.sort_by_key(|p| (old.get(p).map(|e| e.id).unwrap_or(u32::MAX), p.clone()));
        let first = old.get(&paths[0]).map(|e| e.id as usize);
        let reusable = first.is_some_and(|first| {
            paths
                .iter()
                .enumerate()
                .all(|(i, p)| old.get(p).is_some_and(|e| e.id as usize == first + i))
        });
        let first = if reusable {
            first.unwrap()
        } else {
            let n = next_id;
            next_id += paths.len();
            n
        };
        if first + paths.len() > 65535 {
            return Err(invalid("incremental file IDs exhausted; use a full build"));
        }
        first_ids.insert(directory.clone(), first as u16);
        for (i, p) in paths.iter().enumerate() {
            let id = (first + i) as u32;
            if old.get(p).is_some_and(|e| e.id != id) {
                remapped += 1;
            }
            entries.get_mut(p).unwrap().id = id;
        }
    }
    let mut fat = previous_fat.to_vec();
    fat.resize(next_id * 8, 0);
    let mut fnt = vec![0; dirs.len() * 8];
    for (i, p) in dirs.iter().enumerate() {
        let offset = fnt.len() as u32;
        put32(&mut fnt, i * 8, offset);
        let first = *first_ids.get(p).unwrap_or(&0);
        fnt[i * 8 + 4..i * 8 + 6].copy_from_slice(&first.to_le_bytes());
        let parent_id = if i == 0 {
            dirs.len() as u16
        } else {
            0xf000 | dir_ids[parent(p)] as u16
        };
        fnt[i * 8 + 6..i * 8 + 8].copy_from_slice(&parent_id.to_le_bytes());
        if let Some(paths) = files.get(p) {
            for path in paths {
                let n = name(path).as_bytes();
                fnt.push(n.len() as u8);
                fnt.extend_from_slice(n);
            }
        }
        if let Some(paths) = children.get(p) {
            for path in paths {
                let n = name(path).as_bytes();
                fnt.push(0x80 | n.len() as u8);
                fnt.extend_from_slice(n);
                fnt.extend_from_slice(&(0xf000 | dir_ids[path] as u16).to_le_bytes());
            }
        }
        fnt.push(0);
    }
    Ok((fnt, fat, remapped))
}
