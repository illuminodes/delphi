//! On-disk Robin Hood hash table mapping txid → `DiskTxHandle`.
//!
//! 64-byte buckets, power-of-two slot count, keyed by `Hash256` (txid).
//! Probing uses the first 8 bytes of the txid as the hash (SHA-256d output
//! is uniformly distributed).

#![allow(clippy::cast_possible_truncation, clippy::missing_errors_doc)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use mantle::primitives::{BlockHeight, Hash256};

use crate::disk_arena::{DiskSpan, DiskTxHandle};

const BUCKET_SIZE: u64 = 64;
const MAGIC: [u8; 8] = *b"MANTLTX\0";
const FORMAT_VERSION: u32 = 1;
const INITIAL_SLOT_COUNT: u64 = 1024;
const LOAD_FACTOR_NUM: u64 = 85;
const LOAD_FACTOR_DEN: u64 = 100;

// ── Bucket layout (64 bytes) ────────────────────────────────────────────────
// [0..32]   txid (Hash256)
// [32..36]  span.base (u32 le)
// [36..38]  span.count (u16 le)
// [38..42]  height (u32 le)
// [42..50]  live bitmap (u64 le)
// [50]      coinbase (u8, 0 or 1)
// [51]      probe_distance (u8, 0 = empty)
// [52..64]  padding (zeroed)

#[derive(Clone)]
struct Bucket {
    txid: [u8; 32],
    span_base: u32,
    span_count: u16,
    height: u32,
    live: u64,
    coinbase: bool,
    probe_distance: u8,
}

impl Bucket {
    #[inline]
    fn is_empty(&self) -> bool {
        self.probe_distance == 0
    }

    #[inline]
    fn matches(&self, txid: &Hash256) -> bool {
        self.txid == *txid.as_bytes()
    }

    fn to_handle(&self) -> DiskTxHandle {
        DiskTxHandle {
            span: DiskSpan {
                base: self.span_base,
                count: self.span_count,
            },
            height: BlockHeight::new(self.height),
            live: self.live,
            coinbase: self.coinbase,
        }
    }

    fn from_handle(txid: &Hash256, handle: &DiskTxHandle, probe_distance: u8) -> Self {
        Self {
            txid: *txid.as_bytes(),
            span_base: handle.span.base,
            span_count: handle.span.count,
            height: handle.height.to_u32(),
            live: handle.live,
            coinbase: handle.coinbase,
            probe_distance,
        }
    }

    fn encode(&self, buf: &mut [u8; BUCKET_SIZE as usize]) {
        buf[0..32].copy_from_slice(&self.txid);
        buf[32..36].copy_from_slice(&self.span_base.to_le_bytes());
        buf[36..38].copy_from_slice(&self.span_count.to_le_bytes());
        buf[38..42].copy_from_slice(&self.height.to_le_bytes());
        buf[42..50].copy_from_slice(&self.live.to_le_bytes());
        buf[50] = u8::from(self.coinbase);
        buf[51] = self.probe_distance;
        buf[52..64].fill(0);
    }

    fn decode(buf: &[u8; BUCKET_SIZE as usize]) -> Self {
        let mut txid = [0u8; 32];
        txid.copy_from_slice(&buf[0..32]);
        Self {
            txid,
            span_base: u32::from_le_bytes(buf[32..36].try_into().unwrap()),
            span_count: u16::from_le_bytes(buf[36..38].try_into().unwrap()),
            height: u32::from_le_bytes(buf[38..42].try_into().unwrap()),
            live: u64::from_le_bytes(buf[42..50].try_into().unwrap()),
            coinbase: buf[50] != 0,
            probe_distance: buf[51],
        }
    }
}

// ── File header (stored in slot 0, same size as a bucket) ───────────────────

struct TableHeader {
    slot_count: u64,
    live_count: u64,
}

