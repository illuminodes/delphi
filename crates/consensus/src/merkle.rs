use mantle::hash::sha256d;
use mantle::primitives::Hash256;

/// Compute the merkle root from a slice, cloning into a working buffer.
///
/// Returns `None` only on an empty input.
#[must_use]
pub fn compute_merkle_root(txids: &[Hash256]) -> Option<Hash256> {
    merkle_root_owned(txids.to_vec())
}

/// Same as `compute_merkle_root` but consumes an already-owned Vec, avoiding
/// the clone. Use this when you already hold ownership of the txid list.
///
/// # Panics
/// Never — the internal `unwrap` on `last()` is protected by the odd-length
/// pad that runs immediately before it.
#[must_use]
pub fn merkle_root_owned(mut row: Vec<Hash256>) -> Option<Hash256> {
    if row.is_empty() {
        return None;
    }
    let mut buf = [0u8; 64];
    while row.len() > 1 {
        // Pad odd row by duplicating the last element.
        if row.len() % 2 == 1 {
            let last = *row.last().unwrap();
            row.push(last);
        }
        // Fold pairs in-place: write each parent hash into the front half,
        // then truncate. No allocations after the initial vec.
        let parents = row.len() / 2;
        for i in 0..parents {
            buf[..32].copy_from_slice(&row[i * 2].0);
            buf[32..].copy_from_slice(&row[i * 2 + 1].0);
            row[i] = Hash256(sha256d(&buf));
        }
        row.truncate(parents);
    }
    Some(row[0])
}


#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Hash256 {
        Hash256([byte; 32])
    }

    fn txid_from_hex(s: &str) -> Hash256 {
        let bytes: Vec<u8> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect();
        // txids are stored internally in byte order (not display-reversed)
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Hash256(arr)
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(compute_merkle_root(&[]), None);
    }

    #[test]
    fn single_tx_is_its_own_root() {
        let txid = h(0xab);
        assert_eq!(compute_merkle_root(&[txid]), Some(txid));
    }

    #[test]
    fn two_txs() {
        let a = h(0x01);
        let b = h(0x02);
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&a.0);
        buf[32..].copy_from_slice(&b.0);
        let expected = Hash256(sha256d(&buf));
        assert_eq!(compute_merkle_root(&[a, b]), Some(expected));
    }

    #[test]
    fn odd_row_duplicates_last() {
        // Three txids: row becomes [h(AB), h(CC)] then root = h(h(AB) || h(CC))
        let a = h(0xaa);
        let b = h(0xbb);
        let c = h(0xcc);

        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&a.0);
        buf[32..].copy_from_slice(&b.0);
        let ab = Hash256(sha256d(&buf));

        buf[..32].copy_from_slice(&c.0);
        buf[32..].copy_from_slice(&c.0);
        let cc = Hash256(sha256d(&buf));

        buf[..32].copy_from_slice(&ab.0);
        buf[32..].copy_from_slice(&cc.0);
        let expected = Hash256(sha256d(&buf));

        assert_eq!(compute_merkle_root(&[a, b, c]), Some(expected));
    }

    #[test]
    fn genesis_merkle_root() {
        // Genesis block has exactly one transaction. The merkle root equals the
        // txid — no hashing applied. Verify against the known genesis txid.
        // Stored in internal byte order (not display-reversed).
        let genesis_txid_hex =
            "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a";
        let txid = txid_from_hex(genesis_txid_hex);
        let root = compute_merkle_root(&[txid]).unwrap();
        assert_eq!(root, txid);
    }
}
