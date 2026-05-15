use std::io;

use mantle::codec::Decode;
use mantle::primitives::{Block, Hash256};

const SLOT_SIZE: usize = 4 * 1024 * 1024;
const SLOT_COUNT: usize = 4;

/// Lifetime-bound handle to a filled slab slot. Borrows the slab's buffer,
/// preventing the slot from being reused until the handle is dropped.
pub struct SlabHandle<'a> {
    data: &'a [u8],
    slot: u16,
}

impl<'a> SlabHandle<'a> {
    #[must_use]
    pub fn as_slice(&self) -> &'a [u8] {
        self.data
    }

    #[must_use]
    pub fn slot(&self) -> u16 {
        self.slot
    }
}

/// Pre-allocated multi-slot block I/O slab, owned by the file-io worker.
///
/// Each slot is a contiguous `SLOT_SIZE`-byte region within a single
/// allocation. `io_uring` reads into slots via the safe `Sqe::read` API.
/// After a read, the caller gets a [`SlabHandle`] that borrows the data —
/// the Rust borrow checker enforces that the slot cannot be reused while
/// the handle is alive. No `unsafe`, no raw pointers, no `set_len`.
pub struct BlockIoSlab {
    buf: Vec<u8>,
    used_len: Vec<u32>,
    free: Vec<u16>,
}

impl BlockIoSlab {
    pub fn new() -> Self {
        let buf = vec![0u8; SLOT_COUNT * SLOT_SIZE];
        let used_len = vec![0u32; SLOT_COUNT];
        let free = (0..u16::try_from(SLOT_COUNT).expect("SLOT_COUNT fits u16"))
            .rev()
            .collect();
        Self {
            buf,
            used_len,
            free,
        }
    }

    pub fn alloc(&mut self) -> Option<u16> {
        self.free.pop()
    }

    pub fn release(&mut self, slot: u16) {
        let idx = slot as usize;
        if idx >= SLOT_COUNT {
            return;
        }
        if self.used_len[idx] == 0 {
            return;
        }
        self.used_len[idx] = 0;
        self.free.push(slot);
    }

    fn slot_mut(&mut self, slot: u16, len: usize) -> &mut [u8] {
        let off = (slot as usize) * SLOT_SIZE;
        &mut self.buf[off..off + len]
    }

    fn slot_ref(&self, slot: u16) -> &[u8] {
        let idx = slot as usize;
        let off = idx * SLOT_SIZE;
        let len = self.used_len[idx] as usize;
        &self.buf[off..off + len]
    }

    /// Get a lifetime-bound handle to a filled slot's data.
    pub fn handle(&self, slot: u16) -> SlabHandle<'_> {
        SlabHandle {
            data: self.slot_ref(slot),
            slot,
        }
    }

    /// Read from disk into a slab slot using the safe `Sqe::read` API.
    fn fill_slot(
        &mut self,
        store: &crate::store::FileStore,
        ring: &mut ququmatz::IoUring,
        slot: u16,
        offset: u64,
        len: u32,
    ) -> io::Result<()> {
        let len_usize = len as usize;
        if len_usize > SLOT_SIZE {
            return Err(io::Error::other(format!(
                "block {len} bytes exceeds slot size {SLOT_SIZE}"
            )));
        }
        let dst = self.slot_mut(slot, len_usize);
        let sqe = ququmatz::Sqe::read(store.fd(), dst, offset).user_data(u64::MAX);
        ring.push(sqe)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        loop {
            ring.submit_and_wait(1)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;
            while let Some(cqe) = ring.complete() {
                if cqe.user_data == u64::MAX {
                    if cqe.result < 0 {
                        return Err(io::Error::from_raw_os_error(-cqe.result));
                    }
                    self.used_len[slot as usize] = len;
                    return Ok(());
                }
            }
        }
    }

    /// Read block bytes from disk, decode, and verify the header hash.
    /// The slab slot is allocated, filled, used for decode, and released
    /// within this call — no data escapes the worker.
    pub fn read_decode_verify(
        &mut self,
        store: &crate::store::FileStore,
        ring: &mut ququmatz::IoUring,
        offset: u64,
        len: u32,
        expected_hash: Hash256,
    ) -> io::Result<Block> {
        let slot = self
            .alloc()
            .ok_or_else(|| io::Error::other("block-io slab full"))?;
        let result = self.fill_and_decode(store, ring, slot, offset, len, expected_hash);
        self.release(slot);
        result
    }

    fn fill_and_decode(
        &mut self,
        store: &crate::store::FileStore,
        ring: &mut ququmatz::IoUring,
        slot: u16,
        offset: u64,
        len: u32,
        expected_hash: Hash256,
    ) -> io::Result<Block> {
        self.fill_slot(store, ring, slot, offset, len)?;
        let data = self.slot_ref(slot);
        let block = Block::decode(&mut io::Cursor::new(data))
            .map_err(|e| io::Error::other(format!("decode: {e}")))?;
        if block.header.hash() != expected_hash {
            return Err(io::Error::other(format!(
                "hash mismatch: expected {}, got {}",
                expected_hash.to_display_hex(),
                block.header.hash().to_display_hex()
            )));
        }
        Ok(block)
    }

    /// Read raw block bytes, returning an owned copy.
    /// Fallback for callers that need raw bytes without decode.
    pub fn read_raw(
        &mut self,
        store: &crate::store::FileStore,
        ring: &mut ququmatz::IoUring,
        offset: u64,
        len: u32,
    ) -> io::Result<Vec<u8>> {
        let slot = self
            .alloc()
            .ok_or_else(|| io::Error::other("block-io slab full"))?;
        self.fill_slot(store, ring, slot, offset, len)?;
        let data = self.slot_ref(slot).to_vec();
        self.release(slot);
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_release() {
        let mut slab = BlockIoSlab::new();
        assert_eq!(slab.free.len(), SLOT_COUNT);

        let slot = slab.alloc().expect("should allocate");
        assert_eq!(slab.free.len(), SLOT_COUNT - 1);

        slab.used_len[slot as usize] = 100;
        slab.release(slot);
        assert_eq!(slab.free.len(), SLOT_COUNT);
    }

    #[test]
    fn double_release_is_noop() {
        let mut slab = BlockIoSlab::new();
        let slot = slab.alloc().expect("should allocate");
        slab.used_len[slot as usize] = 100;
        slab.release(slot);
        let before = slab.free.len();
        slab.release(slot);
        assert_eq!(slab.free.len(), before);
    }

    #[test]
    fn exhaust_slots() {
        let mut slab = BlockIoSlab::new();
        let mut slots = Vec::new();
        for _ in 0..SLOT_COUNT {
            slots.push(slab.alloc().expect("should allocate"));
        }
        assert!(slab.alloc().is_none());

        slab.used_len[slots[0] as usize] = 1;
        slab.release(slots[0]);
        assert!(slab.alloc().is_some());
    }

    #[test]
    fn handle_borrows_data() {
        let mut slab = BlockIoSlab::new();
        let slot = slab.alloc().expect("should allocate");
        let off = (slot as usize) * SLOT_SIZE;
        slab.buf[off] = 0xAB;
        slab.buf[off + 1] = 0xCD;
        slab.used_len[slot as usize] = 2;

        let handle = slab.handle(slot);
        assert_eq!(handle.as_slice(), &[0xAB, 0xCD]);
        assert_eq!(handle.slot(), slot);
    }

    #[test]
    fn release_out_of_bounds_is_noop() {
        let mut slab = BlockIoSlab::new();
        slab.release(u16::MAX);
        assert_eq!(slab.free.len(), SLOT_COUNT);
    }
}
