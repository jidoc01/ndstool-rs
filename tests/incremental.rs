use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    data: PathBuf,
    source: PathBuf,
    output: PathBuf,
}
static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ndstool-rs"))
        .args(args)
        .output()
        .unwrap()
}
fn ok(args: &[&str]) -> Output {
    let output = run(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn u32_at(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn listing(rom: &Path) -> BTreeMap<String, (u32, u32, u32)> {
    let output = ok(&["-l", p(rom)]);
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let f = l.split_whitespace().collect::<Vec<_>>();
            (
                f[4].to_owned(),
                (
                    f[0].parse().unwrap(),
                    u32::from_str_radix(&f[1][2..], 16).unwrap(),
                    u32::from_str_radix(&f[2][2..], 16).unwrap(),
                ),
            )
        })
        .collect()
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ndstool-incremental-{}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let seed = root.join("seed");
        fs::create_dir_all(seed.join("sub")).unwrap();
        for (n, b) in [
            ("a.bin", vec![1; 512]),
            ("b.bin", vec![2; 512]),
            ("c.bin", vec![3; 512]),
            ("sub/d.bin", vec![4; 100]),
        ] {
            fs::write(seed.join(n), b).unwrap();
        }
        let a9 = root.join("a9");
        let a7 = root.join("a7");
        fs::write(&a9, vec![0x77; 128]).unwrap();
        fs::write(&a7, vec![0x55; 128]).unwrap();
        let source = root.join("source.nds");
        let data = root.join("data");
        let output = root.join("output.nds");
        ok(&["-c", p(&source), "-9", p(&a9), "-7", p(&a7), "-d", p(&seed)]);
        ok(&["-x", p(&source), "-d", p(&data), "--incremental"]);
        Self {
            root,
            data,
            source,
            output,
        }
    }
    fn link(&self) -> Output {
        ok(&["-c", p(&self.output), "-d", p(&self.data), "--incremental"])
    }
    fn verify(&self) {
        let items = listing(&self.output);
        fn collect(root: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for e in fs::read_dir(root).unwrap() {
                let e = e.unwrap();
                if e.file_name() == ".ndstool-rs" {
                    continue;
                }
                if e.file_type().unwrap().is_dir() {
                    collect(&e.path(), base, out)
                } else {
                    out.insert(
                        format!(
                            "/{}",
                            e.path()
                                .strip_prefix(base)
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .replace('\\', "/")
                        ),
                        fs::read(e.path()).unwrap(),
                    );
                }
            }
        }
        let mut expected = BTreeMap::new();
        collect(&self.data, &self.data, &mut expected);
        assert_eq!(
            items.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>()
        );
        let bytes = fs::read(&self.output).unwrap();
        for (path, (_, s, e)) in items {
            assert_eq!(&bytes[s as usize..e as usize], expected[&path], "{path}");
        }
        let original = fs::read(&self.source).unwrap();
        for off in [0x20, 0x30] {
            let s = u32_at(&original, off) as usize;
            let n = u32_at(&original, off + 12) as usize;
            assert_eq!(&bytes[s..s + n], &original[s..s + n]);
        }
    }
}

#[test]
fn all_four_edits_and_repeated_builds_update_the_previous_state() {
    let f = Fixture::new();
    let before = listing(&f.source);
    fs::write(f.data.join("b.bin"), vec![8; 4096]).unwrap();
    fs::write(f.data.join("a.bin"), [9; 512]).unwrap();
    f.link();
    f.verify();
    let after = listing(&f.output);
    assert_eq!(after["/c.bin"].1, before["/c.bin"].1);
    assert_ne!(after["/b.bin"].1, before["/b.bin"].1);
    assert_eq!(after["/a.bin"].0, before["/a.bin"].0);
    let state_before = fs::read(f.data.join(".ndstool-rs/layout.bin")).unwrap();
    let nochange = f.link();
    assert!(String::from_utf8_lossy(&nochange.stdout).contains("0 changed/new"));
    assert_eq!(
        state_before,
        fs::read(f.data.join(".ndstool-rs/layout.bin")).unwrap()
    );
    f.verify();

    fs::write(f.data.join("b.bin"), [6; 12]).unwrap();
    f.link();
    f.verify();
    fs::write(f.data.join("new.bin"), [7; 700]).unwrap();
    f.link();
    f.verify();
    let ids = listing(&f.output);
    assert_eq!(ids["/sub/d.bin"].0, before["/sub/d.bin"].0);
    fs::remove_file(f.data.join("b.bin")).unwrap();
    f.link();
    f.verify();
    f.link();
    f.verify(); // Retired FAT slots must not alias live entries.
    fs::create_dir(f.data.join("another")).unwrap();
    fs::write(f.data.join("another/empty.bin"), []).unwrap();
    f.link();
    f.verify();
}