impl TableHeader {
    fn encode(&self, buf: &mut [u8; BUCKET_SIZE as usize]) {
        *buf = [0u8; BUCKET_SIZE as usize];
        buf[0..8].copy_from_slice(&MAGIC);
        buf[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf[12..20].copy_from_slice(&self.slot_count.to_le_bytes());
        buf[20..28].copy_from_slice(&self.live_count.to_le_bytes());
    }

    fn decode(buf: &[u8; BUCKET_SIZE as usize]) -> io::Result<Self> {
        if buf[0..8] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad tx index magic"));
        }
        let version = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported tx index version {version}"),
            ));
        }
        Ok(Self {
            slot_count: u64::from_le_bytes(buf[12..20].try_into().unwrap()),
            live_count: u64::from_le_bytes(buf[20..28].try_into().unwrap()),
        })
    }
}

// ── Hash function ───────────────────────────────────────────────────────────

#[inline]
fn hash_txid(txid: &Hash256) -> u64 {
    u64::from_le_bytes(txid.as_bytes()[..8].try_into().unwrap())
}

// ── TxIndex ─────────────────────────────────────────────────────────────────

pub struct TxIndex {
    file: File,
    path: PathBuf,
    slot_count: u64,
    mask: u64,
    live_count: u64,
}

struct InsertState {
    bucket: Bucket,
    slot: u64,
    ideal: u64,
    resolved: bool,
}

impl TxIndex {
    pub fn open(path: &Path) -> io::Result<Self> {
        if path.exists() {
            Self::open_existing(path)
        } else {
            Self::create(path, INITIAL_SLOT_COUNT)
        }
    }

    pub fn open_with_capacity(path: &Path, min_slots: u64) -> io::Result<Self> {
        if path.exists() {
            Self::open_existing(path)
        } else {
            let slots = min_slots.next_power_of_two();
            Self::create(path, slots)
        }
    }

    fn create(path: &Path, slot_count: u64) -> io::Result<Self> {
        debug_assert!(slot_count.is_power_of_two());
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let file_size = (slot_count + 1) * BUCKET_SIZE;
        file.set_len(file_size)?;

        let header = TableHeader { slot_count, live_count: 0 };
        let mut buf = [0u8; BUCKET_SIZE as usize];
        header.encode(&mut buf);
        pwrite_all(&file, &buf, 0)?;

        Ok(Self {
            file,
            path: path.to_owned(),
            slot_count,
            mask: slot_count - 1,
            live_count: 0,
        })
    }

    fn open_existing(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut buf = [0u8; BUCKET_SIZE as usize];
        pread_exact(&file, &mut buf, 0)?;
        let header = TableHeader::decode(&buf)?;
        Ok(Self {
            file,
            path: path.to_owned(),
            slot_count: header.slot_count,
            mask: header.slot_count - 1,
            live_count: header.live_count,
        })
    }

    #[inline]
    fn slot_offset(slot: u64) -> u64 {
        (slot + 1) * BUCKET_SIZE
    }

    fn read_bucket(&self, slot: u64) -> io::Result<Bucket> {
        let mut buf = [0u8; BUCKET_SIZE as usize];
        pread_exact(&self.file, &mut buf, Self::slot_offset(slot))?;
        Ok(Bucket::decode(&buf))
    }

    fn write_bucket(&self, slot: u64, bucket: &Bucket) -> io::Result<()> {
        let mut buf = [0u8; BUCKET_SIZE as usize];
        bucket.encode(&mut buf);
        pwrite_all(&self.file, &buf, Self::slot_offset(slot))
    }

    fn write_empty(&self, slot: u64) -> io::Result<()> {
        let buf = [0u8; BUCKET_SIZE as usize];
        pwrite_all(&self.file, &buf, Self::slot_offset(slot))
    }

    #[inline]
    fn ideal_slot(&self, txid: &Hash256) -> u64 {
        hash_txid(txid) & self.mask
    }

    fn needs_resize(&self) -> bool {
        self.live_count * LOAD_FACTOR_DEN > self.slot_count * LOAD_FACTOR_NUM
    }

