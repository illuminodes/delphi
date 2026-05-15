//! On-disk Robin Hood hash table for the UTXO set.
//!
//! Fixed 88-byte buckets stored in `utxo_table.bin`. Power-of-two slot count.
//! Standard scriptPubKeys (≤34 bytes) stored inline; nonstandard overflow to
//! a sidecar file. Phase 1 uses synchronous `pread`/`pwrite`; Phase 2 replaces
//! with `io_uring` batched I/O.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::hash::ByteSliceExt;
use mantle::primitives::{
    BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, ScriptPubKey,
};
use mantle_consensus::chain_view::{Coin, TipInfo};

use super::{Engine, EngineError};
use crate::types::{CommitBlockReq, Durability, LookupResult};

const BUCKET_SIZE: usize = 88;
const MAGIC: [u8; 8] = *b"MANTLEU\0";
const FORMAT_VERSION: u32 = 2;
const INITIAL_SLOT_COUNT: u64 = 1024;
const MAX_INLINE_SPK: usize = 34;
const OVERFLOW_MARKER: u16 = 0xFFFF;
const LOAD_FACTOR_NUM: u64 = 85;
const LOAD_FACTOR_DEN: u64 = 100;

// ── Bucket layout ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Bucket {
    probe_distance: u16,
    coinbase: bool,
    spk_len: u16,
    height: u32,
    value: u64,
    txid: [u8; 32],
    vout: u32,
    script_pubkey: [u8; MAX_INLINE_SPK],
}

impl Bucket {
    fn is_empty(&self) -> bool {
        self.probe_distance == 0
    }

    fn outpoint(&self) -> OutPoint {
        OutPoint {
            txid: Hash256::from_bytes(self.txid),
            vout: self.vout,
        }
    }

    fn matches(&self, op: &OutPoint) -> bool {
        self.txid == *op.txid.as_bytes() && self.vout == op.vout
    }

    fn to_coin(&self, overflow: &File) -> io::Result<Coin> {
        let script_pubkey = if self.spk_len == OVERFLOW_MARKER {
            let offset = u64::from_le_bytes(self.script_pubkey[..8].try_into().unwrap());
            let spk = read_overflow(overflow, offset)?;
            ScriptPubKey::from_slice(&spk)
        } else {
            ScriptPubKey::from_slice(&self.script_pubkey[..self.spk_len as usize])
        };
        Ok(Coin {
            value: Satoshi::new(self.value),
            script_pubkey,
            height: BlockHeight::new(self.height),
            coinbase: self.coinbase,
        })
    }

    fn from_outpoint_and_coin(
        op: &OutPoint,
        coin: &Coin,
        probe_distance: u16,
        overflow: &mut File,
    ) -> io::Result<Self> {
        let spk = coin.script_pubkey.as_slice();
        let spk_len = spk.len();
        let mut script_pubkey = [0u8; MAX_INLINE_SPK];
        if spk_len <= MAX_INLINE_SPK {
            script_pubkey[..spk_len].copy_from_slice(spk);
        } else {
            let offset = append_overflow(overflow, spk)?;
            script_pubkey[..8].copy_from_slice(&offset.to_le_bytes());
        }
        Ok(Self {
            probe_distance,
            coinbase: coin.coinbase,
            spk_len: if spk_len <= MAX_INLINE_SPK {
                u16::try_from(spk_len).expect("spk_len <= MAX_INLINE_SPK <= u16::MAX")
            } else {
                OVERFLOW_MARKER
            },
            height: coin.height.to_u32(),
            value: coin.value.to_u64(),
            txid: *op.txid.as_bytes(),
            vout: op.vout,
            script_pubkey,
        })
    }

    fn encode(&self, buf: &mut [u8; BUCKET_SIZE]) {
        // Layout (88 bytes total):
        //  [0]      probe_distance low byte
        //  [1]      coinbase
        //  [2..4]   spk_len (u16 le)
        //  [4..8]   height (u32 le)
        //  [8..16]  value (u64 le)
        //  [16..48] txid
        //  [48..52] vout (u32 le)
        //  [52..86] script_pubkey (34 bytes inline)
        //  [86]     probe_distance high byte
        //  [87]     padding
        let pd = self.probe_distance.to_le_bytes();
        buf[0] = pd[0];
        buf[1] = u8::from(self.coinbase);
        buf[2..4].copy_from_slice(&self.spk_len.to_le_bytes());
        buf[4..8].copy_from_slice(&self.height.to_le_bytes());
        buf[8..16].copy_from_slice(&self.value.to_le_bytes());
        buf[16..48].copy_from_slice(&self.txid);
        buf[48..52].copy_from_slice(&self.vout.to_le_bytes());
        buf[52..86].copy_from_slice(&self.script_pubkey);
        buf[86] = pd[1];
        buf[87] = 0;
    }

    fn decode(buf: &[u8; BUCKET_SIZE]) -> Self {
        let mut txid = [0u8; 32];
        txid.copy_from_slice(&buf[16..48]);
        let mut script_pubkey = [0u8; MAX_INLINE_SPK];
        script_pubkey.copy_from_slice(&buf[52..86]);
        Self {
            probe_distance: u16::from_le_bytes([buf[0], buf[86]]),
            coinbase: buf[1] != 0,
            spk_len: u16::from_le_bytes([buf[2], buf[3]]),
            height: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            value: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            txid,
            vout: u32::from_le_bytes([buf[48], buf[49], buf[50], buf[51]]),
            script_pubkey,
        }
    }
}

// ── Insert state (used by insert_batch_uring) ────────────────────────────────

pub struct InsertState {
    bucket: Bucket,
    slot: u64,
    ideal: u64,
    resolved: bool,
}

// ── Hash function ────────────────────────────────────────────────────────────

fn hash_outpoint(op: &OutPoint) -> u64 {
    // OutPoint.txid is SHA-256d output — first 8 bytes are uniformly distributed.
    u64::from_le_bytes(op.txid.as_bytes()[..8].try_into().unwrap())
}

// ── File header ──────────────────────────────────────────────────────────────

struct TableHeader {
    slot_count: u64,
    live_count: u64,
}

impl TableHeader {
    fn encode(&self, buf: &mut [u8; BUCKET_SIZE]) {
        *buf = [0u8; BUCKET_SIZE];
        buf[0..8].copy_from_slice(&MAGIC);
        buf[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf[12..20].copy_from_slice(&self.slot_count.to_le_bytes());
        buf[20..28].copy_from_slice(&self.live_count.to_le_bytes());
    }

    fn decode(buf: &[u8; BUCKET_SIZE]) -> io::Result<Self> {
        if buf[0..8] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad UTXO table magic",
            ));
        }
        let version = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported UTXO table version {version}"),
            ));
        }
        Ok(Self {
            slot_count: u64::from_le_bytes(buf[12..20].try_into().unwrap()),
            live_count: u64::from_le_bytes(buf[20..28].try_into().unwrap()),
        })
    }
}

// ── RobinHoodTable ───────────────────────────────────────────────────────────

pub struct RobinHoodTable {
    file: File,
    path: PathBuf,
    overflow: File,
    slot_count: u64,
    mask: u64,
    live_count: u64,
}

impl RobinHoodTable {
    /// # Errors
    /// Returns an error if the file cannot be created or opened.
    pub fn open(path: &Path) -> io::Result<Self> {
        if path.exists() {
            Self::open_existing(path)
        } else {
            Self::create(path, INITIAL_SLOT_COUNT)
        }
    }

    /// # Errors
    /// Returns an error if the file cannot be created or opened.
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

        let file_size = (slot_count + 1) * BUCKET_SIZE as u64;
        file.set_len(file_size)?;

        let header = TableHeader {
            slot_count,
            live_count: 0,
        };
        let mut buf = [0u8; BUCKET_SIZE];
        header.encode(&mut buf);
        pwrite_all(&file, &buf, 0)?;

        let overflow = open_overflow_file(&overflow_path(path))?;

