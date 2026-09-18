//! Stage a full build beside its destination, publishing only on success.
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) struct Output {
    pub(crate) path: PathBuf,
}
impl Output {
    pub(crate) fn new(destination: &Path) -> io::Result<(Self, File)> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        match fs::symlink_metadata(destination) {
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(io::Error::other("output must be a regular file"))
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        let parent = destination.parent().unwrap_or(Path::new("."));
        for _ in 0..100 {
            let path = parent.join(format!(
                ".ndstool-build-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self { path }, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("cannot reserve temporary output"))
    }
    pub(crate) fn publish(self, destination: &Path) -> io::Result<()> {
        // All writers must be closed first, especially on Windows. This is
        // failure-safe publication, not a power-loss durability guarantee.
        fs::rename(&self.path, destination)
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