    fn flush_header(&self) -> io::Result<()> {
        let header = TableHeader {
            slot_count: self.slot_count,
            live_count: self.live_count,
        };
        let mut buf = [0u8; BUCKET_SIZE as usize];
        header.encode(&mut buf);
        pwrite_all(&self.file, &buf, 0)
    }

    // ── Public API ──────────────────────────────────────────────────────────

    pub fn get(&self, txid: &Hash256) -> io::Result<Option<DiskTxHandle>> {
        let ideal = self.ideal_slot(txid);
        let mut dist: u8 = 1;
        let mut slot = ideal;
        loop {
            let bucket = self.read_bucket(slot)?;
            if bucket.is_empty() {
                return Ok(None);
            }
            if bucket.probe_distance < dist {
                return Ok(None);
            }
            if bucket.matches(txid) {
                return Ok(Some(bucket.to_handle()));
            }
            dist = dist.saturating_add(1);
            slot = (slot + 1) & self.mask;
            if slot == ideal {
                return Ok(None);
            }
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn insert(&mut self, txid: &Hash256, handle: &DiskTxHandle) -> io::Result<()> {
        if self.needs_resize() {
            self.resize()?;
        }

        let ideal = self.ideal_slot(txid);
        let mut current = Bucket::from_handle(txid, handle, 1);
        let mut slot = ideal;

        loop {
            let existing = self.read_bucket(slot)?;

            if existing.is_empty() {
                self.write_bucket(slot, &current)?;
                self.live_count += 1;
                return Ok(());
            }

            if existing.matches(&Hash256::from_bytes(current.txid)) {
                current.probe_distance = existing.probe_distance;
                self.write_bucket(slot, &current)?;
                return Ok(());
            }

            if existing.probe_distance < current.probe_distance {
                self.write_bucket(slot, &current)?;
                current = existing;
            }

            current.probe_distance = current.probe_distance.saturating_add(1);
            slot = (slot + 1) & self.mask;
            assert!(
                slot != ideal,
                "TxIndex::insert: table full despite load-factor check"
            );
        }
    }

    /// Update an existing entry's handle in-place.
    pub fn update(&self, txid: &Hash256, handle: &DiskTxHandle) -> io::Result<bool> {
        let ideal = self.ideal_slot(txid);
        let mut dist: u8 = 1;
        let mut slot = ideal;
        loop {
            let bucket = self.read_bucket(slot)?;
            if bucket.is_empty() {
                return Ok(false);
            }
            if bucket.probe_distance < dist {
                return Ok(false);
            }
            if bucket.matches(txid) {
                let updated = Bucket::from_handle(txid, handle, bucket.probe_distance);
                self.write_bucket(slot, &updated)?;
                return Ok(true);
            }
            dist = dist.saturating_add(1);
            slot = (slot + 1) & self.mask;
            if slot == ideal {
                return Ok(false);
            }
        }
    }

    pub fn remove(&mut self, txid: &Hash256) -> io::Result<bool> {
        let ideal = self.ideal_slot(txid);
        let mut dist: u8 = 1;
        let mut slot = ideal;

        loop {
            let bucket = self.read_bucket(slot)?;
            if bucket.is_empty() {
                return Ok(false);
            }
            if bucket.probe_distance < dist {
                return Ok(false);
            }
            if bucket.matches(txid) {
                self.backshift(slot)?;
                self.live_count -= 1;
                return Ok(true);
            }
            dist = dist.saturating_add(1);
            slot = (slot + 1) & self.mask;
            if slot == ideal {
                return Ok(false);
            }
        }
    }

    fn backshift(&self, removed_slot: u64) -> io::Result<()> {
        let mut prev = removed_slot;
        let mut slot = (removed_slot + 1) & self.mask;
        loop {
            let bucket = self.read_bucket(slot)?;
            if bucket.is_empty() || bucket.probe_distance == 1 {
                self.write_empty(prev)?;
                return Ok(());
            }
            let mut shifted = bucket;
            shifted.probe_distance -= 1;
            self.write_bucket(prev, &shifted)?;
            prev = slot;
            slot = (slot + 1) & self.mask;
        }
    }

    fn resize(&mut self) -> io::Result<()> {
        let new_count = self.slot_count * 2;
        let new_path = self.path.with_extension("new");
        {
            let mut new_table = Self::create(&new_path, new_count)?;
            for slot in 0..self.slot_count {
                let bucket = self.read_bucket(slot)?;
                if !bucket.is_empty() {
                    let txid = Hash256::from_bytes(bucket.txid);
                    let handle = bucket.to_handle();
                    new_table.insert(&txid, &handle)?;
                }
            }
            new_table.flush_header()?;
        }
        std::fs::rename(&new_path, &self.path)?;
        self.file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        self.slot_count = new_count;
        self.mask = new_count - 1;
        Ok(())
    }

    pub fn sync(&self) -> io::Result<()> {
        self.flush_header()?;
        self.file.sync_all()
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.live_count
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    #[must_use]
    pub fn slot_count(&self) -> u64 {
        self.slot_count
    }

    // ── io_uring batched operations ─────────────────────────────────────────

    #[allow(clippy::cast_sign_loss)]
    fn uring_fd(&self) -> ququmatz::types::RawFd {
        ququmatz::types::RawFd::from_raw(self.file.as_raw_fd() as usize)
    }

    /// Batch-get multiple txids via `io_uring` wave probing.
    /// Returns a Vec of `Option<DiskTxHandle>` in the same order as the input.
    #[allow(clippy::missing_panics_doc)]
    pub fn get_batch_uring(
        &self,
        txids: &[Hash256],
        ring: &mut ququmatz::IoUring,
        arena: &mut Vec<u8>,
    ) -> io::Result<Vec<Option<DiskTxHandle>>> {
        let n = txids.len();
        if n == 0 {
            return Ok(Vec::new());
        }

        let fd = self.uring_fd();
        let bs = BUCKET_SIZE as usize;
        let needed = n * bs;
        arena.clear();
        arena.resize(needed, 0u8);

        let mut slots: Vec<u64> = txids.iter().map(|t| self.ideal_slot(t)).collect();
        let mut dists: Vec<u8> = vec![1; n];
        let mut resolved: Vec<bool> = vec![false; n];
        let mut results: Vec<Option<DiskTxHandle>> = vec![None; n];

        loop {
            let mut submitted = 0u32;
            for i in 0..n {
                if resolved[i] {
                    continue;
                }
                let offset = Self::slot_offset(slots[i]);
                let buf_start = i * bs;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        bs as u32,
                        offset,
                    )
                }.user_data(i as u64);
                flush_if_full(ring, sqe, &mut submitted)?;
            }

            if submitted == 0 {
                break;
            }

            ring.submit_and_wait(submitted)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;

            while let Some(cqe) = ring.complete() {
                let i = cqe.user_data as usize;
                if i >= n { continue; }
                if cqe.result < 0 {
                    return Err(io::Error::from_raw_os_error(-cqe.result));
                }

                let buf_start = i * bs;
                let buf: &[u8; 64] = arena[buf_start..buf_start + bs].try_into().unwrap();
                let bucket = Bucket::decode(buf);

                if bucket.is_empty() || bucket.probe_distance < dists[i] {
                    resolved[i] = true;
                    continue;
                }
                if bucket.matches(&txids[i]) {
                    results[i] = Some(bucket.to_handle());
                    resolved[i] = true;
                    continue;
                }
                dists[i] = dists[i].saturating_add(1);
                slots[i] = (slots[i] + 1) & self.mask;
            }
        }

        Ok(results)
    }

    /// Batch-insert multiple (txid, handle) pairs via `io_uring` wave probing.
    #[allow(clippy::missing_panics_doc, clippy::too_many_lines)]
    pub fn insert_batch_uring(
        &mut self,
        pairs: &[(Hash256, DiskTxHandle)],
        ring: &mut ququmatz::IoUring,
        arena: &mut Vec<u8>,
        write_arena: &mut Vec<u8>,
    ) -> io::Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }

        let incoming = pairs.len() as u64;
        if (self.live_count + incoming) * LOAD_FACTOR_DEN > self.slot_count * LOAD_FACTOR_NUM {
            self.resize()?;
        }

        let fd = self.uring_fd();
        let bs = BUCKET_SIZE as usize;
        let n = pairs.len();
        let needed = n * bs;

        arena.clear();
        arena.resize(needed, 0u8);
        write_arena.clear();
        write_arena.resize(needed, 0u8);

        let mut states: Vec<InsertState> = pairs.iter().map(|(txid, handle)| {
            let ideal = self.ideal_slot(txid);
            InsertState {
                bucket: Bucket::from_handle(txid, handle, 1),
                slot: ideal,
                ideal,
                resolved: false,
            }
        }).collect();

        loop {
            let mut submitted = 0u32;
            for (i, state) in states.iter().enumerate() {
                if state.resolved { continue; }
                let offset = Self::slot_offset(state.slot);
                let buf_start = i * bs;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        bs as u32,
                        offset,
                    )
                }.user_data(i as u64);
                flush_if_full(ring, sqe, &mut submitted)?;
            }

            if submitted == 0 { break; }

            ring.submit_and_wait(submitted)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;

            let mut writes: Vec<u64> = Vec::new();
            let mut write_cursor: usize = 0;

            while let Some(cqe) = ring.complete() {
                let i = cqe.user_data as usize;
                if i >= n { continue; }
                if cqe.result < 0 {
                    return Err(io::Error::from_raw_os_error(-cqe.result));
                }

                let buf_start = i * bs;
                let buf: &[u8; 64] = arena[buf_start..buf_start + bs].try_into().unwrap();
                let existing = Bucket::decode(buf);
                let state = &mut states[i];

                let write_to_slot = if existing.is_empty() {
                    self.live_count += 1;
                    state.resolved = true;
                    Some(state.slot)
                } else if existing.matches(&Hash256::from_bytes(state.bucket.txid)) {
                    state.bucket.probe_distance = existing.probe_distance;
                    state.resolved = true;
                    Some(state.slot)
                } else if existing.probe_distance < state.bucket.probe_distance {
                    let slot = state.slot;
                    let to_write = std::mem::replace(&mut state.bucket, existing);
                    state.bucket.probe_distance = state.bucket.probe_distance.saturating_add(1);
                    state.slot = (state.slot + 1) & self.mask;
                    state.ideal = state.slot;
                    let wb = write_cursor * bs;
                    if wb + bs > write_arena.len() {
                        write_arena.resize(wb + bs, 0);
                    }
                    to_write.encode((&mut write_arena[wb..wb + bs]).try_into().unwrap());
                    writes.push(slot);
                    write_cursor += 1;
                    None
                } else {
                    state.bucket.probe_distance = state.bucket.probe_distance.saturating_add(1);
                    state.slot = (state.slot + 1) & self.mask;
                    assert!(
                        state.slot != state.ideal,
                        "TxIndex::insert_batch_uring: table full despite load-factor check"
                    );
                    None
                };

                if let Some(slot) = write_to_slot {
                    let wb = write_cursor * bs;
                    if wb + bs > write_arena.len() {
                        write_arena.resize(wb + bs, 0);
                    }
                    state.bucket.encode((&mut write_arena[wb..wb + bs]).try_into().unwrap());
                    writes.push(slot);
                    write_cursor += 1;
                }
            }

            // Submit writes
            let mut write_count = 0u32;
            for (w_idx, slot) in writes.iter().enumerate() {
                let buf_start = w_idx * bs;
                let offset = Self::slot_offset(*slot);
                let sqe = unsafe {
                    ququmatz::Sqe::write_ptr(
                        fd,
                        write_arena[buf_start..].as_ptr(),
                        bs as u32,
                        offset,
                    )
                }.user_data(u64::MAX);
                flush_if_full(ring, sqe, &mut write_count)?;
            }

            if write_count > 0 {
                ring.submit_and_wait(write_count)
                    .map_err(|e| io::Error::other(format!("{e:?}")))?;
                while ring.complete().is_some() {}
            }
        }

        Ok(())
    }

    /// Batch-remove multiple txids via `io_uring`.
    #[allow(clippy::missing_panics_doc)]
    pub fn remove_batch_uring(
        &mut self,
        txids: &[Hash256],
        ring: &mut ququmatz::IoUring,
        arena: &mut Vec<u8>,
    ) -> io::Result<()> {
        if txids.is_empty() {
            return Ok(());
        }

        let n = txids.len();
        let fd = self.uring_fd();
        let bs = BUCKET_SIZE as usize;
        let needed = n * bs;
        arena.clear();
        arena.resize(needed, 0u8);

        let mut slots: Vec<u64> = txids.iter().map(|t| self.ideal_slot(t)).collect();
        let mut dists: Vec<u8> = vec![1; n];
        let mut resolved: Vec<bool> = vec![false; n];
        let mut found_slots: Vec<Option<u64>> = vec![None; n];

        // Phase 1: locate each entry
        loop {
            let mut submitted = 0u32;
            for i in 0..n {
                if resolved[i] { continue; }
                let offset = Self::slot_offset(slots[i]);
                let buf_start = i * bs;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        bs as u32,
                        offset,
                    )
                }.user_data(i as u64);
                flush_if_full(ring, sqe, &mut submitted)?;
            }

            if submitted == 0 { break; }

            ring.submit_and_wait(submitted)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;

            while let Some(cqe) = ring.complete() {
                let i = cqe.user_data as usize;
                if i >= n { continue; }
                if cqe.result < 0 {
                    return Err(io::Error::from_raw_os_error(-cqe.result));
                }

                let buf_start = i * bs;
                let buf: &[u8; 64] = arena[buf_start..buf_start + bs].try_into().unwrap();
                let bucket = Bucket::decode(buf);

                if bucket.is_empty() || bucket.probe_distance < dists[i] {
                    resolved[i] = true;
                    continue;
                }
                if bucket.matches(&txids[i]) {
                    found_slots[i] = Some(slots[i]);
                    resolved[i] = true;
                    continue;
                }
                dists[i] = dists[i].saturating_add(1);
                slots[i] = (slots[i] + 1) & self.mask;
            }
        }

        // Phase 2: backshift each found entry (synchronous per-entry for correctness)
        for found_slot in found_slots.into_iter().flatten() {
            self.backshift(found_slot)?;
            self.live_count -= 1;
        }

        Ok(())
    }

    /// Iterate over all live entries, calling `f` for each.
    pub fn for_each<F>(&self, mut f: F) -> io::Result<()>
    where
        F: FnMut(Hash256, DiskTxHandle),
    {
        for slot in 0..self.slot_count {
            let bucket = self.read_bucket(slot)?;
            if !bucket.is_empty() {
                f(Hash256::from_bytes(bucket.txid), bucket.to_handle());
            }
        }
        Ok(())
    }
}

