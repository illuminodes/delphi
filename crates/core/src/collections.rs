use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};

/// A fast, non-cryptographic hasher for keys that are already uniformly
/// distributed (e.g. `Hash256`, `OutPoint`). Reads 8 bytes starting at a
/// fixed offset and uses them directly as the hash value — no mixing needed
/// because the input bytes are already the output of SHA-256d.
///
/// **Safety**: only use this for types whose byte representation is already
/// cryptographically random. Never use it for user-supplied strings or integers.
#[derive(Clone, Copy, Default)]
pub struct PrehashedHasher {
    state: u64,
}

impl Hasher for PrehashedHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Take the first 8 bytes as a little-endian u64. Callers write
        // fixed-size types so `bytes` is never shorter than 8 in practice
        // (`Hash256` is 32 bytes, `OutPoint` is 36 bytes). Pad with zeros
        // if somehow shorter — the hasher must not panic.
        let mut buf = [0u8; 8];
        let n = bytes.len().min(8);
        buf[..n].copy_from_slice(&bytes[..n]);
        self.state = u64::from_le_bytes(buf);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.state
    }
}

/// `BuildHasher` that constructs [`PrehashedHasher`] instances.
#[derive(Clone, Copy, Default)]
pub struct BuildPrehashedHasher;

impl BuildHasher for BuildPrehashedHasher {
    type Hasher = PrehashedHasher;

    #[inline]
    fn build_hasher(&self) -> Self::Hasher {
        PrehashedHasher::default()
    }
}

/// `HashMap` backed by [`PrehashedHasher`]. Only use for keys that are already
/// cryptographically uniform (e.g. `Hash256`, `OutPoint`).
pub type PrehashedMap<K, V> = HashMap<K, V, BuildPrehashedHasher>;

/// `HashSet` backed by [`PrehashedHasher`]. Only use for keys that are already
/// cryptographically uniform (e.g. `Hash256`, `OutPoint`).
pub type PrehashedSet<K> = HashSet<K, BuildPrehashedHasher>;