#[test]
fn no_change_preserves_every_rom_byte() {
    let f = Fixture::new();
    f.link();
    assert_eq!(fs::read(&f.source).unwrap(), fs::read(&f.output).unwrap());
    let modified = fs::metadata(&f.output).unwrap().modified().unwrap();
    let repeat = f.link();
    assert!(String::from_utf8_lossy(&repeat.stdout).contains("0 patched bytes, output reused"));
    assert_eq!(
        modified,
        fs::metadata(&f.output).unwrap().modified().unwrap()
    );
}

#[test]
fn ten_successive_edits_of_each_kind_reuse_the_output() {
    for operation in 0..4 {
        let f = Fixture::new();
        for i in 0..10 {
            fs::write(f.data.join(format!("test-{i}.bin")), vec![i as u8; 1024]).unwrap();
        }
        f.link();
        for i in 0..10 {
            let path = f.data.join(format!("test-{i}.bin"));
            match operation {
                0 => fs::write(f.data.join(format!("added-{i}.bin")), vec![42; 700 + i]).unwrap(),
                1 => fs::remove_file(path).unwrap(),
                2 => fs::write(path, vec![43; 1024 + 65536 * (i + 1)]).unwrap(),
                _ => fs::write(path, vec![44; 512 - i]).unwrap(),
            }
            let result = f.link();
            assert!(String::from_utf8_lossy(&result.stdout).contains("output patched"));
            f.verify();
        }
    }
}

#[test]
fn changed_or_different_output_gets_a_full_export() {
    let f = Fixture::new();
    f.link();
    fs::write(&f.output, b"externally replaced output").unwrap();
    let repaired = f.link();
    assert!(String::from_utf8_lossy(&repaired.stdout).contains("output exported"));
    f.verify();
    let other = f.root.join("other.nds");
    let result = ok(&["-c", p(&other), "-d", p(&f.data), "--incremental"]);
    assert!(String::from_utf8_lossy(&result.stdout).contains("output exported"));
    assert_eq!(fs::read(other).unwrap(), fs::read(&f.output).unwrap());
    let folder = f.root.join("keep-directory");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("keep"), b"keep").unwrap();
    assert!(!run(&["-c", p(&folder), "-d", p(&f.data), "--incremental"])
        .status
        .success());
    assert_eq!(fs::read(folder.join("keep")).unwrap(), b"keep");
}

#[test]
fn mixed_edits_survive_one_hundred_previous_state_updates() {
    let f = Fixture::new();
    f.link();
    let mut random = 0x12345678u32;
    for step in 0..100 {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        let path = f.data.join(format!("mixed-{}.bin", (random >> 8) % 12));
        let size = fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0);
        match (random >> 16) % 4 {
            0 => fs::write(path, vec![step as u8; 31 + step]).unwrap(),
            1 if path.exists() => fs::remove_file(path).unwrap(),
            2 => fs::write(path, vec![step as u8; size + 2049]).unwrap(),
            _ => fs::write(path, vec![step as u8; size / 2]).unwrap(),
        }
        f.link();
        f.verify();
    }
}