        Ok(Self {
            file,
            path: path.to_owned(),
            overflow,
            slot_count,
            mask: slot_count - 1,
            live_count: 0,
        })
    }

    fn open_existing(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut buf = [0u8; BUCKET_SIZE];
        pread_exact(&file, &mut buf, 0)?;
        let header = TableHeader::decode(&buf)?;
        let overflow = open_overflow_file(&overflow_path(path))?;
        Ok(Self {
            file,
            path: path.to_owned(),
            overflow,
            slot_count: header.slot_count,
            mask: header.slot_count - 1,
            live_count: header.live_count,
        })
    }

    fn slot_offset(slot: u64) -> u64 {
        (slot + 1) * BUCKET_SIZE as u64
    }

    fn read_bucket(&self, slot: u64) -> io::Result<Bucket> {
        let mut buf = [0u8; BUCKET_SIZE];
        pread_exact(&self.file, &mut buf, Self::slot_offset(slot))?;
        Ok(Bucket::decode(&buf))
    }

    fn write_bucket(&self, slot: u64, bucket: &Bucket) -> io::Result<()> {
        let mut buf = [0u8; BUCKET_SIZE];
        bucket.encode(&mut buf);
        pwrite_all(&self.file, &buf, Self::slot_offset(slot))
    }

    fn write_empty(&self, slot: u64) -> io::Result<()> {
        let buf = [0u8; BUCKET_SIZE];
        pwrite_all(&self.file, &buf, Self::slot_offset(slot))
    }

    fn ideal_slot(&self, op: &OutPoint) -> u64 {
        hash_outpoint(op) & self.mask
    }

    fn needs_resize(&self) -> bool {
        self.live_count * LOAD_FACTOR_DEN > self.slot_count * LOAD_FACTOR_NUM
    }

    fn flush_header(&self) -> io::Result<()> {
        let header = TableHeader {
            slot_count: self.slot_count,
            live_count: self.live_count,
        };
        let mut buf = [0u8; BUCKET_SIZE];
        header.encode(&mut buf);
        pwrite_all(&self.file, &buf, 0)
    }

    #[cfg(unix)]
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn fd(&self) -> ququmatz::types::RawFd {
        use std::os::unix::io::AsRawFd;
        ququmatz::types::RawFd::from_raw(self.file.as_raw_fd() as usize)
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// # Errors
    /// Returns an error on I/O failure.
    pub fn get(&self, op: &OutPoint) -> io::Result<Option<Coin>> {
        let ideal = self.ideal_slot(op);
        let mut dist: u16 = 1;
        let mut slot = ideal;
        loop {
            let bucket = self.read_bucket(slot)?;
            if bucket.is_empty() {
                return Ok(None);
            }
            if bucket.probe_distance < dist {
                return Ok(None);
            }
            if bucket.matches(op) {
                return Ok(Some(bucket.to_coin(&self.overflow)?));
            }
            dist += 1;
            slot = (slot + 1) & self.mask;
            if slot == ideal {
                return Ok(None);
            }
        }
    }

    /// # Errors
    /// Returns an error on I/O failure.
    ///
    /// # Panics
    /// Panics if the table is full despite the load-factor resize check, which
    /// indicates a bug in the resize logic.
    pub fn insert(&mut self, op: &OutPoint, coin: &Coin) -> io::Result<()> {
        if self.needs_resize() {
            self.resize()?;
        }

        let ideal = self.ideal_slot(op);
        let mut current = Bucket::from_outpoint_and_coin(op, coin, 1, &mut self.overflow)?;
        let mut slot = ideal;

        loop {
            let existing = self.read_bucket(slot)?;

            if existing.is_empty() {
                self.write_bucket(slot, &current)?;
                self.live_count += 1;
                return Ok(());
            }

            if existing.matches(&current.outpoint()) {
                current.probe_distance = existing.probe_distance;
                self.write_bucket(slot, &current)?;
                return Ok(());
            }

            if existing.probe_distance < current.probe_distance {
                self.write_bucket(slot, &current)?;
                current = existing;
            }

            current.probe_distance += 1;
            slot = (slot + 1) & self.mask;
            assert!(
                slot != ideal,
                "RobinHoodTable::insert: table full despite load-factor check"
            );
        }
    }

    /// # Errors
    /// Returns an error on I/O failure.
    pub fn remove(&mut self, op: &OutPoint) -> io::Result<bool> {
        let ideal = self.ideal_slot(op);
        let mut dist: u16 = 1;
        let mut slot = ideal;

        loop {
            let bucket = self.read_bucket(slot)?;
            if bucket.is_empty() {
                return Ok(false);
            }
            if bucket.probe_distance < dist {
                return Ok(false);
            }
            if bucket.matches(op) {
                self.backshift(slot)?;
                self.live_count -= 1;
                return Ok(true);
            }
            dist += 1;
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
                    let op = bucket.outpoint();
                    let coin = bucket.to_coin(&self.overflow)?;
                    new_table.insert(&op, &coin)?;
                }
            }
            new_table.flush_header()?;
        }
        std::fs::rename(&new_path, &self.path)?;
        let new_overflow_path = overflow_path(&new_path);
        let old_overflow_path = overflow_path(&self.path);
        std::fs::rename(&new_overflow_path, &old_overflow_path)?;
        self.file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        self.overflow = open_overflow_file(&old_overflow_path)?;
        self.slot_count = new_count;
        self.mask = new_count - 1;
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.live_count
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    /// # Errors
    /// Returns an error on I/O failure.
    pub fn sync(&self) -> io::Result<()> {
        self.flush_header()?;
        self.overflow.sync_all()?;
        self.file.sync_all()
    }

    /// Batched insert using `io_uring`.
    ///
    /// Inserts all `(OutPoint, Coin)` pairs via wave-based Robin Hood probing.
    /// Each wave submits one `pread` per pending entry (its current probe slot),
    /// waits for all completions, then either writes the entry into an empty slot,
    /// displaces a weaker incumbent (Robin Hood steal), or advances to the next
    /// probe slot.  Typically completes in 2–4 waves for realistic load factors.
    ///
    /// Scratch vecs (`arena`, `states`) are passed in by the caller so they can
    /// be reused across calls without re-allocating.
    ///
    /// # Errors
    /// Returns an error on I/O failure.
    ///
    /// # Panics
    /// Panics if the table fills despite the load-factor check (bug in resize
    /// logic) or if the `live_count` would overflow.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub fn insert_batch_uring(
        &mut self,
        pairs: &[(OutPoint, Coin)],
        ring: &mut ququmatz::IoUring,
        arena: &mut Vec<u8>,
        write_arena: &mut Vec<u8>,
        states: &mut Vec<InsertState>,
    ) -> io::Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }

        if (self.live_count + u64::try_from(pairs.len()).expect("batch fits u64")) * LOAD_FACTOR_DEN
            > self.slot_count * LOAD_FACTOR_NUM
        {
            self.resize()?;
        }

        let fd = self.fd();

        states.clear();
        for (op, coin) in pairs {
            let ideal = self.ideal_slot(op);
            states.push(InsertState {
                bucket: Bucket::from_outpoint_and_coin(op, coin, 1, &mut self.overflow)?,
                slot: ideal,
                ideal,
                resolved: false,
            });
        }

        let n = states.len();
        let needed = n * BUCKET_SIZE;
        arena.clear();
        arena.resize(needed, 0u8);
        write_arena.clear();
        write_arena.resize(needed, 0u8);

        loop {
            let mut submitted = 0u32;
            for (i, state) in states.iter().enumerate() {
                if state.resolved {
                    continue;
                }
                let offset = Self::slot_offset(state.slot);
                let buf_start = i * BUCKET_SIZE;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        u32::try_from(BUCKET_SIZE).expect("BUCKET_SIZE fits u32"),
                        offset,
                    )
                }
                .user_data(i as u64);
                flush_if_full(ring, sqe, &mut submitted)?;
            }

            if submitted == 0 {
                break;
            }

            ring.submit_and_wait(submitted).map_err(uring_err)?;

            let mut writes: Vec<u64> = Vec::new();
            let mut write_cursor: usize = 0;

            while let Some(cqe) = ring.complete() {
                let i = usize::try_from(cqe.user_data).expect("user_data index fits usize");
                if i >= n {
                    continue;
                }
                let res = cqe.result;
                if res < 0 {
                    return Err(io::Error::from_raw_os_error(-res));
                }

                let buf_start = i * BUCKET_SIZE;
                let buf: &[u8; BUCKET_SIZE] = arena[buf_start..buf_start + BUCKET_SIZE]
                    .try_into()
                    .unwrap();
                let existing = Bucket::decode(buf);

                let state = &mut states[i];

                let write_to_slot = if existing.is_empty() {
                    self.live_count += 1;
                    state.resolved = true;
                    Some(state.slot)
                } else if existing.matches(&state.bucket.outpoint()) {
                    state.bucket.probe_distance = existing.probe_distance;
                    state.resolved = true;
                    Some(state.slot)
                } else if existing.probe_distance < state.bucket.probe_distance {
                    let slot = state.slot;
                    let to_write = std::mem::replace(&mut state.bucket, existing);
                    state.bucket.probe_distance += 1;
                    state.slot = (state.slot + 1) & self.mask;
                    state.ideal = state.slot;
                    // Encode the entry we're placing (not the displaced one).
                    let wb = write_cursor * BUCKET_SIZE;
                    if wb + BUCKET_SIZE > write_arena.len() {
                        write_arena.resize(wb + BUCKET_SIZE, 0);
                    }
                    to_write.encode((&mut write_arena[wb..wb + BUCKET_SIZE]).try_into().unwrap());
                    writes.push(slot);
                    write_cursor += 1;
                    None // already handled
                } else {
                    state.bucket.probe_distance += 1;
                    state.slot = (state.slot + 1) & self.mask;
                    assert!(
                        state.slot != state.ideal,
                        "insert_batch_uring: table full despite load-factor check"
                    );
                    None
                };

                if let Some(slot) = write_to_slot {
                    let wb = write_cursor * BUCKET_SIZE;
                    if wb + BUCKET_SIZE > write_arena.len() {
                        write_arena.resize(wb + BUCKET_SIZE, 0);
                    }
                    state
                        .bucket
                        .encode((&mut write_arena[wb..wb + BUCKET_SIZE]).try_into().unwrap());
                    writes.push(slot);
                    write_cursor += 1;
                }
            }

            let mut write_count = 0u32;
            for (w_idx, slot) in writes.iter().enumerate() {
                let buf_start = w_idx * BUCKET_SIZE;
                let offset = Self::slot_offset(*slot);
                let sqe = unsafe {
                    ququmatz::Sqe::write_ptr(
                        fd,
                        write_arena[buf_start..].as_ptr(),
                        u32::try_from(BUCKET_SIZE).expect("BUCKET_SIZE fits u32"),
                        offset,
                    )
                }
                .user_data(u64::MAX);
                flush_if_full(ring, sqe, &mut write_count)?;
            }

            if write_count > 0 {
                ring.submit_and_wait(write_count).map_err(uring_err)?;
                while ring.complete().is_some() {}
            }
        }

        Ok(())
    }

    /// Batched delete using `io_uring`.
    ///
    /// Phase 1: batch-read to locate each target slot across all ops (same
    /// round-loop as `get_batch_uring`).  Phase 2: for each found target, run
    /// the Robin Hood backshift.  Backshift chains across all ops are
    /// interleaved in the ring — one read per active chain per round — so all
    /// N chains make progress in parallel.
    ///
    /// Scratch vecs are passed in by the caller for reuse.
    ///
    /// # Errors
    /// Returns an error on I/O failure.
    ///
    /// # Panics
    /// Panics only on internal invariant violations that cannot occur in
    /// correct usage (infallible `u32`/`usize` casts on compile-time constants).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub fn remove_batch_uring(
        &mut self,
        ops: &[OutPoint],
        ring: &mut ququmatz::IoUring,
        arena: &mut Vec<u8>,
        slots: &mut Vec<u64>,
        dists: &mut Vec<u16>,
        resolved: &mut Vec<bool>,
    ) -> io::Result<()> {
        if ops.is_empty() {
            return Ok(());
        }

        let n = ops.len();
        let fd = self.fd();

        // ── Phase 1: find each target slot (mirrors get_batch_uring) ─────────

        slots.clear();
        slots.extend(ops.iter().map(|op| self.ideal_slot(op)));
        dists.clear();
        dists.resize(n, 1);
        resolved.clear();
        resolved.resize(n, false);

        // found_slots[i] = Some(slot) once we've located the entry for ops[i].
        let mut found_slots: Vec<Option<u64>> = vec![None; n];

        let needed = n * BUCKET_SIZE;
        arena.clear();
        arena.resize(needed, 0u8);

        loop {
            let mut submitted = 0u32;
            for i in 0..n {
                if resolved[i] {
                    continue;
                }
                let offset = Self::slot_offset(slots[i]);
                let buf_start = i * BUCKET_SIZE;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        u32::try_from(BUCKET_SIZE).expect("BUCKET_SIZE fits u32"),
                        offset,
                    )
                }
                .user_data(i as u64);
                flush_if_full(ring, sqe, &mut submitted)?;
            }

            if submitted == 0 {
                break;
            }

            ring.submit_and_wait(submitted).map_err(uring_err)?;

            let mut need_reprobe = false;
            while let Some(cqe) = ring.complete() {
                let i = usize::try_from(cqe.user_data).expect("user_data index fits usize");
                if i >= n {
                    continue;
                }
                let res = cqe.result;
                if res < 0 {
                    return Err(io::Error::from_raw_os_error(-res));
                }

                let buf_start = i * BUCKET_SIZE;
                let buf: &[u8; BUCKET_SIZE] = arena[buf_start..buf_start + BUCKET_SIZE]
                    .try_into()
                    .unwrap();
                let bucket = Bucket::decode(buf);

                if bucket.is_empty() || bucket.probe_distance < dists[i] {
                    resolved[i] = true; // not found
                    continue;
                }
                if bucket.matches(&ops[i]) {
                    found_slots[i] = Some(slots[i]);
                    resolved[i] = true;
                    continue;
                }
                dists[i] += 1;
                slots[i] = (slots[i] + 1) & self.mask;
                need_reprobe = true;
            }

            if !need_reprobe {
                break;
            }
        }

        // ── Phase 2: backshift chains, interleaved across all found entries ───
        //
        // For each found entry we maintain a (prev, next) pair of slots where
        // `prev` is the slot to blank/overwrite and `next` is the slot to read
        // next.  Each round we submit one read per active chain; after
        // completions we either write (blank prev or copy shifted bucket) and
        // advance, or terminate the chain.

        // Active backshift state: (prev_slot, next_slot).
        // Index j corresponds to the j-th *found* entry (not the original op index).
        let mut shifts: Vec<(u64, u64)> = Vec::new();
        // We also need a per-shift arena slot; reuse the arena with a separate section.
        // Re-size arena to hold one bucket per active shift chain.
        let found_count = found_slots.iter().filter(|s| s.is_some()).count();
        if found_count == 0 {
            return Ok(());
        }
        // Read arena: one slot per chain for the next-slot reads.
        // Write arena: one slot per chain, stable across submit_and_wait, so
        // the kernel never sees a buffer that has been reused for a read.
        arena.clear();
        arena.resize(found_count * BUCKET_SIZE, 0u8);
        let mut write_arena: Vec<u8> = vec![0u8; found_count * BUCKET_SIZE];

        for slot in found_slots.iter().flatten() {
            let next = (*slot + 1) & self.mask;
            shifts.push((*slot, next));
        }
        self.live_count -= u64::try_from(found_count).expect("found_count fits u64");

        // shift_done[j] = true once chain j has terminated.
        let mut shift_done: Vec<bool> = vec![false; shifts.len()];

        loop {
            let mut submitted = 0u32;
            for (j, &(_, next)) in shifts.iter().enumerate() {
                if shift_done[j] {
                    continue;
                }
                let offset = Self::slot_offset(next);
                let buf_start = j * BUCKET_SIZE;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        u32::try_from(BUCKET_SIZE).expect("BUCKET_SIZE fits u32"),
                        offset,
                    )
                }
                .user_data(j as u64);
                flush_if_full(ring, sqe, &mut submitted)?;
            }

            if submitted == 0 {
                break;
            }

            ring.submit_and_wait(submitted).map_err(uring_err)?;

            // Collect write decisions; (slot_to_write, chain_index, is_blank).
            let mut write_ops: Vec<(u64, usize, bool)> = Vec::new();

            while let Some(cqe) = ring.complete() {
                let j = usize::try_from(cqe.user_data).expect("user_data index fits usize");
                if j >= shifts.len() {
                    continue;
                }
                let res = cqe.result;
                if res < 0 {
                    return Err(io::Error::from_raw_os_error(-res));
                }

                let buf_start = j * BUCKET_SIZE;
                let buf: &[u8; BUCKET_SIZE] = arena[buf_start..buf_start + BUCKET_SIZE]
                    .try_into()
                    .unwrap();
                let next_bucket = Bucket::decode(buf);
                let (prev, next) = shifts[j];

                if next_bucket.is_empty() || next_bucket.probe_distance == 1 {
                    // Terminate: blank prev slot.
                    write_ops.push((prev, j, true));
                    shift_done[j] = true;
                } else {
                    // Shift: write next_bucket (pd-1) into prev, advance chain.
                    let mut shifted = next_bucket;
                    shifted.probe_distance -= 1;
                    let wbuf_start = j * BUCKET_SIZE;
                    shifted.encode(
                        (&mut write_arena[wbuf_start..wbuf_start + BUCKET_SIZE])
                            .try_into()
                            .unwrap(),
                    );
                    write_ops.push((prev, j, false));
                    shifts[j] = (next, (next + 1) & self.mask);
                }
            }

            // Submit writes (buffers live in write_arena, stable until wait).
            let mut write_count = 0u32;
            for (slot, j, blank) in write_ops {
                let wbuf_start = j * BUCKET_SIZE;
                if blank {
                    write_arena[wbuf_start..wbuf_start + BUCKET_SIZE].fill(0);
                }
                let offset = Self::slot_offset(slot);
                let sqe = unsafe {
                    ququmatz::Sqe::write_ptr(
                        fd,
                        write_arena[wbuf_start..].as_ptr(),
                        u32::try_from(BUCKET_SIZE).expect("BUCKET_SIZE fits u32"),
                        offset,
                    )
                }
                .user_data(u64::MAX);
                flush_if_full(ring, sqe, &mut write_count)?;
            }

            if write_count > 0 {
                ring.submit_and_wait(write_count).map_err(uring_err)?;
                while ring.complete().is_some() {}
            }

            if shift_done.iter().all(|d| *d) {
                break;
            }
        }

        Ok(())
    }

    /// Batched UTXO lookup using `io_uring`.
    ///
    /// Submits one `pread` per outpoint for the ideal bucket, waits for all
    /// completions, then submits follow-up reads for Robin Hood probe chains.
    /// Typically completes in 1–2 rounds.
    ///
    /// `arena` must be pre-sized to `ops.len() * BUCKET_SIZE`. Results are
    /// written into `results` in the same order as `ops`.
    ///
    /// # Errors
    /// Returns an error on I/O failure.
    ///
    /// # Panics
    /// Panics if `arena` is shorter than `ops.len() * BUCKET_SIZE`.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_arguments)]
    pub fn get_batch_uring(
        &self,
        ops: &[OutPoint],
        results: &mut Vec<Option<Coin>>,
        ring: &mut ququmatz::IoUring,
        arena: &mut Vec<u8>,
        slots: &mut Vec<u64>,
        dists: &mut Vec<u16>,
        resolved: &mut Vec<bool>,
    ) -> io::Result<()> {
        let n = ops.len();
        if n == 0 {
            return Ok(());
        }

        results.clear();
        results.resize(n, None);

        let fd = self.fd();

        // Per-outpoint state: current probe slot and distance.
        slots.clear();
        slots.extend(ops.iter().map(|op| self.ideal_slot(op)));
        dists.clear();
        dists.resize(n, 1);
        resolved.clear();
        resolved.resize(n, false);
        // Arena for bucket reads — reused across calls, capacity grows to max block size.
        let needed = n * BUCKET_SIZE;
        arena.clear();
        arena.resize(needed, 0);

        loop {
            // Submit reads for all unresolved outpoints.
            let mut submitted = 0u32;
            for i in 0..n {
                if resolved[i] {
                    continue;
                }
                let offset = Self::slot_offset(slots[i]);
                let buf_start = i * BUCKET_SIZE;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        u32::try_from(BUCKET_SIZE).expect("BUCKET_SIZE fits u32"),
                        offset,
                    )
                }
                .user_data(i as u64);
                flush_if_full(ring, sqe, &mut submitted)?;
            }

            if submitted == 0 {
                break;
            }

            ring.submit_and_wait(submitted).map_err(uring_err)?;

            // Process completions.
            let mut need_reprobe = false;
            while let Some(cqe) = ring.complete() {
                let i = usize::try_from(cqe.user_data).expect("user_data index fits usize");
                if i >= n {
                    continue;
                }

                let res = cqe.result;
                if res < 0 {
                    return Err(io::Error::from_raw_os_error(-res));
                }

                let buf_start = i * BUCKET_SIZE;
                let buf: &[u8; BUCKET_SIZE] = arena[buf_start..buf_start + BUCKET_SIZE]
                    .try_into()
                    .unwrap();
                let bucket = Bucket::decode(buf);

                if bucket.is_empty() || bucket.probe_distance < dists[i] {
                    // Not found — empty slot or Robin Hood invariant says stop.
                    resolved[i] = true;
                    continue;
                }

                if bucket.matches(&ops[i]) {
                    // Found.
                    results[i] = Some(bucket.to_coin(&self.overflow)?);
                    resolved[i] = true;
                    continue;
                }

                // Probe miss — advance to next slot for next round.
                dists[i] += 1;
                slots[i] = (slots[i] + 1) & self.mask;
                need_reprobe = true;
            }

            if !need_reprobe {
                break;
            }
        }

        Ok(())
    }
}

