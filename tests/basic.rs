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
fn places_arm7_after_large_arm9() {
    let dir = temp_dir();
    let arm9 = dir.join("large-arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    let out7 = dir.join("out7.bin");
    fs::write(&arm9, vec![0x9au8; 0x9000]).unwrap();
    fs::write(&arm7, [5u8, 6, 7]).unwrap();
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
    let inspected = Command::new(bin)
        .args(["-i", rom.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(stdout.contains("ARM7        : 0x00009200"));
    assert!(Command::new(bin)
        .args(["-x", rom.to_str().unwrap(), "-7", out7.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert_eq!(fs::read(out7).unwrap(), [5, 6, 7]);
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

#[test]
fn create_and_extract_nested_data_file() {
    let dir = temp_dir();
    let data = dir.join("data");
    let nested = data.join("sub");
    let extracted = dir.join("extracted");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("hello.txt"), b"nested hello").unwrap();
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
    let listed = Command::new(bin)
        .args(["-l", rom.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("/sub/hello.txt"));
    assert_eq!(
        fs::read(extracted.join("sub").join("hello.txt")).unwrap(),
        b"nested hello"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn create_from_elf32_load_segment() {
    let dir = temp_dir();
    let elf9 = dir.join("arm9.elf");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    let mut bytes = vec![0u8; 88];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 1;
    bytes[5] = 1;
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&40u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&0x02000004u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&52u32.to_le_bytes());
    bytes[42..44].copy_from_slice(&32u16.to_le_bytes());
    bytes[44..46].copy_from_slice(&1u16.to_le_bytes());
    bytes[52..56].copy_from_slice(&1u32.to_le_bytes());
    bytes[56..60].copy_from_slice(&84u32.to_le_bytes());
    bytes[60..64].copy_from_slice(&0x02000000u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&4u32.to_le_bytes());
    bytes[72..76].copy_from_slice(&8u32.to_le_bytes());
    bytes[84..88].copy_from_slice(&[9, 8, 7, 6]);
    fs::write(&elf9, bytes).unwrap();
    fs::write(&arm7, [1u8]).unwrap();
    let bin = env!("CARGO_BIN_EXE_ndstool-rs");
    assert!(Command::new(bin)
        .args([
            "-c",
            rom.to_str().unwrap(),
            "-9",
            elf9.to_str().unwrap(),
            "-7",
            arm7.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    let inspected = Command::new(bin)
        .args(["-i", rom.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(stdout.contains("entry 0x02000004"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn create_from_elf32_overlay_segments() {
    let dir = temp_dir();
    let elf9 = dir.join("arm9-overlays.elf");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    let overlays = dir.join("overlays");
    let mut bytes = vec![0u8; 168];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 1;
    bytes[5] = 1;
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&40u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&0x02000000u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&52u32.to_le_bytes());
    bytes[40..42].copy_from_slice(&52u16.to_le_bytes());
    bytes[42..44].copy_from_slice(&32u16.to_le_bytes());
    bytes[44..46].copy_from_slice(&3u16.to_le_bytes());

    // Normal ARM9 segment.
    bytes[52..56].copy_from_slice(&1u32.to_le_bytes());
    bytes[56..60].copy_from_slice(&148u32.to_le_bytes());
    bytes[60..64].copy_from_slice(&0x02000000u32.to_le_bytes());
    bytes[64..68].copy_from_slice(&0x02000000u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&4u32.to_le_bytes());
    bytes[72..76].copy_from_slice(&4u32.to_le_bytes());
    bytes[76..80].copy_from_slice(&5u32.to_le_bytes());

    // Overlay table segment: one short entry.
    bytes[84..88].copy_from_slice(&1u32.to_le_bytes());
    bytes[88..92].copy_from_slice(&152u32.to_le_bytes());
    bytes[92..96].copy_from_slice(&0u32.to_le_bytes());
    bytes[96..100].copy_from_slice(&0u32.to_le_bytes());
    bytes[100..104].copy_from_slice(&12u32.to_le_bytes());
    bytes[104..108].copy_from_slice(&12u32.to_le_bytes());
    bytes[108..112].copy_from_slice(&0x0020_0000u32.to_le_bytes());

    // Overlay payload segment.
    bytes[116..120].copy_from_slice(&1u32.to_le_bytes());
    bytes[120..124].copy_from_slice(&164u32.to_le_bytes());
    bytes[124..128].copy_from_slice(&0x02200000u32.to_le_bytes());
    bytes[128..132].copy_from_slice(&0x02200000u32.to_le_bytes());
    bytes[132..136].copy_from_slice(&4u32.to_le_bytes());
    bytes[136..140].copy_from_slice(&8u32.to_le_bytes());
    bytes[140..144].copy_from_slice(&0x0020_0000u32.to_le_bytes());

    bytes[148..152].copy_from_slice(&[9, 8, 7, 6]);
    bytes[152..156].copy_from_slice(&0x11111111u32.to_le_bytes());
    bytes[156..160].copy_from_slice(&0x22222222u32.to_le_bytes());
    bytes[164..168].copy_from_slice(b"OVL!");
    fs::write(&elf9, bytes).unwrap();
    fs::write(&arm7, [1u8]).unwrap();

    let bin = env!("CARGO_BIN_EXE_ndstool-rs");
    assert!(Command::new(bin)
        .args([
            "-c",
            rom.to_str().unwrap(),
            "-9",
            elf9.to_str().unwrap(),
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
            "-y",
            overlays.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(
        fs::read(overlays.join("overlay_0000.bin")).unwrap(),
        b"OVL!"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn create_and_extract_raw_banner() {
    let dir = temp_dir();
    let banner = dir.join("banner.bin");
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    let extracted = dir.join("banner-out.bin");
    let logo = dir.join("logo.bin");
    let logo_out = dir.join("logo-out.bin");
    let mut banner_bytes = vec![0u8; 0x840];
    banner_bytes[0] = 1;
    banner_bytes[2] = 0x34;
    banner_bytes[3] = 0x12;
    fs::write(&banner, &banner_bytes).unwrap();
    let logo_bytes: Vec<u8> = (0..156).map(|i| i as u8).collect();
    fs::write(&logo, &logo_bytes).unwrap();
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
            "-t",
            banner.to_str().unwrap(),
            "-o",
            logo.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert!(Command::new(bin)
        .args([
            "-x",
            rom.to_str().unwrap(),
            "-b",
            extracted.to_str().unwrap(),
            "-o",
            logo_out.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(fs::read(extracted).unwrap(), banner_bytes);
    assert_eq!(fs::read(logo_out).unwrap(), logo_bytes);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn create_banner_from_bmp() {
    let dir = temp_dir();
    let bmp = dir.join("icon.bmp");
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    let extracted = dir.join("banner.bin");
    let row = 32 * 3;
    let stride = (row + 3) / 4 * 4;
    let mut bmp_bytes = vec![0u8; 54 + stride * 32];
    let bmp_size = bmp_bytes.len() as u32;
    bmp_bytes[0..2].copy_from_slice(b"BM");
    bmp_bytes[2..6].copy_from_slice(&bmp_size.to_le_bytes());
    bmp_bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp_bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp_bytes[18..22].copy_from_slice(&32i32.to_le_bytes());
    bmp_bytes[22..26].copy_from_slice(&32i32.to_le_bytes());
    bmp_bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp_bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..32 {
        for x in 0..32 {
            let p = 54 + y * stride + x * 3;
            bmp_bytes[p..p + 3].copy_from_slice(&[0, 0, 255]);
        }
    }
    fs::write(&bmp, bmp_bytes).unwrap();
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
            "-t",
            bmp.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert!(Command::new(bin)
        .args([
            "-x",
            rom.to_str().unwrap(),
            "-b",
            extracted.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(fs::metadata(extracted).unwrap().len(), 0x840);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn extract_overlay_from_table() {
    let dir = temp_dir();
    let data = dir.join("data");
    let out = dir.join("out");
    let overlays = dir.join("overlays");
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    fs::create_dir_all(&data).unwrap();
    let payload = b"overlay payload";
    fs::write(data.join("payload.bin"), payload).unwrap();
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
    let mut bytes = fs::read(&rom).unwrap();
    let fat = u32::from_le_bytes(bytes[0x48..0x4c].try_into().unwrap()) as usize;
    let file_start = u32::from_le_bytes(bytes[fat..fat + 4].try_into().unwrap());
    let table = (bytes.len() + 0x1ff) & !0x1ff;
    bytes.resize(table + 32, 0xff);
    bytes[table..table + 4].copy_from_slice(&7u32.to_le_bytes());
    bytes[table + 24..table + 28].copy_from_slice(&0u32.to_le_bytes());
    bytes[0x50..0x54].copy_from_slice(&(table as u32).to_le_bytes());
    bytes[0x54..0x58].copy_from_slice(&32u32.to_le_bytes());
    fs::write(&rom, bytes).unwrap();
    let _ = file_start;
    assert!(Command::new(bin)
        .args([
            "-x",
            rom.to_str().unwrap(),
            "-d",
            out.to_str().unwrap(),
            "-y",
            overlays.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(
        fs::read(overlays.join("overlay_0007.bin")).unwrap(),
        payload
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn create_from_external_overlay_table() {
    let dir = temp_dir();
    let overlay_root = dir.join("overlay-input");
    let extracted = dir.join("overlay-output");
    let table = dir.join("arm9.tbl");
    let arm9 = dir.join("arm9.bin");
    let arm7 = dir.join("arm7.bin");
    let rom = dir.join("test.nds");
    fs::create_dir_all(&overlay_root).unwrap();
    fs::write(overlay_root.join("overlay_0003.bin"), b"external overlay").unwrap();
    let mut table_bytes = vec![0u8; 32];
    table_bytes[0..4].copy_from_slice(&3u32.to_le_bytes());
    table_bytes[4..8].copy_from_slice(&0x02200000u32.to_le_bytes());
    table_bytes[8..12].copy_from_slice(&16u32.to_le_bytes());
    table_bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
    fs::write(&table, table_bytes).unwrap();
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
            "-y9",
            table.to_str().unwrap(),
            "-y",
            overlay_root.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert!(Command::new(bin)
        .args([
            "-x",
            rom.to_str().unwrap(),
            "-y",
            extracted.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(
        fs::read(extracted.join("overlay_0003.bin")).unwrap(),
        b"external overlay"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn secure_area_encrypt_decrypt_round_trip() {
    let dir = temp_dir();
    let rom = dir.join("secure.nds");
    let mut bytes = vec![0xffu8; 0x8000];
    bytes[12..16].copy_from_slice(b"TEST");
    bytes[0x4000..0x4008].fill(0xff);
    bytes[0x4000..0x4004].copy_from_slice(&0xe7ffdeffu32.to_le_bytes());
    bytes[0x4004..0x4008].copy_from_slice(&0xe7ffdeffu32.to_le_bytes());
    let original = bytes[0x4000..0x4800].to_vec();
    fs::write(&rom, bytes).unwrap();

    let bin = env!("CARGO_BIN_EXE_ndstool-rs");
    assert!(Command::new(bin)
        .args(["-se", rom.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(Command::new(bin)
        .args(["-sd", rom.to_str().unwrap()])
        .status()
        .unwrap()
        .success());

    let result = fs::read(&rom).unwrap();
    assert_eq!(&result[0x4000..0x4800], original.as_slice());
    let _ = fs::remove_dir_all(dir);
}