#[test]
fn batches_of_one_through_ten_edits_preserve_unchanged_layout() {
    for count in 1..=10 {
        let f = Fixture::new();
        for i in 0..count {
            for kind in ["remove", "grow", "shrink"] {
                fs::write(f.data.join(format!("{kind}-{i}.bin")), [11; 512]).unwrap();
            }
            // Multiple empty files may share offsets; growing them must not
            // cause their newly allocated ranges to overlap each other.
            fs::write(f.data.join(format!("empty-{i}.bin")), []).unwrap();
        }
        f.link();
        let before = listing(&f.output);
        for i in 0..count {
            fs::write(f.data.join(format!("added-{i}.bin")), [22; 600]).unwrap();
            fs::remove_file(f.data.join(format!("remove-{i}.bin"))).unwrap();
            fs::write(f.data.join(format!("grow-{i}.bin")), vec![33; 65536]).unwrap();
            fs::write(f.data.join(format!("shrink-{i}.bin")), [44; 5]).unwrap();
            fs::write(f.data.join(format!("empty-{i}.bin")), [55; 513]).unwrap();
        }
        f.link();
        f.verify();
        let after = listing(&f.output);
        for name in ["/a.bin", "/b.bin", "/c.bin", "/sub/d.bin"] {
            // Structural edits may remap IDs, but unchanged physical ranges
            // must remain identical even with simultaneous growth/removal.
            assert_eq!(
                (before[name].1, before[name].2),
                (after[name].1, after[name].2)
            );
        }
    }
}

#[test]
fn failure_does_not_replace_output_or_state_and_cli_rejects_ignored_options() {
    let f = Fixture::new();
    f.link();
    let output = fs::read(&f.output).unwrap();
    let state = fs::read(f.data.join(".ndstool-rs/layout.bin")).unwrap();
    let result = run(&[
        "-c",
        p(&f.output),
        "-d",
        p(&f.data),
        "--incremental",
        "-9",
        "ignored.bin",
    ]);
    assert!(!result.status.success());
    fs::write(f.data.join(".ndstool-rs/build.lock"), []).unwrap();
    assert!(
        !run(&["-c", p(&f.output), "-d", p(&f.data), "--incremental"])
            .status
            .success()
    );
    assert_eq!(output, fs::read(&f.output).unwrap());
    assert_eq!(
        state,
        fs::read(f.data.join(".ndstool-rs/layout.bin")).unwrap()
    );
}

#[test]
fn malformed_cache_is_rejected_without_panicking() {
    let f = Fixture::new();
    fs::write(
        f.data.join(".ndstool-rs/layout.bin"),
        b"NDSRS003\xff\xff\xff\xff",
    )
    .unwrap();
    let result = run(&["-c", p(&f.output), "-d", p(&f.data), "--incremental"]);
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"));
    assert!(!f.output.exists());
}

#[test]
fn capacity_growth_zero_length_and_delete_every_file() {
    let f = Fixture::new();
    fs::write(f.data.join("b.bin"), vec![0x9a; 200_000]).unwrap();
    f.link();
    f.verify();
    let bytes = fs::read(&f.output).unwrap();
    assert!(bytes.len() > fs::metadata(&f.source).unwrap().len() as usize);
    assert!(bytes.len().is_power_of_two());
    assert_eq!(128usize * 1024 << bytes[0x14], bytes.len());
    assert!(u32_at(&bytes, 0x80) as usize <= bytes.len());
    fs::write(f.data.join("b.bin"), []).unwrap();
    f.link();
    f.verify();
    for file in ["a.bin", "b.bin", "c.bin", "sub/d.bin"] {
        fs::remove_file(f.data.join(file)).unwrap();
    }
    f.link();
    f.verify();
    fs::write(f.data.join("new.bin"), [42; 777]).unwrap();
    f.link();
    f.verify();
}

#[test]
fn opaque_trailer_is_preserved_and_growth_does_not_overwrite_it() {
    use std::io::{Seek, SeekFrom, Write};
    let f = Fixture::new();
    let last = listing(&f.source)
        .values()
        .map(|(_, _, end)| *end)
        .max()
        .unwrap();
    let trailer_at = (last as u64 + 4095) & !4095;
    let trailer = vec![0xa5; 512];
    let mut source = fs::OpenOptions::new().write(true).open(&f.source).unwrap();
    source.seek(SeekFrom::Start(trailer_at)).unwrap();
    source.write_all(&trailer).unwrap();
    drop(source);
    ok(&["-x", p(&f.source), "-d", p(&f.data), "--incremental"]);
    f.link();
    assert_eq!(fs::read(&f.source).unwrap(), fs::read(&f.output).unwrap());
    fs::write(f.data.join("sub/d.bin"), vec![8; 8192]).unwrap();
    f.link();
    f.verify();
    let output = fs::read(&f.output).unwrap();
    assert_eq!(
        &output[trailer_at as usize..trailer_at as usize + 512],
        trailer
    );
}