// ── RobinHoodEngine — Engine trait impl ──────────────────────────────────────

const HEADER_RECORD_SIZE: usize = 116;

const STORAGE_RING_ENTRIES: u32 = 32768;

const WRITE_BUFFER_FLUSH_THRESHOLD: usize = 500_000;

// ── Write buffer ─────────────────────────────────────────────────────────────

enum BufferEntry {
    Live(Coin),
}

// SAFETY: RobinHoodEngine is only accessed from the single storage thread
// after being moved there via spawn(). The IoUring raw pointers inside are
// thread-local to that thread — no concurrent access.
unsafe impl Send for RobinHoodEngine {}

pub struct RobinHoodEngine {
    data_dir: PathBuf,
    table: RobinHoodTable,
    ring: ququmatz::IoUring,
    write_buffer: PrehashedMap<OutPoint, BufferEntry>,
    pending_disk_deletes: Vec<OutPoint>,
    read_arena: Vec<u8>,
    // Scratch buffers reused across read_coins / get_batch_uring calls.
    rc_results: Vec<Option<Coin>>,
    rc_disk_indices: Vec<usize>,
    rc_disk_outpoints: Vec<OutPoint>,
    rc_disk_coins: Vec<Option<Coin>>,
    uring_slots: Vec<u64>,
    uring_dists: Vec<u16>,
    uring_resolved: Vec<bool>,
    // Scratch buffers reused across flush_write_buffer calls.
    wb_pairs: Vec<(OutPoint, Coin)>,
    wb_del_slots: Vec<u64>,
    wb_del_dists: Vec<u16>,
    wb_del_resolved: Vec<bool>,
    header_file: File,
    header_write_len: u64,
    header_index: PrehashedMap<Hash256, u32>,
    checkpoint_file: File,
    tip: TipInfo,
}

