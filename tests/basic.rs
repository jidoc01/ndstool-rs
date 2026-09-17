use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_dir() -> std::path::PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("ndstool-rs-test-{n}"));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn create_and_inspect_basic_rom() {
    let dir = temp_dir();
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    fs::write(&arm9, [1u8, 2, 3, 4]).unwrap();
    fs::write(&arm7, [5u8, 6, 7]).unwrap();

    let bin = env!("CARGO_BIN_EXE_ndstool-rs");
    let created = Command::new(bin)
        .args([
            "-c",
            rom.to_str().unwrap(),
            "-9",
            arm9.to_str().unwrap(),
            "-7",
            arm7.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    let inspected = Command::new(bin)
        .args(["-i", rom.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(inspected.status.success());
    assert!(stdout.contains("ARM9") && stdout.contains("ARM7"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn extract_arm_binaries() {
    let dir = temp_dir();
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    let out9 = dir.join("out9.bin");
    let out7 = dir.join("out7.bin");
    fs::write(&arm9, [9u8, 8, 7, 6]).unwrap();
    fs::write(&arm7, [1u8, 3, 5]).unwrap();
    let bin = env!("CARGO_BIN_EXE_ndstool-rs");
    assert!(Command::new(bin)
        .args([
            "-c",
            rom.to_str().unwrap(),
            "-9",
            arm9.to_str().unwrap(),
            "-7",
            arm7.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert!(Command::new(bin)
        .args([
            "-x",
            rom.to_str().unwrap(),
            "-9",
            out9.to_str().unwrap(),
            "-7",
            out7.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(fs::read(out9).unwrap(), [9, 8, 7, 6]);
    assert_eq!(fs::read(out7).unwrap(), [1, 3, 5]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn create_and_extract_root_data_file() {
    let dir = temp_dir();
    let data = dir.join("data");
    let extracted = dir.join("extracted");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("hello.txt"), b"hello nds").unwrap();
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    fs::write(&arm9, [1u8]).unwrap();
    fs::write(&arm7, [2u8]).unwrap();
    let bin = env!("CARGO_BIN_EXE_ndstool-rs");
    assert!(Command::new(bin)
        .args([
            "-c",
            rom.to_str().unwrap(),
            "-9",
            arm9.to_str().unwrap(),
            "-7",
            arm7.to_str().unwrap(),
            "-d",
            data.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert!(Command::new(bin)
        .args([
            "-x",
            rom.to_str().unwrap(),
            "-d",
            extracted.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(fs::read(extracted.join("hello.txt")).unwrap(), b"hello nds");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn fix_header_crc_action_succeeds() {
    let dir = temp_dir();
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    fs::write(&arm9, [1u8]).unwrap();
    fs::write(&arm7, [2u8]).unwrap();
    let bin = env!("CARGO_BIN_EXE_ndstool-rs");
    assert!(Command::new(bin)
        .args([
            "-c",
            rom.to_str().unwrap(),
            "-9",
            arm9.to_str().unwrap(),
            "-7",
            arm7.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    let mut bytes = fs::read(&rom).unwrap();
    bytes[0] ^= 0xff;
    fs::write(&rom, bytes).unwrap();
    assert!(Command::new(bin)
        .args(["-f", rom.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    let inspected = Command::new(bin)
        .args(["-i", rom.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(inspected.status.success());
    let _ = fs::remove_dir_all(dir);
}
