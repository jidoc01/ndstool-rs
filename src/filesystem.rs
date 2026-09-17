use crate::model::{Entry, Header};
use std::{
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
