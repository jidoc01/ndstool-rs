use std::io;

pub(crate) fn lookup(magic: &[u32; 0x412], value: u32) -> u32 {
    let a = magic[((value >> 24) & 0xff) as usize + 18];
    let b = magic[((value >> 16) & 0xff) as usize + 18 + 256];
    let c = magic[((value >> 8) & 0xff) as usize + 18 + 512];
    let d = magic[(value & 0xff) as usize + 18 + 768];
    d.wrapping_add(c ^ b.wrapping_add(a))
}

pub(crate) fn encrypt(magic: &[u32; 0x412], left: &mut u32, right: &mut u32) {
    let mut a = *left;
    let mut b = *right;
    for i in 0..16 {
        let c = magic[i] ^ a;
        a = b ^ lookup(magic, c);
        b = c;
    }
    *right = a ^ magic[16];
    *left = b ^ magic[17];
}

pub(crate) fn decrypt(magic: &[u32; 0x412], left: &mut u32, right: &mut u32) {
    let mut a = *left;
    let mut b = *right;
    for i in (2..=17).rev() {
        let c = magic[i] ^ a;
        a = b ^ lookup(magic, c);
        b = c;
    }
    *left = b ^ magic[0];
    *right = a ^ magic[1];
}

pub(crate) fn update_hashtable(magic: &mut [u32; 0x412], key: &[u8; 8]) {
    for j in 0..18 {
        let mut value = 0u32;
        for i in 0..4 {
            value = (value << 8) | key[(j * 4 + i) & 7] as u32;
        }
        magic[j] ^= value;
    }
    let mut left = 0u32;
    let mut right = 0u32;
    for i in (0..18).step_by(2) {
        encrypt(magic, &mut left, &mut right);
        magic[i] = left;
        magic[i + 1] = right;
    }
    for i in (0..0x400).step_by(2) {
        encrypt(magic, &mut left, &mut right);
        magic[i + 18] = left;
        magic[i + 19] = right;
    }
}

pub(crate) fn validate_block(block: &[u8]) -> io::Result<()> {
    if block.len() < 0x800 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure area is smaller than 0x800 bytes",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decrypt, encrypt};

    #[test]
    fn encrypt_decrypt_round_trip() {
        let mut magic = [0u32; 0x412];
        for (i, value) in magic.iter_mut().enumerate() {
            *value = (i as u32)
                .wrapping_mul(0x1020304)
                .rotate_left((i % 31) as u32);
        }
        let (left, right) = (0x12345678, 0x9abcdef0);
        let mut encoded = (left, right);
        encrypt(&magic, &mut encoded.0, &mut encoded.1);
        decrypt(&magic, &mut encoded.0, &mut encoded.1);
        assert_eq!(encoded, (left, right));
    }
}