// ── I/O helpers ─────────────────────────────────────────────────────────────

fn flush_if_full(
    ring: &mut ququmatz::IoUring,
    sqe: ququmatz::Sqe,
    submitted: &mut u32,
) -> io::Result<()> {
    let copy = sqe;
    match ring.push(sqe) {
        Ok(()) => {
            *submitted += 1;
            Ok(())
        }
        Err(ququmatz::Error::Submit(ququmatz::SubmitError::QueueFull)) => {
            ring.submit_and_wait(*submitted)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;
            while ring.complete().is_some() {}
            *submitted = 0;
            ring.push(copy).map_err(|e| io::Error::other(format!("{e:?}")))?;
            *submitted += 1;
            Ok(())
        }
        Err(e) => Err(io::Error::other(format!("{e:?}"))),
    }
}

fn pwrite_all(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

fn pread_exact(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ns = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
            let path = std::env::temp_dir().join(format!("mantle-tx-index-{}-{}", std::process::id(), ns));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    fn fake_txid(n: u8) -> Hash256 {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        bytes[31] = n.wrapping_mul(7);
        Hash256::from_bytes(bytes)
    }

    fn fake_handle(base: u32, count: u16, height: u32) -> DiskTxHandle {
        DiskTxHandle {
            span: DiskSpan { base, count },
            height: BlockHeight::new(height),
            live: u64::MAX >> (64 - count as u64),
            coinbase: false,
        }
    }

    #[test]
    fn insert_get_remove() {
        let dir = TmpDir::new();
        let mut index = TxIndex::open(&dir.path().join("tx_index.bin")).unwrap();

        let txid = fake_txid(1);
        let handle = fake_handle(0, 3, 100);

        assert!(index.get(&txid).unwrap().is_none());

        index.insert(&txid, &handle).unwrap();
        assert_eq!(index.len(), 1);

        let got = index.get(&txid).unwrap().unwrap();
        assert_eq!(got.span.base, 0);
        assert_eq!(got.span.count, 3);
        assert_eq!(got.height, BlockHeight::new(100));

        assert!(index.remove(&txid).unwrap());
        assert_eq!(index.len(), 0);
        assert!(index.get(&txid).unwrap().is_none());
    }

    #[test]
    fn update_in_place() {
        let dir = TmpDir::new();
        let mut index = TxIndex::open(&dir.path().join("tx_index.bin")).unwrap();

        let txid = fake_txid(42);
        let handle = fake_handle(10, 5, 200);
        index.insert(&txid, &handle).unwrap();

        let mut updated = handle;
        updated.live = 0b1010;
        assert!(index.update(&txid, &updated).unwrap());

        let got = index.get(&txid).unwrap().unwrap();
        assert_eq!(got.live, 0b1010);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn many_inserts_trigger_resize() {
        let dir = TmpDir::new();
        let mut index = TxIndex::open(&dir.path().join("tx_index.bin")).unwrap();
        assert_eq!(index.slot_count(), 1024);

        for i in 0u16..900 {
            let txid = fake_txid_u16(i);
            let handle = fake_handle(u32::from(i), 2, u32::from(i) + 100);
            index.insert(&txid, &handle).unwrap();
        }

        assert!(index.slot_count() > 1024);
        assert_eq!(index.len(), 900);

        for i in 0u16..900 {
            let txid = fake_txid_u16(i);
            let got = index.get(&txid).unwrap().unwrap();
            assert_eq!(got.span.base, u32::from(i));
        }
    }

    #[test]
    fn reopen_preserves_data() {
        let dir = TmpDir::new();
        let path = dir.path().join("tx_index.bin");
        {
            let mut index = TxIndex::open(&path).unwrap();
            let txid = fake_txid(99);
            let handle = fake_handle(7, 4, 500);
            index.insert(&txid, &handle).unwrap();
            index.sync().unwrap();
        }
        let index = TxIndex::open(&path).unwrap();
        assert_eq!(index.len(), 1);
        let got = index.get(&fake_txid(99)).unwrap().unwrap();
        assert_eq!(got.span.base, 7);
        assert_eq!(got.height, BlockHeight::new(500));
    }

    #[test]
    fn for_each_iterates_all() {
        let dir = TmpDir::new();
        let mut index = TxIndex::open(&dir.path().join("tx_index.bin")).unwrap();

        for i in 0u8..10 {
            index.insert(&fake_txid(i), &fake_handle(u32::from(i), 1, 0)).unwrap();
        }

        let mut count = 0u32;
        index.for_each(|_, _| count += 1).unwrap();
        assert_eq!(count, 10);
    }

    fn fake_txid_u16(n: u16) -> Hash256 {
        let mut bytes = [0u8; 32];
        bytes[0..2].copy_from_slice(&n.to_le_bytes());
        bytes[30..32].copy_from_slice(&n.wrapping_mul(13).to_le_bytes());
        Hash256::from_bytes(bytes)
    }
}
