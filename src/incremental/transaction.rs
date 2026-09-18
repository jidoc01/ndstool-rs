//! Range-level undo journal. No full snapshot copy on the repeated-output path.
//! A receipt is a size/mtime hint, not a defense against deliberate tampering.
//! ROM readers must be closed during linking: publication is now in-place.
use super::*;

fn absolute_output(out: &Path) -> io::Result<PathBuf> {
    if fs::symlink_metadata(out).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err(invalid(
            "incremental output must be a regular file, not a directory or symlink",
        ));
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(fs::canonicalize(parent)?.join(
        out.file_name()
            .ok_or_else(|| invalid("missing output filename"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_range_update_rolls_back_both_files_and_length() {
        let root = std::env::temp_dir().join(format!("ndstool-journal-{}", nonce()));
        let cache = dir(&root);
        let pending = cache.join("pending");
        fs::create_dir_all(&pending).unwrap();
        let output = root.join("result.nds");
        let base = cache.join("snapshot-test.nds");
        let original = vec![0x55; 1024];
        fs::write(&base, vec![0x99; 2048]).unwrap();
        fs::write(&output, vec![0x99; 2048]).unwrap();
        fs::write(cache.join("layout.bin"), b"old state").unwrap();
        fs::write(pending.join("before"), b"old state").unwrap();
        fs::write(pending.join("next"), b"new state").unwrap();
        fs::write(pending.join("snapshot"), b"snapshot-test.nds").unwrap();
        fs::write(
            pending.join("output"),
            absolute_output(&output).unwrap().to_str().unwrap(),
        )
        .unwrap();
        let mut undo = Vec::new();
        for n in [1024u64, 1, 0, 1024] {
            undo.extend_from_slice(&n.to_le_bytes());
        }
        undo.extend_from_slice(&original);
        fs::write(pending.join("undo"), undo).unwrap();
        // A different target cannot cause writes to an unrelated old output.
        assert!(recover(&root, &root.join("different.nds")).is_err());
        recover(&root, &output).unwrap();
        assert_eq!(fs::read(&base).unwrap(), original);
        assert_eq!(fs::read(&output).unwrap(), original);
        assert_eq!(fs::read(cache.join("layout.bin")).unwrap(), b"old state");
        assert!(!pending.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_journal_is_discarded_without_undoing_the_rom() {
        let root = std::env::temp_dir().join(format!("ndstool-journal-{}", nonce()));
        let pending = dir(&root).join("pending");
        fs::create_dir_all(&pending).unwrap();
        fs::write(pending.join("next"), b"new state").unwrap();
        fs::write(dir(&root).join("layout.bin"), b"new state").unwrap();
        let output = root.join("result.nds");
        fs::write(&output, [42; 512]).unwrap();
        recover(&root, &output).unwrap();
        assert_eq!(fs::read(output).unwrap(), [42; 512]);
        assert!(!pending.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failures_at_snapshot_output_and_state_commit_boundaries_are_recoverable() {
        for phase in 1..=3 {
            let root = std::env::temp_dir().join(format!("ndstool-fault-{}", nonce()));
            fs::create_dir_all(dir(&root)).unwrap();
            let base = dir(&root).join("snapshot-test.nds");
            let output = root.join("output.nds");
            let original = vec![5; 512];
            fs::write(&base, &original).unwrap();
            fs::write(&output, &original).unwrap();
            let before = State {
                snapshot: "snapshot-test.nds".into(),
                high_water: 512,
                entries: vec![],
                stamps: BTreeMap::new(),
            };
            save(&root, &before).unwrap();
            fs::write(
                dir(&root).join("output.receipt"),
                receipt(&output, &encode(&before)).unwrap(),
            )
            .unwrap();
            let next = State {
                high_water: 768,
                ..before
            };
            let patches = vec![(32, vec![8; 40]), (600, vec![7; 10])];
            let result =
                commit_with_checkpoint(&root, &base, &output, &next, patches, 1024, |at| {
                    if at == phase {
                        Err(io::Error::other("injected I/O failure"))
                    } else {
                        Ok(())
                    }
                });
            assert!(result.is_err());
            if phase < 3 {
                assert_eq!(fs::read(&base).unwrap(), original);
                assert_eq!(fs::read(&output).unwrap(), original);
                assert_eq!(load(&root).unwrap().high_water, 512);
            } else {
                let mut expected = original;
                expected.resize(1024, 0xff);
                expected[32..72].fill(8);
                expected[600..610].fill(7);
                assert_eq!(fs::read(&base).unwrap(), expected);
                assert_eq!(fs::read(&output).unwrap(), expected);
                assert_eq!(load(&root).unwrap().high_water, 768);
            }
            assert!(!dir(&root).join("pending").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }
}
fn durable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}
fn receipt(out: &Path, state: &[u8]) -> io::Result<Vec<u8>> {
    let path = absolute_output(out)?;
    let name = path
        .to_str()
        .ok_or_else(|| invalid("non-UTF8 output path"))?;
    let s = stamp(&fs::metadata(&path)?)?;
    let mut b = (name.len() as u32).to_le_bytes().to_vec();
    b.extend_from_slice(name.as_bytes());
    b.extend_from_slice(&s.size.to_le_bytes());
    b.extend_from_slice(&s.modified.to_le_bytes());
    b.extend_from_slice(state);
    Ok(b)
}
fn restore(undo: &Path, target: &Path) -> io::Result<()> {
    let mut source = File::open(undo)?;
    let mut word = [0; 8];
    source.read_exact(&mut word)?;
    let length = u64::from_le_bytes(word);
    source.read_exact(&mut word)?;
    let count = u64::from_le_bytes(word);
    if length > u32::MAX as u64 || count > 65540 {
        return Err(invalid("invalid undo journal"));
    }
    let mut dest = OpenOptions::new().read(true).write(true).open(target)?;
    for _ in 0..count {
        source.read_exact(&mut word)?;
        let start = u64::from_le_bytes(word);
        source.read_exact(&mut word)?;
        let size = u64::from_le_bytes(word);
        if start > length || size > length - start {
            return Err(invalid("undo range exceeds original ROM"));
        }
        dest.seek(SeekFrom::Start(start))?;
        if io::copy(&mut (&mut source).take(size), &mut dest)? != size {
            return Err(invalid("truncated undo journal"));
        }
    }
    dest.set_len(length)?;
    dest.sync_all()
}

fn retire_journal(root: &Path) -> io::Result<()> {
    // Never leave a half-deleted directory named `pending` after interruption.
    // Once renamed, it is unreferenced garbage, safe to remove independently.
    let retired = dir(root).join(format!("retired-{}", nonce()));
    fs::rename(dir(root).join("pending"), &retired)?;
    let _ = fs::remove_dir_all(retired);
    Ok(())
}

/// Called only under build.lock. After a killed process the user must first
/// remove its stale lock after checking that no other linker is running.
pub(super) fn recover(root: &Path, out: &Path) -> io::Result<()> {
    let pending = dir(root).join("pending");
    if !pending.exists() {
        return Ok(());
    }
    let next = fs::read(pending.join("next"))?;
    let committed = fs::read(dir(root).join("layout.bin")).is_ok_and(|b| b == next);
    if !committed {
        let snapshot = fs::read_to_string(pending.join("snapshot"))?;
        if !snapshot.starts_with("snapshot-")
            || !snapshot.ends_with(".nds")
            || snapshot.contains(['/', '\\', ':'])
        {
            return Err(invalid("invalid journal snapshot"));
        }
        if pending.join("output").exists() {
            let expected = fs::read_to_string(pending.join("output"))?;
            if absolute_output(out)?.to_str() != Some(expected.as_str()) {
                return Err(invalid(
                    "unfinished transaction: retry using the previous output path",
                ));
            }
            restore(&pending.join("undo"), out)?;
        }
        restore(&pending.join("undo"), &dir(root).join(snapshot))?;
        let temp = dir(root).join(format!("recovered-{}.tmp", nonce()));
        durable(&temp, &fs::read(pending.join("before"))?)?;
        publish(&temp, &dir(root).join("layout.bin"))?;
    }
    // Without a completed receipt we conservatively export the full ROM once.
    if !committed {
        let _ = fs::remove_file(dir(root).join("output.receipt"));
    }
    retire_journal(root)
}

pub(super) fn commit(
    root: &Path,
    base: &Path,
    out: &Path,
    next: &State,
    patches: Vec<(u64, Vec<u8>)>,
    new_length: u64,
) -> io::Result<()> {
    commit_with_checkpoint(root, base, out, next, patches, new_length, |_| Ok(()))
}

// Injectable checkpoints exercise failure recovery without production fault
// flags or environment variables. The release closure is optimized away.
fn commit_with_checkpoint(
    root: &Path,
    base: &Path,
    out: &Path,
    next: &State,
    patches: Vec<(u64, Vec<u8>)>,
    new_length: u64,
    checkpoint: impl Fn(u8) -> io::Result<()>,
) -> io::Result<()> {
    let out = absolute_output(out)?;
    let previous = fs::read(dir(root).join("layout.bin"))?;
    let next_bytes = encode(next);
    let receipt_path = dir(root).join("output.receipt");
    let fast = match (fs::read(&receipt_path), receipt(&out, &previous)) {
        (Ok(saved), Ok(current)) => saved == current,
        _ => false,
    };
    let length = fs::metadata(base)?.len();
    if patches.is_empty() && new_length == length && previous == next_bytes && fast {
        println!("Incremental I/O: 0 patched bytes, output reused");
        return Ok(());
    }
    let staging = dir(root).join(format!("journal-{}", nonce()));
    fs::create_dir(&staging)?;
    let mut source = File::open(base)?;
    let mut undo = File::create(staging.join("undo"))?;
    undo.write_all(&length.to_le_bytes())?;
    let ranges: Vec<_> = patches
        .iter()
        .filter_map(|(start, b)| {
            let size = (b.len() as u64).min(length.saturating_sub(*start));
            (size > 0).then_some((*start, size))
        })
        .collect();
    undo.write_all(&(ranges.len() as u64).to_le_bytes())?;
    for (start, size) in ranges {
        undo.write_all(&start.to_le_bytes())?;
        undo.write_all(&size.to_le_bytes())?;
        // Do not seek the journal destination: records are sequential.
        source.seek(SeekFrom::Start(start))?;
        if io::copy(&mut (&mut source).take(size), &mut undo)? != size {
            return Err(invalid("snapshot truncated while journaling"));
        }
    }
    undo.sync_all()?;
    drop(undo);
    drop(source);
    durable(&staging.join("before"), &previous)?;
    durable(&staging.join("next"), &next_bytes)?;
    durable(&staging.join("snapshot"), next.snapshot.as_bytes())?;
    if fast {
        durable(&staging.join("output"), out.to_str().unwrap().as_bytes())?;
    }
    let pending = dir(root).join("pending");
    fs::rename(&staging, &pending)?;
    // Never trust a partially updated output after interruption.
    if receipt_path.exists() {
        fs::remove_file(&receipt_path)?;
    }
    let apply = |target: &Path| -> io::Result<()> {
        let mut f = OpenOptions::new().read(true).write(true).open(target)?;
        if new_length > length {
            fill(&mut f, length, new_length)?;
        }
        for (start, b) in &patches {
            write_at(&mut f, *start, b)?;
        }
        f.sync_all()
    };
    let result = (|| {
        apply(base)?;
        checkpoint(1)?;
        if fast {
            apply(&out)?;
        } else {
            let temp = out.with_extension(format!("ndstool-{}.tmp", nonce()));
            fs::copy(base, &temp)?;
            OpenOptions::new().write(true).open(&temp)?.sync_all()?;
            publish(&temp, &out)?;
        }
        checkpoint(2)?;
        save(root, next)?;
        checkpoint(3)?;
        let temp = dir(root).join(format!("receipt-{}.tmp", nonce()));
        durable(&temp, &receipt(&out, &next_bytes)?)?;
        publish(&temp, &receipt_path)?;
        retire_journal(root)?;
        println!(
            "Incremental I/O: {} patched bytes, output {}",
            patches.iter().map(|(_, b)| b.len() as u64).sum::<u64>(),
            if fast { "patched" } else { "exported" }
        );
        Ok(())
    })();
    if let Err(error) = result {
        recover(root, &out)
            .map_err(|recovery| invalid(&format!("{error}; recovery failed: {recovery}")))?;
        return Err(error);
    }
    Ok(())
}
