// FNV-1a 64-bit
pub fn hash_field(name: &str) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET;

    for byte in name.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }

    hash
}
