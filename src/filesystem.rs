use crate::model::{Entry, Header};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
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
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeMap<String, Node>,
}

pub(crate) struct FsImage {
    pub(crate) data: Vec<u8>,
    pub(crate) fnt: Vec<u8>,
    pub(crate) fat: Vec<u8>,
}

fn insert(node: &mut Node, parts: &[&str], data: Vec<u8>) {
    if parts.len() == 1 {
        node.files.insert(parts[0].to_string(), data);
        return;
    }
    insert(
        node.dirs.entry(parts[0].to_string()).or_default(),
        &parts[1..],
        data,
    );
}

pub(crate) fn build_image(root: &Path, base_offset: u32) -> io::Result<FsImage> {
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
                insert(dir, &[&name], std::fs::read(path)?);
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
        ordered: &mut Vec<Vec<u8>>,
    ) {
        let idx = (id & 0x0fff) as usize;
        if infos.len() <= idx {
            infos.resize(idx + 1, (0, 0, 0));
        }
        let start = names.len() as u32;
        let first = *file_id;
        for (name, data) in &node.files {
            names.push(name.len() as u8);
            names.extend_from_slice(name.as_bytes());
            ordered.push(data.clone());
            *file_id += 1;
        }
        let mut child_ids = Vec::new();
        for name in node.dirs.keys() {
            let child = 0xf000 | *next_dir;
            *next_dir += 1;
            child_ids.push(child);
            names.push(0x80 | name.len() as u8);
            names.extend_from_slice(name.as_bytes());
            names.extend_from_slice(&child.to_le_bytes());
        }
        names.push(0);
        infos[idx] = (start, first, parent);
        for (child_id, child_node) in child_ids.into_iter().zip(node.dirs.values()) {
            encode(
                child_node, child_id, id, file_id, next_dir, names, infos, ordered,
            );
        }
    }
    let mut file_id = 0u16;
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
    let table_size = infos.len() * 8;
    let mut fnt = vec![0u8; table_size + names.len()];
    for (i, (start, first, parent)) in infos.iter().enumerate() {
        fnt[i * 8..i * 8 + 4].copy_from_slice(&((table_size as u32) + *start).to_le_bytes());
        fnt[i * 8 + 4..i * 8 + 6].copy_from_slice(&first.to_le_bytes());
        fnt[i * 8 + 6..i * 8 + 8].copy_from_slice(&parent.to_le_bytes());
    }
    fnt[table_size..].copy_from_slice(&names);
    let mut data = Vec::new();
    let mut fat = Vec::new();
    let mut cursor = base_offset;
    for bytes in ordered {
        let aligned = (cursor + 0x1ff) & !0x1ff;
        data.resize(data.len() + (aligned - cursor) as usize, 0xff);
        cursor = aligned;
        let start = cursor;
        data.extend_from_slice(&bytes);
        cursor += bytes.len() as u32;
        fat.extend_from_slice(&start.to_le_bytes());
        fat.extend_from_slice(&cursor.to_le_bytes());
    }
    Ok(FsImage { data, fnt, fat })
}