impl RobinHoodEngine {
    #[allow(clippy::missing_errors_doc)]
    pub fn open(data_dir: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(data_dir).map_err(io_err)?;

        let table_path = data_dir.join("utxo_table.bin");
        let header_path = data_dir.join("headers.bin");
        let checkpoint_path = data_dir.join("checkpoint.bin");

        let table = RobinHoodTable::open(&table_path).map_err(io_err)?;

        let header_file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&header_path)
            .map_err(io_err)?;

        let checkpoint_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&checkpoint_path)
            .map_err(io_err)?;

        let ring = ququmatz::IoUring::new(STORAGE_RING_ENTRIES)
            .map_err(|e| EngineError::Io(uring_err(e)))?;

        let mut engine = Self {
            data_dir: data_dir.to_owned(),
            table,
            ring,
            write_buffer: PrehashedMap::with_capacity_and_hasher(
                WRITE_BUFFER_FLUSH_THRESHOLD,
                BuildPrehashedHasher,
            ),
            pending_disk_deletes: Vec::with_capacity(WRITE_BUFFER_FLUSH_THRESHOLD),
            read_arena: Vec::new(),
            rc_results: Vec::new(),
            rc_disk_indices: Vec::new(),
            rc_disk_outpoints: Vec::new(),
            rc_disk_coins: Vec::new(),
            uring_slots: Vec::new(),
            uring_dists: Vec::new(),
            uring_resolved: Vec::new(),
            wb_pairs: Vec::new(),
            wb_del_slots: Vec::new(),
            wb_del_dists: Vec::new(),
            wb_del_resolved: Vec::new(),
            header_file,
            header_write_len: 0,
            header_index: PrehashedMap::with_hasher(BuildPrehashedHasher),
            checkpoint_file,
            tip: TipInfo {
                hash: Hash256::ZERO,
                height: BlockHeight::ZERO,
            },
        };
        engine.recover()?;
        Ok(engine)
    }

    fn header_path(&self) -> PathBuf {
        self.data_dir.join("headers.bin")
    }

    fn flush_write_buffer(&mut self) -> Result<(), EngineError> {
        self.wb_pairs.clear();
        for (op, entry) in self.write_buffer.drain() {
            let BufferEntry::Live(coin) = entry;
            self.wb_pairs.push((op, coin));
        }

        for (op, coin) in self.wb_pairs.drain(..) {
            self.table.insert(&op, &coin).map_err(io_err)?;
        }

        let deletes = std::mem::take(&mut self.pending_disk_deletes);
        self.table
            .remove_batch_uring(
                &deletes,
                &mut self.ring,
                &mut self.read_arena,
                &mut self.wb_del_slots,
                &mut self.wb_del_dists,
                &mut self.wb_del_resolved,
            )
            .map_err(io_err)?;
        self.pending_disk_deletes = deletes;

        Ok(())
    }
}

impl Engine for RobinHoodEngine {
    fn read_coin(&mut self, op: &OutPoint) -> Result<Option<Coin>, EngineError> {
        match self.write_buffer.get(op) {
            Some(BufferEntry::Live(coin)) => return Ok(Some(coin.clone())),
            None => {}
        }
        self.table.get(op).map_err(io_err)
    }

    fn read_coins(
        &mut self,
        ops: &[OutPoint],
        out: &mut Vec<LookupResult>,
    ) -> Result<(), EngineError> {
        let results = &mut self.rc_results;
        let disk_indices = &mut self.rc_disk_indices;
        let disk_outpoints = &mut self.rc_disk_outpoints;
        let disk_coins = &mut self.rc_disk_coins;

        results.clear();
        disk_indices.clear();
        disk_outpoints.clear();

        out.reserve(ops.len());
        results.resize(ops.len(), None);

        for (i, &op) in ops.iter().enumerate() {
            match self.write_buffer.get(&op) {
                Some(BufferEntry::Live(coin)) => results[i] = Some(coin.clone()),
                None => {
                    disk_indices.push(i);
                    disk_outpoints.push(op);
                }
            }
        }

        if !disk_outpoints.is_empty() {
            self.table
                .get_batch_uring(
                    disk_outpoints,
                    disk_coins,
                    &mut self.ring,
                    &mut self.read_arena,
                    &mut self.uring_slots,
                    &mut self.uring_dists,
                    &mut self.uring_resolved,
                )
                .map_err(io_err)?;
            for (j, &i) in disk_indices.iter().enumerate() {
                if let Some(coin) = disk_coins.get(j).and_then(|c| c.as_ref()) {
                    results[i] = Some(coin.clone());
                }
            }
        }

        for (i, &op) in ops.iter().enumerate() {
            out.push(LookupResult {
                outpoint: op,
                coin: results[i].take(),
            });
        }
        Ok(())
    }

    fn write_block(&mut self, req: &CommitBlockReq) -> Result<(), EngineError> {
        for nc in req.created {
            let coin = Coin {
                value: nc.value,
                script_pubkey: ScriptPubKey::from_slice(nc.script_pubkey),
                height: nc.height,
                coinbase: nc.coinbase,
            };
            self.write_buffer
                .insert(nc.outpoint, BufferEntry::Live(coin));
        }
        for op in req.spent {
            match self.write_buffer.remove(op) {
                Some(BufferEntry::Live(_)) => {
                    // Created and spent within the buffer window — elided.
                }
                None => {
                    // Coin is on disk — schedule deletion for next flush.
                    self.pending_disk_deletes.push(*op);
                }
            }
        }

        // Only flush when durability requires persistence. With Memory
        // durability (bench / test paths) the buffer is never written to disk,
        // so flushing here would be O(N) synchronous pread/pwrite for no
        // benefit — just let the buffer grow.
        if req.durability != Durability::Memory
            && self.write_buffer.len() + self.pending_disk_deletes.len()
                > WRITE_BUFFER_FLUSH_THRESHOLD
        {
            self.flush_write_buffer()?;
        }

        debug_assert_eq!(
            req.header.prev_block(),
            req.prev_hash,
            "CommitBlockReq.header.prev_block must match req.prev_hash"
        );
        self.header_write_len = write_header_record(
            &mut self.header_file,
            req.height,
            &req.header,
            req.hash,
            self.header_write_len,
        )?;
        self.header_index.insert(req.hash, req.height.to_u32());

        self.tip = TipInfo {
            hash: req.hash,
            height: req.height,
        };
        Ok(())
    }

