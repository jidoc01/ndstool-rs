use std::{
    fs,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

pub(crate) fn extract_range(rom: &Path, out: &Path, offset: u32, size: u32) -> io::Result<()> {
    let mut f = File::open(rom)?;
    f.seek(SeekFrom::Start(offset as u64))?;
    let mut buf = vec![0; size as usize];
    f.read_exact(&mut buf)?;
    fs::write(out, buf)
}
