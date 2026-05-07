use sha2::{Digest, Sha256};

/// `SHA256d`: double-SHA256, used for block/tx hashes and message checksums.
#[inline]
#[must_use]
pub fn sha256d(data: &[u8]) -> [u8; 32] {
    let first = Sha256::digest(data);
    Sha256::digest(first).into()
}

/// First 4 bytes of `SHA256d` — used as the P2P message checksum.
///
/// # Errors
/// Never returns `Err` in practice; the `Result` type allows callers to
/// propagate it uniformly with `?` without unwrapping.
#[inline]
pub fn checksum(data: &[u8]) -> std::io::Result<[u8; 4]> {
    let [a, b, c, d, ..] = sha256d(data);
    Ok([a, b, c, d])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256d_empty() {
        // SHA256d("") is a known value
        let hash = sha256d(b"");
        assert_eq!(
            hash,
            [
                0x5d, 0xf6, 0xe0, 0xe2, 0x76, 0x13, 0x59, 0xd3, 0x0a, 0x82, 0x75, 0x05, 0x8e, 0x29,
                0x9f, 0xcc, 0x03, 0x81, 0x53, 0x45, 0x45, 0xf5, 0x5c, 0xf4, 0x3e, 0x41, 0x98, 0x3f,
                0x5d, 0x4c, 0x94, 0x56,
            ]
        );
    }

    #[test]
    fn checksum_is_first_4_bytes() {
        let data = b"hello";
        let full = sha256d(data);
        assert_eq!(checksum(data).expect("checksum cannot fail"), [full[0], full[1], full[2], full[3]]);
    }
}