    fn read_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, EngineError> {
        let Some(&height_u32) = self.header_index.get(hash) else {
            return Ok(None);
        };
        self.read_header_at(BlockHeight::new(height_u32))
    }

    fn read_header_at(&self, h: BlockHeight) -> Result<Option<BlockHeader>, EngineError> {
        let offset = u64::from(h.to_u32()) * HEADER_RECORD_SIZE as u64;
        if offset + HEADER_RECORD_SIZE as u64 > self.header_write_len {
            return Ok(None);
        }
        let mut buf = [0u8; HEADER_RECORD_SIZE];
        pread_exact(&self.header_file, &mut buf, offset).map_err(io_err)?;
        Ok(Some(decode_header_record(&buf)))
    }

    fn write_header(&mut self, h: BlockHeight, header: &BlockHeader) -> Result<(), EngineError> {
        let hash = header.hash();
        self.header_write_len = write_header_record(
            &mut self.header_file,
            h,
            header,
            hash,
            self.header_write_len,
        )?;
        self.header_index.insert(hash, h.to_u32());
        Ok(())
    }

    fn flush(&mut self, d: Durability) -> Result<(), EngineError> {
        if d == Durability::Synced {
            self.flush_write_buffer()?;
            self.table.sync().map_err(io_err)?;
            self.header_file.flush().map_err(io_err)?;
            self.header_file.sync_all().map_err(io_err)?;
            self.checkpoint_file
                .seek(SeekFrom::Start(0))
                .map_err(io_err)?;
            let h = self.tip.height.to_u32().to_le_bytes();
            self.checkpoint_file.write_all(&h).map_err(io_err)?;
            self.checkpoint_file.sync_all().map_err(io_err)?;
        }
        Ok(())
    }

    fn recover(&mut self) -> Result<TipInfo, EngineError> {
        // Read checkpoint.
        let mut cp_buf = [0u8; 4];
        let n = self.checkpoint_file.read(&mut cp_buf).map_err(io_err)?;
        let checkpoint_height = if n == 4 {
            u32::from_le_bytes(cp_buf)
        } else {
            0
        };

        // Rebuild header index from headers.bin, up to the checkpoint height.
        self.header_index.clear();
        let header_path = self.header_path();
        if let Ok(mut hf) = File::open(&header_path) {
            let file_len = hf.seek(SeekFrom::End(0)).map_err(io_err)?;
            hf.seek(SeekFrom::Start(0)).map_err(io_err)?;
            let record_count = file_len / HEADER_RECORD_SIZE as u64;
            for _i in 0..record_count {
                let mut buf = [0u8; HEADER_RECORD_SIZE];
                if hf.read_exact(&mut buf).is_err() {
                    break;
                }
                let hash = Hash256::from_bytes(buf.read_hash(0));
                let height = buf.read_u32_le(112);
                if height <= checkpoint_height || checkpoint_height == 0 {
                    self.header_index.insert(hash, height);
                    self.tip = TipInfo {
                        hash,
                        height: BlockHeight::new(height),
                    };
                }
            }
            self.header_write_len = record_count * HEADER_RECORD_SIZE as u64;
        }

        // If there's no checkpoint and no headers, the UTXO table is stale —
        // wipe it so a fresh start doesn't inherit data from a previous run.
        if checkpoint_height == 0 && self.header_index.is_empty() && !self.table.is_empty() {
            let table_path = self.data_dir.join("utxo_table.bin");
            self.table = RobinHoodTable::create(&table_path, INITIAL_SLOT_COUNT).map_err(io_err)?;
        }

        Ok(self.tip.clone())
    }

    fn tip(&self) -> TipInfo {
        self.tip.clone()
    }
}

// ── Header record serialization (shared with FlatEngine) ─────────────────────

fn write_header_record(
    f: &mut File,
    h: BlockHeight,
    header: &BlockHeader,
    hash: Hash256,
    file_len: u64,
) -> Result<u64, EngineError> {
    let offset = u64::from(h.to_u32()) * HEADER_RECORD_SIZE as u64;
    let new_len = if offset > file_len {
        f.set_len(offset).map_err(io_err)?;
        offset
    } else {
        file_len
    };
    f.write_all(hash.as_bytes()).map_err(io_err)?;
    f.write_all(&header.version().to_le_bytes())
        .map_err(io_err)?;
    f.write_all(header.prev_block().as_bytes())
        .map_err(io_err)?;
    f.write_all(header.merkle_root().as_bytes())
        .map_err(io_err)?;
    f.write_all(&header.time().to_le_bytes()).map_err(io_err)?;
    f.write_all(&header.bits().to_le_bytes()).map_err(io_err)?;
    f.write_all(&header.nonce().to_le_bytes()).map_err(io_err)?;
    f.write_all(&h.to_u32().to_le_bytes()).map_err(io_err)?;
    Ok(new_len + HEADER_RECORD_SIZE as u64)
}

#[allow(clippy::cast_possible_wrap)]
fn decode_header_record(buf: &[u8; HEADER_RECORD_SIZE]) -> BlockHeader {
    use mantle::primitives::{BlockTime, BlockVersion, Nonce};
    BlockHeader::new(
        BlockVersion::new(buf.read_u32_le(32) as i32),
        Hash256::from_bytes(buf.read_hash(36)),
        Hash256::from_bytes(buf.read_hash(68)),
        BlockTime::new(buf.read_u32_le(100)),
        CompactBits::new(buf.read_u32_le(104)),
        Nonce::new(buf.read_u32_le(108)),
    )
}

// ── Overflow sidecar helpers ────────────────────────────────────────────────
//
// Layout per record: u32_le(len) + script_bytes.  The offset stored in the
// bucket's inline script_pubkey[0..8] points to the start of the record.

fn overflow_path(table_path: &Path) -> PathBuf {
    table_path.with_extension("overflow")
}

fn open_overflow_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn append_overflow(file: &mut File, spk: &[u8]) -> io::Result<u64> {
    let offset = file.seek(SeekFrom::End(0))?;
    let len = u32::try_from(spk.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "script > 4 GiB"))?;
    file.write_all(&len.to_le_bytes())?;
    file.write_all(spk)?;
    Ok(offset)
}

fn read_overflow(file: &File, offset: u64) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    pread_exact(file, &mut len_buf, offset)?;
    let len = usize::try_from(u32::from_le_bytes(len_buf)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "overflow script length exceeds usize",
        )
    })?;
    let mut buf = vec![0u8; len];
    pread_exact(file, &mut buf, offset + 4)?;
    Ok(buf)
}

fn io_err(e: impl Into<io::Error>) -> EngineError {
    EngineError::Io(e.into())
}

fn uring_err(e: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("{e:?}"))
}

// ── pread/pwrite helpers ─────────────────────────────────────────────────────

