use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ndstool-full-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("data/sub dir")).unwrap();
        fs::write(path.join("arm9"), [1; 100]).unwrap();
        fs::write(path.join("arm7"), [2; 100]).unwrap();
        for (name, size, byte) in [
            ("DLmachine.dat", 0, 3),
            ("charmake.dat", 513, 4),
            ("sub dir/large", 2_100_003, 5),
            ("sub dir/empty", 0, 0),
        ] {
            fs::write(path.join("data").join(name), vec![byte; size]).unwrap();
        }
        Self(path)
    }
    fn build(&self, name: &str, extra: &[&str]) -> PathBuf {
        let out = self.0.join(name);
        let result = Command::new(env!("CARGO_BIN_EXE_ndstool-rs"))
            .arg("-c")
            .arg(&out)
            .arg("-9")
            .arg(self.0.join("arm9"))
            .arg("-7")
            .arg(self.0.join("arm7"))
            .arg("-d")
            .arg(self.0.join("data"))
            .args(extra)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        out
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parallel_full_output_is_identical_and_can_replace_existing_output() {
    let f = Fixture::new();
    let expected = fs::read(f.build("serial.nds", &[])).unwrap();
    for jobs in ["1", "2", "3", "20"] {
        assert_eq!(
            fs::read(f.build("parallel.nds", &["--jobs", jobs])).unwrap(),
            expected
        );
    }
    assert!(!fs::read_dir(&f.0).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".ndstool-build-")));
}

#[test]
fn random_layout_preserves_ids_names_and_payloads() {
    let f = Fixture::new();
    let stable = fs::read(f.build("stable.nds", &[])).unwrap();
    let random =
        fs::read(f.build("random.nds", &["--jobs", "3", "--enable-random-layout"])).unwrap();
    fn field(b: &[u8], p: usize) -> usize {
        u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize
    }
    let sfnt = field(&stable, 0x40);
    let rfnt = field(&random, 0x40);
    let n = field(&stable, 0x44);
    assert_eq!(&stable[sfnt..sfnt + n], &random[rfnt..rfnt + n]);
    let sfat = field(&stable, 0x48);
    let rfat = field(&random, 0x48);
    for i in (0..field(&stable, 0x4c)).step_by(8) {
        assert_eq!(
            &stable[field(&stable, sfat + i)..field(&stable, sfat + i + 4)],
            &random[field(&random, rfat + i)..field(&random, rfat + i + 4)]
        );
    }
}

#[test]
fn output_can_be_one_of_the_inputs_without_truncating_it() {
    let f = Fixture::new();
    let expected = fs::read(f.build("expected.nds", &[])).unwrap();
    assert_eq!(fs::read(f.build("arm9", &[])).unwrap(), expected);
}