#[cfg(unix)]
fn pread_exact(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

#[cfg(unix)]
fn pwrite_all(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

/// Push `sqe` onto the ring. If the SQ is full, submit and drain all in-flight
/// entries first, then retry. `submitted` tracks how many SQEs are pending
/// for the caller's final `submit_and_wait`.
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
            ring.submit_and_wait(*submitted).map_err(uring_err)?;
            while ring.complete().is_some() {}
            *submitted = 0;
            ring.push(copy).map_err(uring_err)?;
            *submitted += 1;
            Ok(())
        }
        Err(e) => Err(uring_err(e)),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mantle::primitives::{BlockTime, BlockVersion, Nonce};

    struct TmpFile(PathBuf);
    impl TmpFile {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "mantle-rh-{}-{}-{}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos()
            ));
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let new_path = self.0.with_extension("new");
            let _ = std::fs::remove_file(&new_path);
            let _ = std::fs::remove_file(overflow_path(&self.0));
            let _ = std::fs::remove_file(overflow_path(&new_path));
        }
    }

    fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
            .collect()
    }

    fn make_coin(value: u64, spk: &[u8]) -> Coin {
        Coin {
            value: Satoshi::new(value),
            script_pubkey: ScriptPubKey::from_slice(spk),
            height: BlockHeight::new(100),
            coinbase: false,
        }
    }

    fn make_op(i: u8) -> OutPoint {
        OutPoint {
            txid: Hash256::from_bytes([i; 32]),
            vout: 0,
        }
    }

    #[test]
    fn create_and_reopen() {
        let tmp = TmpFile::new("create");
        {
            let table = RobinHoodTable::open(tmp.path()).unwrap();
            assert_eq!(table.len(), 0);
            assert!(table.is_empty());
        }
        {
            let table = RobinHoodTable::open(tmp.path()).unwrap();
            assert_eq!(table.len(), 0);
        }
    }

    #[test]
    fn insert_get_remove() {
        let tmp = TmpFile::new("basic");
        let mut table = RobinHoodTable::open(tmp.path()).unwrap();

        let op = make_op(1);
        let coin = make_coin(5_000_000_000, &[0x51]);

        assert!(table.get(&op).unwrap().is_none());

        table.insert(&op, &coin).unwrap();
        assert_eq!(table.len(), 1);

        let got = table.get(&op).unwrap().unwrap();
        assert_eq!(got.value.to_u64(), 5_000_000_000);
        assert_eq!(got.script_pubkey.as_slice(), &[0x51]);

        assert!(table.remove(&op).unwrap());
        assert_eq!(table.len(), 0);
        assert!(table.get(&op).unwrap().is_none());
        assert!(!table.remove(&op).unwrap());
    }

    #[test]
    fn insert_many_triggers_resize() {
        let tmp = TmpFile::new("resize");
        let mut table = RobinHoodTable::open_with_capacity(tmp.path(), 16).unwrap();
        let initial_slots = table.slot_count;
        assert_eq!(initial_slots, 16);

        for i in 0..200u8 {
            let op = make_op(i);
            let coin = make_coin(u64::from(i) * 1000, &[0x51]);
            table.insert(&op, &coin).unwrap();
        }
        assert_eq!(table.len(), 200);
        assert!(table.slot_count > initial_slots);

        for i in 0..200u8 {
            let op = make_op(i);
            let got = table.get(&op).unwrap().unwrap();
            assert_eq!(got.value.to_u64(), u64::from(i) * 1000);
        }
    }

    #[test]
    fn persistence_across_reopen() {
        let tmp = TmpFile::new("persist");
        let op = make_op(42);
        let coin = make_coin(999, &[0x76, 0xa9]);
        {
            let mut table = RobinHoodTable::open(tmp.path()).unwrap();
            table.insert(&op, &coin).unwrap();
            table.sync().unwrap();
        }
        {
            let table = RobinHoodTable::open(tmp.path()).unwrap();
            assert_eq!(table.len(), 1);
            let got = table.get(&op).unwrap().unwrap();
            assert_eq!(got.value.to_u64(), 999);
            assert_eq!(got.script_pubkey.as_slice(), &[0x76, 0xa9]);
        }
    }

    #[test]
    fn remove_with_backshift() {
        let tmp = TmpFile::new("backshift");
        let mut table = RobinHoodTable::open_with_capacity(tmp.path(), 16).unwrap();

        let ops: Vec<OutPoint> = (0..10u8).map(make_op).collect();
        for (i, op) in ops.iter().enumerate() {
            let coin = make_coin(u64::try_from(i).unwrap(), &[0x51]);
            table.insert(op, &coin).unwrap();
        }

        table.remove(&ops[3]).unwrap();
        table.remove(&ops[7]).unwrap();
        assert_eq!(table.len(), 8);

        for (i, op) in ops.iter().enumerate() {
            if i == 3 || i == 7 {
                assert!(table.get(op).unwrap().is_none());
            } else {
                let got = table.get(op).unwrap().unwrap();
                assert_eq!(got.value.to_u64(), u64::try_from(i).unwrap());
            }
        }
    }

    #[test]
    fn update_existing_key() {
        let tmp = TmpFile::new("update");
        let mut table = RobinHoodTable::open(tmp.path()).unwrap();

        let op = make_op(5);
        table.insert(&op, &make_coin(100, &[0x51])).unwrap();
        table.insert(&op, &make_coin(200, &[0x51])).unwrap();

        assert_eq!(table.len(), 1);
        assert_eq!(table.get(&op).unwrap().unwrap().value.to_u64(), 200);
    }

    #[test]
    fn standard_script_lengths() {
        let tmp = TmpFile::new("scripts");
        let mut table = RobinHoodTable::open(tmp.path()).unwrap();

        let cases: &[(&str, &[u8])] = &[
            (
                "p2pkh",
                &[
                    0x76, 0xa9, 0x14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
                    18, 19, 20, 0x88, 0xac,
                ],
            ),
            (
                "p2sh",
                &[
                    0xa9, 0x14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
                    20, 0x87,
                ],
            ),
            (
                "p2wpkh",
                &[
                    0x00, 0x14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
                    20,
                ],
            ),
            ("p2wsh", &{
                let mut v = vec![0x00, 0x20];
                v.extend_from_slice(&[0xAB; 32]);
                v
            }),
            ("p2tr", &{
                let mut v = vec![0x51, 0x20];
                v.extend_from_slice(&[0xCD; 32]);
                v
            }),
        ];

        for (i, (name, spk)) in cases.iter().enumerate() {
            let op = make_op(u8::try_from(i).expect("test index fits u8"));
            let coin = make_coin(u64::try_from(i).unwrap(), spk);
            table.insert(&op, &coin).unwrap();
            let got = table.get(&op).unwrap().unwrap();
            assert_eq!(
                got.script_pubkey.as_slice(),
                *spk,
                "{name} script round-trip failed"
            );
        }
    }

    #[test]
    fn overflow_script_roundtrip() {
        let tmp = TmpFile::new("overflow");
        let mut table = RobinHoodTable::open(tmp.path()).unwrap();

        // 80-byte bare multisig — exceeds MAX_INLINE_SPK (34).
        let big_spk: Vec<u8> = (0..80).collect();
        let op = make_op(1);
        let coin = make_coin(42_000, &big_spk);
        table.insert(&op, &coin).unwrap();

        let got = table.get(&op).unwrap().unwrap();
        assert_eq!(got.script_pubkey.as_slice(), &big_spk[..]);
        assert_eq!(got.value.to_u64(), 42_000);
    }

    #[test]
    fn overflow_survives_resize() {
        let tmp = TmpFile::new("overflow-resize");
        let mut table = RobinHoodTable::open_with_capacity(tmp.path(), 16).unwrap();

        let big_spk: Vec<u8> = (0..64).collect();
        let overflow_op = make_op(200);
        table
            .insert(&overflow_op, &make_coin(999, &big_spk))
            .unwrap();

        // Fill until resize triggers.
        for i in 0..100u8 {
            table
                .insert(&make_op(i), &make_coin(u64::from(i), &[0x51]))
                .unwrap();
        }
        assert!(table.slot_count > 16);

        let got = table.get(&overflow_op).unwrap().unwrap();
        assert_eq!(got.script_pubkey.as_slice(), &big_spk[..]);
        assert_eq!(got.value.to_u64(), 999);
    }

    #[test]
    fn batched_uring_lookup() {
        let tmp = TmpFile::new("uring-batch");
        let mut table = RobinHoodTable::open_with_capacity(tmp.path(), 64).unwrap();
        let mut ring = ququmatz::IoUring::new(256).unwrap();
        let mut arena = Vec::new();

        let count = 50u8;
        for i in 0..count {
            let op = make_op(i);
            let coin = make_coin(u64::from(i) * 100, &[0x51]);
            table.insert(&op, &coin).unwrap();
        }

        let ops: Vec<OutPoint> = (0..count).map(make_op).collect();
        let mut results = Vec::new();
        let (mut slots, mut dists, mut resolved) = (Vec::new(), Vec::new(), Vec::new());
        table
            .get_batch_uring(
                &ops,
                &mut results,
                &mut ring,
                &mut arena,
                &mut slots,
                &mut dists,
                &mut resolved,
            )
            .unwrap();

        assert_eq!(results.len(), count as usize);
        for (i, result) in results.iter().enumerate() {
            let coin = result.as_ref().unwrap();
            assert_eq!(coin.value.to_u64(), i as u64 * 100);
        }

        // Also test miss: outpoints not in the table.
        let miss_ops: Vec<OutPoint> = (200..210u8).map(make_op).collect();
        let mut miss_results = Vec::new();
        table
            .get_batch_uring(
                &miss_ops,
                &mut miss_results,
                &mut ring,
                &mut arena,
                &mut slots,
                &mut dists,
                &mut resolved,
            )
            .unwrap();
        assert!(miss_results.iter().all(Option::is_none));
    }

    #[test]
    fn write_buffer_elides_create_then_spend() {
        let dir = TmpFile::new("wb-elide");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let op = OutPoint {
            txid: Hash256::from_bytes([10u8; 32]),
            vout: 0,
        };

        // Create a coin — goes into write buffer, not disk.
        let req1 = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: Hash256::from_bytes([11u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(1000),
                script_pubkey: &[0x51],
                coinbase: false,
                height: BlockHeight::new(1),
            }],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req1).unwrap();

        // Read from buffer — should find it.
        assert!(eng.read_coin(&op).unwrap().is_some());

        // Disk table should be empty (not flushed yet).
        assert_eq!(eng.table.len(), 0);

        // Spend the coin — should elide (never hits disk).
        let req2 = CommitBlockReq {
            token: crate::types::ReqToken(2),
            height: BlockHeight::new(2),
            hash: Hash256::from_bytes([12u8; 32]),
            prev_hash: Hash256::from_bytes([11u8; 32]),
            header: {
                let mut h = make_header(0);
                h.set_prev_block(Hash256::from_bytes([11u8; 32]));
                h
            },
            created: &[],
            spent: &[op],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req2).unwrap();

        // Coin should be gone from buffer and disk.
        assert!(eng.read_coin(&op).unwrap().is_none());
        assert_eq!(eng.table.len(), 0);
        assert!(eng.pending_disk_deletes.is_empty());
    }

    #[test]
    fn write_buffer_flushes_on_sync() {
        let dir = TmpFile::new("wb-flush");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let op = OutPoint {
            txid: Hash256::from_bytes([20u8; 32]),
            vout: 0,
        };
        let req = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: Hash256::from_bytes([21u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(2000),
                script_pubkey: &[0x51],
                coinbase: false,
                height: BlockHeight::new(1),
            }],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req).unwrap();

        // Not on disk yet.
        assert_eq!(eng.table.len(), 0);

        // Flush with Synced durability.
        eng.flush(Durability::Synced).unwrap();

        // Now on disk.
        assert_eq!(eng.table.len(), 1);
        assert!(eng.write_buffer.is_empty());
    }

    // ── Engine integration tests ─────────────────────────────────────────

    fn make_header(time: u32) -> BlockHeader {
        BlockHeader::new(
            BlockVersion::new(1),
            Hash256::ZERO,
            Hash256::ZERO,
            BlockTime::new(time),
            CompactBits::new(0x1d00_ffff),
            Nonce::new(0),
        )
    }

    #[test]
    fn engine_write_and_read_coin() {
        let dir = TmpFile::new("eng-coin");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let op = OutPoint {
            txid: Hash256::from_bytes([1u8; 32]),
            vout: 0,
        };
        let req = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: Hash256::from_bytes([2u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(5_000_000_000),
                script_pubkey: &[0x51],
                coinbase: true,
                height: BlockHeight::new(1),
            }],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req).unwrap();
        let coin = eng.read_coin(&op).unwrap().unwrap();
        assert_eq!(coin.value.to_u64(), 5_000_000_000);
        assert!(coin.coinbase);
    }

    #[test]
    fn engine_spend_removes_coin() {
        let dir = TmpFile::new("eng-spend");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let op = OutPoint {
            txid: Hash256::from_bytes([3u8; 32]),
            vout: 0,
        };
        let req1 = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(10),
            hash: Hash256::from_bytes([4u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(1000),
                script_pubkey: &[],
                coinbase: false,
                height: BlockHeight::new(10),
            }],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req1).unwrap();
        assert!(eng.read_coin(&op).unwrap().is_some());

        let req2 = CommitBlockReq {
            token: crate::types::ReqToken(2),
            height: BlockHeight::new(11),
            hash: Hash256::from_bytes([5u8; 32]),
            prev_hash: Hash256::from_bytes([4u8; 32]),
            header: {
                let mut h = make_header(0);
                h.set_prev_block(Hash256::from_bytes([4u8; 32]));
                h
            },
            created: &[],
            spent: &[op],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req2).unwrap();
        assert!(eng.read_coin(&op).unwrap().is_none());
    }

    #[test]
    fn engine_header_roundtrip() {
        let dir = TmpFile::new("eng-hdr");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let h = BlockHeader::new(
            BlockVersion::new(0x2000_0007_i32),
            Hash256::from_bytes([0x11; 32]),
            Hash256::from_bytes([0x22; 32]),
            BlockTime::new(12_345),
            CompactBits::new(0x1d00_ffff),
            Nonce::new(0xdead_beef),
        );
        eng.write_header(BlockHeight::new(0), &h).unwrap();
        let got = eng.read_header_at(BlockHeight::new(0)).unwrap().unwrap();
        assert_eq!(got.version(), h.version());
        assert_eq!(got.prev_block(), h.prev_block());
        assert_eq!(got.merkle_root(), h.merkle_root());
        assert_eq!(got.time(), h.time());
        assert_eq!(got.bits().to_u32(), h.bits().to_u32());
        assert_eq!(got.nonce(), h.nonce());
    }

    #[test]
    fn write_buffer_survives_2000_blocks() {
        let dir = TmpFile::new("wb-2k");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let target_height = 1965u32;
        let mut prev_hash = Hash256::ZERO;

        for h in 1..=2000u32 {
            let mut txid_bytes = [0u8; 32];
            txid_bytes[..4].copy_from_slice(&h.to_le_bytes());
            let txid = Hash256::from_bytes(txid_bytes);
            let op = OutPoint { txid, vout: 0 };

            let mut hash_bytes = [0u8; 32];
            hash_bytes[..4].copy_from_slice(&(h + 10000).to_le_bytes());
            let block_hash = Hash256::from_bytes(hash_bytes);

            let req = CommitBlockReq {
                token: crate::types::ReqToken(u64::from(h)),
                height: BlockHeight::new(h),
                hash: block_hash,
                prev_hash,
                header: {
                    let mut h = make_header(h);
                    h.set_prev_block(prev_hash);
                    h
                },
                created: &[mantle_consensus::chain_view::NewCoin {
                    outpoint: op,
                    value: Satoshi::new(5_000_000_000),
                    script_pubkey: &[
                        0x00, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
                        0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
                    ],
                    coinbase: true,
                    height: BlockHeight::new(h),
                }],
                spent: &[],
                block_bytes: None,
                durability: Durability::Memory,
            };
            eng.write_block(&req).unwrap();
            prev_hash = block_hash;
        }

        // Look up the coin from block 1965 via read_coin (single).
        let mut target_txid = [0u8; 32];
        target_txid[..4].copy_from_slice(&target_height.to_le_bytes());
        let target_op = OutPoint {
            txid: Hash256::from_bytes(target_txid),
            vout: 0,
        };
        let coin = eng.read_coin(&target_op).unwrap();
        assert!(
            coin.is_some(),
            "read_coin should find block 1965 coinbase in write buffer"
        );
        assert_eq!(coin.unwrap().height.to_u32(), target_height);

        // Look up via read_coins (batch) — this is the path validate_and_commit uses.
        let mut out = Vec::new();
        eng.read_coins(&[target_op], &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert!(
            out[0].coin.is_some(),
            "read_coins should find block 1965 coinbase in write buffer"
        );
        assert_eq!(out[0].coin.as_ref().unwrap().height.to_u32(), target_height);
    }

    #[test]
    fn write_buffer_survives_2000_blocks_via_handle() {
        let dir = TmpFile::new("wb-2k-handle");
        std::fs::create_dir_all(dir.path()).unwrap();
        let eng = RobinHoodEngine::open(dir.path()).unwrap();

        crate::with_storage(eng, |handle| {
            let target_height = 1965u32;
            let mut prev_hash = Hash256::ZERO;

            for h in 1..=2000u32 {
                let mut txid_bytes = [0u8; 32];
                txid_bytes[..4].copy_from_slice(&h.to_le_bytes());
                let op = OutPoint {
                    txid: Hash256::from_bytes(txid_bytes),
                    vout: 0,
                };

                let mut hash_bytes = [0u8; 32];
                hash_bytes[..4].copy_from_slice(&(h + 10000).to_le_bytes());
                let block_hash = Hash256::from_bytes(hash_bytes);

                let spk = [
                    0x00, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                    0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
                ];
                let created = [mantle_consensus::chain_view::NewCoin {
                    outpoint: op,
                    value: Satoshi::new(5_000_000_000),
                    script_pubkey: &spk,
                    coinbase: true,
                    height: BlockHeight::new(h),
                }];
                handle
                    .commit_block_borrowed(CommitBlockReq {
                        token: crate::types::ReqToken(u64::from(h)),
                        height: BlockHeight::new(h),
                        hash: block_hash,
                        prev_hash,
                        header: {
                            let mut h = make_header(h);
                            h.set_prev_block(prev_hash);
                            h
                        },
                        created: &created,
                        spent: &[],
                        block_bytes: None,
                        durability: Durability::Memory,
                    })
                    .expect("commit_block_borrowed");
                prev_hash = block_hash;
            }

            let mut target_txid = [0u8; 32];
            target_txid[..4].copy_from_slice(&target_height.to_le_bytes());
            let target_op = OutPoint {
                txid: Hash256::from_bytes(target_txid),
                vout: 0,
            };

            let mut out = Vec::new();
            handle
                .lookup_coins_borrowed(&[target_op], &mut out)
                .expect("lookup_coins_borrowed");
            assert_eq!(out.len(), 1, "should get exactly one result");
            assert!(
                out[0].coin.is_some(),
                "handle lookup should find block 1965 coinbase in write buffer"
            );
            assert_eq!(out[0].coin.as_ref().unwrap().height.to_u32(), target_height);
        })
        .expect("with_storage");
    }

    #[test]
    fn write_buffer_survives_2000_blocks_via_open_robin_hood() {
        let dir = TmpFile::new("wb-2k-rh");
        std::fs::create_dir_all(dir.path()).unwrap();

        let mut handle =
            crate::StorageHandle::open_robin_hood(dir.path()).expect("open_robin_hood");

        let target_height = 1965u32;
        let mut prev_hash = Hash256::ZERO;

        for h in 1..=2000u32 {
            let mut txid_bytes = [0u8; 32];
            txid_bytes[..4].copy_from_slice(&h.to_le_bytes());
            let op = OutPoint {
                txid: Hash256::from_bytes(txid_bytes),
                vout: 0,
            };

            let mut hash_bytes = [0u8; 32];
            hash_bytes[..4].copy_from_slice(&(h + 10000).to_le_bytes());
            let block_hash = Hash256::from_bytes(hash_bytes);

            let spk = [
                0x00u8, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
            ];
            let created = [mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(5_000_000_000),
                script_pubkey: &spk,
                coinbase: true,
                height: BlockHeight::new(h),
            }];
            handle
                .commit_block_borrowed(CommitBlockReq {
                    token: crate::types::ReqToken(u64::from(h)),
                    height: BlockHeight::new(h),
                    hash: block_hash,
                    prev_hash,
                    header: {
                        let mut h = make_header(h);
                        h.set_prev_block(prev_hash);
                        h
                    },
                    created: &created,
                    spent: &[],
                    block_bytes: None,
                    durability: Durability::Memory,
                })
                .expect("commit_block_borrowed");
            prev_hash = block_hash;
        }

        let mut target_txid = [0u8; 32];
        target_txid[..4].copy_from_slice(&target_height.to_le_bytes());
        let target_op = OutPoint {
            txid: Hash256::from_bytes(target_txid),
            vout: 0,
        };

        let mut out = Vec::new();
        handle
            .lookup_coins_borrowed(&[target_op], &mut out)
            .expect("lookup_coins_borrowed");
        assert_eq!(out.len(), 1, "should get exactly one result");
        assert!(
            out[0].coin.is_some(),
            "open_robin_hood handle should find block 1965 coinbase in write buffer"
        );
        assert_eq!(out[0].coin.as_ref().unwrap().height.to_u32(), target_height);

        handle.shutdown().expect("shutdown");
    }

    #[test]
    fn replay_block_1965_then_lookup_for_2067() {
        use mantle::codec::Decode;
        use mantle::primitives::{Block, TxHash};
        use mantle_consensus::chain_view::NewCoin;

        let dir = TmpFile::new("replay-1965");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let block_1965_hex = "000000205ede77c0cb37c34615b3b20a2a240dfb5ba9e4eb45ca84c91fbf263d0000000081f980c7703039a91a9a7c5365f9902975e2944209629fa58fc36dd1a25e183192433966ffff001d865e2a0001020000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff0402ad0700ffffffff0200f2052a01000000160014690912f95570260001bc668468269a82b382fd040000000000000000266a24aa21a9ede2f61c3f71d1defd3fa999dfa36953755c690689799962b48bebd836974e8cf90120000000000000000000000000000000000000000000000000000000000000000000000000";
        let block_bytes = decode_hex(block_1965_hex).unwrap();
        let block = Block::decode(&mut std::io::Cursor::new(&block_bytes)).unwrap();

        let coinbase = &block.transactions[0];
        let txid = coinbase.txid().unwrap();
        eprintln!("coinbase txid (internal): {:?}", &txid.as_bytes()[..4]);

        let mut created = Vec::new();
        for (vout, output) in coinbase.outputs.iter().enumerate() {
            created.push(NewCoin {
                outpoint: OutPoint {
                    txid,
                    #[allow(clippy::cast_possible_truncation)]
                    vout: vout as u32,
                },
                value: output.value,
                script_pubkey: output.script_pubkey.as_slice(),
                coinbase: true,
                height: BlockHeight::new(1965),
            });
        }

        let hash = block.header.hash();
        eng.write_block(&CommitBlockReq {
            token: crate::types::ReqToken(1965),
            height: BlockHeight::new(1965),
            hash,
            prev_hash: block.header.prev_block(),
            header: block.header,
            created: &created,
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        })
        .unwrap();

        // Now look up the outpoint that block 2067 tries to spend
        let block_2067_hex = "00000020f009a7425a6b0962bc12d3a2d2497588182202b9b59d46d33835ad0300000000b2ed3db0e5f468e9faa29b50a2bbe9cb041c40c2ecd53156a3d51173719e8e6c6d4b3966c0ff3f1c65c7bf0102010000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff0a021308062f4077697a2fffffffff028df2052a01000000160014a54e2a1ec06389203887661535ed118b7d0538890000000000000000266a24aa21a9ed2b5052d89d429b990f78f48e0b0332e2caf6cea8c878168f9847e5b6b3daff8201200000000000000000000000000000000000000000000000000000000000000000000000000200000000010181f980c7703039a91a9a7c5365f9902975e2944209629fa58fc36dd1a25e18310000000000fdffffff022005000000000000160014a54e2a1ec06389203887661535ed118b7d05388953ec052a01000000160014505c4fa6dd11e0b3a0b0211cfa3ddf33958dd8bd02473044022032d0f97adef165f69da184332f68c91fc27782444aacc213088ac7c5ba6fe3a602206d7dbb42dcc78ba1661bb3106bb8ae1b424036592c0a16c9b51adcad46731eba012102476e8b9914532f15a42d34840b3b143746d1bb35d536055a276b41f1321479e111080000";
        let block_2067_bytes = decode_hex(block_2067_hex).unwrap();
        let block_2067 = Block::decode(&mut std::io::Cursor::new(&block_2067_bytes)).unwrap();

        let spend_input = &block_2067.transactions[1].inputs[0];
        let spend_op = spend_input.previous_output;
        eprintln!(
            "block 2067 tx[1] input[0] spends {:?}:{}",
            &spend_op.txid.as_bytes()[..4],
            spend_op.vout
        );
        eprintln!("txids match: {}", spend_op.txid == txid);

        let coin = eng.read_coin(&spend_op).unwrap();
        assert!(
            coin.is_some(),
            "engine should find block 1965 coinbase output 0"
        );
    }

    #[test]
    fn flush_then_lookup_via_engine() {
        let dir = TmpFile::new("flush-lookup");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let target_height = 1965u32;
        let mut prev_hash = Hash256::ZERO;

        for h in 1..=2016u32 {
            let mut txid_bytes = [0u8; 32];
            txid_bytes[..4].copy_from_slice(&h.to_le_bytes());
            let op = OutPoint {
                txid: Hash256::from_bytes(txid_bytes),
                vout: 0,
            };

            let mut hash_bytes = [0u8; 32];
            hash_bytes[..4].copy_from_slice(&(h + 10000).to_le_bytes());
            let block_hash = Hash256::from_bytes(hash_bytes);

            let durability = if h == 2016 {
                Durability::Synced
            } else {
                Durability::Memory
            };

            let spk = [
                0x00u8, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
            ];
            eng.write_block(&CommitBlockReq {
                token: crate::types::ReqToken(u64::from(h)),
                height: BlockHeight::new(h),
                hash: block_hash,
                prev_hash,
                header: {
                    let mut h = make_header(h);
                    h.set_prev_block(prev_hash);
                    h
                },
                created: &[mantle_consensus::chain_view::NewCoin {
                    outpoint: op,
                    value: Satoshi::new(5_000_000_000),
                    script_pubkey: &spk,
                    coinbase: true,
                    height: BlockHeight::new(h),
                }],
                spent: &[],
                block_bytes: None,
                durability,
            })
            .unwrap();
            eng.flush(durability).unwrap();
            prev_hash = block_hash;
        }

        // Write buffer should be empty after Synced flush at 2016.
        assert!(
            eng.write_buffer.is_empty(),
            "write buffer should be flushed"
        );
        assert!(!eng.table.is_empty(), "table should have entries");

        // Look up block 1965 coinbase via read_coin (synchronous pread).
        let mut target_txid = [0u8; 32];
        target_txid[..4].copy_from_slice(&target_height.to_le_bytes());
        let target_op = OutPoint {
            txid: Hash256::from_bytes(target_txid),
            vout: 0,
        };

        let coin_sync = eng.table.get(&target_op).unwrap();
        assert!(
            coin_sync.is_some(),
            "synchronous table.get should find the coin"
        );

        // Look up via read_coins (batch uring path).
        let mut out = Vec::new();
        eng.read_coins(&[target_op], &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert!(
            out[0].coin.is_some(),
            "batch uring read_coins should find the coin after flush"
        );
    }

    #[test]
    fn flush_then_lookup_via_handle() {
        let dir = TmpFile::new("flush-handle");
        std::fs::create_dir_all(dir.path()).unwrap();
        let eng = RobinHoodEngine::open(dir.path()).unwrap();

        crate::with_storage(eng, |handle| {
            let target_height = 1965u32;
            let mut prev_hash = Hash256::ZERO;

            for h in 1..=2016u32 {
                let mut txid_bytes = [0u8; 32];
                txid_bytes[..4].copy_from_slice(&h.to_le_bytes());
                let op = OutPoint {
                    txid: Hash256::from_bytes(txid_bytes),
                    vout: 0,
                };
                let mut hash_bytes = [0u8; 32];
                hash_bytes[..4].copy_from_slice(&(h + 10000).to_le_bytes());
                let block_hash = Hash256::from_bytes(hash_bytes);
                let durability = if h == 2016 {
                    Durability::Synced
                } else {
                    Durability::Memory
                };
                let spk = [
                    0x00u8, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                    0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
                ];
                let created = [mantle_consensus::chain_view::NewCoin {
                    outpoint: op,
                    value: Satoshi::new(5_000_000_000),
                    script_pubkey: &spk,
                    coinbase: true,
                    height: BlockHeight::new(h),
                }];
                handle
                    .commit_block_borrowed(CommitBlockReq {
                        token: crate::types::ReqToken(u64::from(h)),
                        height: BlockHeight::new(h),
                        hash: block_hash,
                        prev_hash,
                        header: {
                            let mut h = make_header(h);
                            h.set_prev_block(prev_hash);
                            h
                        },
                        created: &created,
                        spent: &[],
                        block_bytes: None,
                        durability,
                    })
                    .expect("commit_block_borrowed");
                prev_hash = block_hash;
            }

            let mut target_txid = [0u8; 32];
            target_txid[..4].copy_from_slice(&target_height.to_le_bytes());
            let target_op = OutPoint {
                txid: Hash256::from_bytes(target_txid),
                vout: 0,
            };

            let mut out = Vec::new();
            handle
                .lookup_coins_borrowed(&[target_op], &mut out)
                .expect("lookup");
            assert_eq!(out.len(), 1);
            assert!(
                out[0].coin.is_some(),
                "handle lookup should find coin after Synced flush at 2016"
            );
        })
        .unwrap();
    }
}
