//! On-disk Robin Hood hash table for the UTXO set.
//!
//! Fixed 88-byte buckets stored in `utxo_table.bin`. Power-of-two slot count.
//! Standard scriptPubKeys (≤34 bytes) stored inline; nonstandard overflow to
//! a sidecar file. Phase 1 uses synchronous `pread`/`pwrite`; Phase 2 replaces
//! with io_uring batched I/O.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::hash::ByteSliceExt;
use mantle::primitives::{BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, ScriptPubKey};
use mantle_consensus::chain_view::{Coin, TipInfo};

use super::{Engine, EngineError};
use crate::types::{CommitBlockReq, Durability, LookupResult};

const BUCKET_SIZE: usize = 88;
const MAGIC: [u8; 8] = *b"MANTLEU\0";
const FORMAT_VERSION: u32 = 1;
const INITIAL_SLOT_COUNT: u64 = 1024;
const MAX_INLINE_SPK: usize = 34;
const OVERFLOW_MARKER: u16 = 0xFFFF;
const LOAD_FACTOR_NUM: u64 = 85;
const LOAD_FACTOR_DEN: u64 = 100;

// ── Bucket layout ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Bucket {
    probe_distance: u8,
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
            txid: Hash256(self.txid),
            vout: self.vout,
        }
    }

    fn matches(&self, op: &OutPoint) -> bool {
        self.txid == op.txid.0 && self.vout == op.vout
    }

    fn to_coin(&self) -> Coin {
        let spk = if self.spk_len == OVERFLOW_MARKER {
            // TODO: read from overflow sidecar file
            &[]
        } else {
            &self.script_pubkey[..self.spk_len as usize]
        };
        Coin {
            value: Satoshi::new(self.value),
            script_pubkey: ScriptPubKey::from_slice(spk),
            height: BlockHeight(self.height),
            coinbase: self.coinbase,
        }
    }

    fn from_outpoint_and_coin(op: &OutPoint, coin: &Coin, probe_distance: u8) -> Self {
        let spk = coin.script_pubkey.as_slice();
        let spk_len = spk.len();
        let mut script_pubkey = [0u8; MAX_INLINE_SPK];
        if spk_len <= MAX_INLINE_SPK {
            script_pubkey[..spk_len].copy_from_slice(spk);
        }
        Self {
            probe_distance,
            coinbase: coin.coinbase,
            spk_len: if spk_len <= MAX_INLINE_SPK {
                spk_len as u16
            } else {
                OVERFLOW_MARKER
            },
            height: coin.height.0,
            value: coin.value.to_u64(),
            txid: op.txid.0,
            vout: op.vout,
            script_pubkey,
        }
    }

    fn encode(&self, buf: &mut [u8; BUCKET_SIZE]) {
        buf[0] = self.probe_distance;
        buf[1] = u8::from(self.coinbase);
        buf[2..4].copy_from_slice(&self.spk_len.to_le_bytes());
        buf[4..8].copy_from_slice(&self.height.to_le_bytes());
        buf[8..16].copy_from_slice(&self.value.to_le_bytes());
        buf[16..48].copy_from_slice(&self.txid);
        buf[48..52].copy_from_slice(&self.vout.to_le_bytes());
        buf[52..86].copy_from_slice(&self.script_pubkey);
        buf[86..88].copy_from_slice(&[0, 0]);
    }

    fn decode(buf: &[u8; BUCKET_SIZE]) -> Self {
        let mut txid = [0u8; 32];
        txid.copy_from_slice(&buf[16..48]);
        let mut script_pubkey = [0u8; MAX_INLINE_SPK];
        script_pubkey.copy_from_slice(&buf[52..86]);
        Self {
            probe_distance: buf[0],
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

// ── Hash function ────────────────────────────────────────────────────────────

fn hash_outpoint(op: &OutPoint) -> u64 {
    // OutPoint.txid is SHA-256d output — first 8 bytes are uniformly distributed.
    u64::from_le_bytes(op.txid.0[..8].try_into().unwrap())
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
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad UTXO table magic"));
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
    slot_count: u64,
    mask: u64,
    live_count: u64,
}

impl RobinHoodTable {
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

        let file_size = (slot_count + 1) * BUCKET_SIZE as u64;
        file.set_len(file_size)?;

        let header = TableHeader { slot_count, live_count: 0 };
        let mut buf = [0u8; BUCKET_SIZE];
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
        let mut buf = [0u8; BUCKET_SIZE];
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

    fn slot_offset(&self, slot: u64) -> u64 {
        (slot + 1) * BUCKET_SIZE as u64
    }

    fn read_bucket(&self, slot: u64) -> io::Result<Bucket> {
        let mut buf = [0u8; BUCKET_SIZE];
        pread_exact(&self.file, &mut buf, self.slot_offset(slot))?;
        Ok(Bucket::decode(&buf))
    }

    fn write_bucket(&self, slot: u64, bucket: &Bucket) -> io::Result<()> {
        let mut buf = [0u8; BUCKET_SIZE];
        bucket.encode(&mut buf);
        pwrite_all(&self.file, &buf, self.slot_offset(slot))
    }

    fn write_empty(&self, slot: u64) -> io::Result<()> {
        let buf = [0u8; BUCKET_SIZE];
        pwrite_all(&self.file, &buf, self.slot_offset(slot))
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
    pub fn fd(&self) -> i32 {
        use std::os::unix::io::AsRawFd;
        self.file.as_raw_fd()
    }

    // ── Public API ───────────────────────────────────────────────────────────

    pub fn get(&self, op: &OutPoint) -> io::Result<Option<Coin>> {
        let ideal = self.ideal_slot(op);
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
            if bucket.matches(op) {
                return Ok(Some(bucket.to_coin()));
            }
            dist += 1;
            slot = (slot + 1) & self.mask;
            if slot == ideal {
                return Ok(None);
            }
        }
    }

    pub fn insert(&mut self, op: &OutPoint, coin: &Coin) -> io::Result<()> {
        if self.needs_resize() {
            self.resize()?;
        }

        let ideal = self.ideal_slot(op);
        let mut current = Bucket::from_outpoint_and_coin(op, coin, 1);
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
        }
    }

    pub fn remove(&mut self, op: &OutPoint) -> io::Result<bool> {
        let ideal = self.ideal_slot(op);
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
                    let coin = bucket.to_coin();
                    new_table.insert(&op, &coin)?;
                }
            }
            new_table.flush_header()?;
        }
        std::fs::rename(&new_path, &self.path)?;
        let file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        self.file = file;
        self.slot_count = new_count;
        self.mask = new_count - 1;
        Ok(())
    }

    pub fn len(&self) -> u64 {
        self.live_count
    }

    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    pub fn sync(&self) -> io::Result<()> {
        self.flush_header()?;
        self.file.sync_all()
    }

    /// Batched UTXO lookup using io_uring.
    ///
    /// Submits one `pread` per outpoint for the ideal bucket, waits for all
    /// completions, then submits follow-up reads for Robin Hood probe chains.
    /// Typically completes in 1–2 rounds.
    ///
    /// `arena` must be pre-sized to `ops.len() * BUCKET_SIZE`. Results are
    /// written into `results` in the same order as `ops`.
    #[allow(clippy::too_many_lines)]
    pub fn get_batch_uring(
        &self,
        ops: &[OutPoint],
        results: &mut Vec<Option<Coin>>,
        ring: &mut ququmatz::IoUring,
        arena: &mut Vec<u8>,
    ) -> io::Result<()> {
        let n = ops.len();
        if n == 0 {
            return Ok(());
        }

        results.clear();
        results.resize(n, None);

        let fd = self.fd();

        // Per-outpoint state: current probe slot and distance.
        let mut slots: Vec<u64> = ops.iter().map(|op| self.ideal_slot(op)).collect();
        let mut dists: Vec<u8> = vec![1; n];
        let mut resolved: Vec<bool> = vec![false; n];
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
                let offset = self.slot_offset(slots[i]);
                let buf_start = i * BUCKET_SIZE;
                let sqe = unsafe {
                    ququmatz::Sqe::read_ptr(
                        fd,
                        arena[buf_start..].as_mut_ptr(),
                        BUCKET_SIZE as u32,
                        offset,
                    )
                }
                .user_data(i as u64);
                ring.push(sqe).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e:?}")))?;
                submitted += 1;
            }

            if submitted == 0 {
                break;
            }

            ring.submit_and_wait(submitted)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e:?}")))?;

            // Process completions.
            let mut need_reprobe = false;
            while let Some(cqe) = ring.complete() {
                let i = cqe.user_data as usize;
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
                    results[i] = Some(bucket.to_coin());
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
    Spent,
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
            .map_err(|e| EngineError::Io(io::Error::new(io::ErrorKind::Other, format!("{e:?}"))))?;

        let mut engine = Self {
            data_dir: data_dir.to_owned(),
            table,
            ring,
            write_buffer: PrehashedMap::with_hasher(BuildPrehashedHasher),
            pending_disk_deletes: Vec::new(),
            read_arena: Vec::new(),
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
        for (op, entry) in self.write_buffer.drain() {
            if let BufferEntry::Live(coin) = entry {
                self.table.insert(&op, &coin).map_err(io_err)?;
            }
        }
        for op in self.pending_disk_deletes.drain(..) {
            self.table.remove(&op).map_err(io_err)?;
        }
        Ok(())
    }
}

impl Engine for RobinHoodEngine {
    fn read_coin(&mut self, op: &OutPoint) -> Result<Option<Coin>, EngineError> {
        match self.write_buffer.get(op) {
            Some(BufferEntry::Live(coin)) => return Ok(Some(coin.clone())),
            Some(BufferEntry::Spent) => return Ok(None),
            None => {}
        }
        self.table.get(op).map_err(io_err)
    }

    fn read_coins(
        &mut self,
        ops: &[OutPoint],
        out: &mut Vec<LookupResult>,
    ) -> Result<(), EngineError> {
        let mut disk_ops: Vec<(usize, OutPoint)> = Vec::new();
        let mut results: Vec<Option<Coin>> = Vec::with_capacity(ops.len());

        for (i, &op) in ops.iter().enumerate() {
            match self.write_buffer.get(&op) {
                Some(BufferEntry::Live(coin)) => results.push(Some(coin.clone())),
                Some(BufferEntry::Spent) => results.push(None),
                None => {
                    disk_ops.push((i, op));
                    results.push(None);
                }
            }
        }

        if !disk_ops.is_empty() {
            let disk_outpoints: Vec<OutPoint> = disk_ops.iter().map(|(_, op)| *op).collect();
            let mut disk_coins: Vec<Option<Coin>> = Vec::new();
            let ring = &mut self.ring;
            let arena = &mut self.read_arena;
            self.table
                .get_batch_uring(&disk_outpoints, &mut disk_coins, ring, arena)
                .map_err(io_err)?;
            for (j, &(i, _)) in disk_ops.iter().enumerate() {
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
        for nc in &req.created {
            let coin = Coin {
                value: nc.value,
                script_pubkey: ScriptPubKey::from_slice(nc.script_pubkey),
                height: nc.height,
                coinbase: nc.coinbase,
            };
            self.write_buffer.insert(nc.outpoint, BufferEntry::Live(coin));
        }
        for op in &req.spent {
            match self.write_buffer.remove(op) {
                Some(BufferEntry::Live(_)) => {
                    // Created and spent within the buffer window — elided.
                }
                Some(BufferEntry::Spent) | None => {
                    // Coin is on disk — schedule deletion for next flush.
                    self.pending_disk_deletes.push(*op);
                }
            }
        }

        if self.write_buffer.len() + self.pending_disk_deletes.len() > WRITE_BUFFER_FLUSH_THRESHOLD {
            self.flush_write_buffer()?;
        }

        debug_assert_eq!(
            req.header.prev_block, req.prev_hash,
            "CommitBlockReq.header.prev_block must match req.prev_hash"
        );
        self.header_write_len = write_header_record(
            &mut self.header_file,
            req.height,
            &req.header,
            req.hash,
            self.header_write_len,
        )?;
        self.header_index.insert(req.hash, req.height.0);

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
        self.read_header_at(BlockHeight(height_u32))
    }

    fn read_header_at(&self, h: BlockHeight) -> Result<Option<BlockHeader>, EngineError> {
        let offset = u64::from(h.0) * HEADER_RECORD_SIZE as u64;
        let mut f = File::open(self.header_path()).map_err(io_err)?;
        let file_len = f.seek(SeekFrom::End(0)).map_err(io_err)?;
        if offset + HEADER_RECORD_SIZE as u64 > file_len {
            return Ok(None);
        }
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        let header = read_header_record(&mut f)?;
        Ok(Some(header))
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
        self.header_index.insert(hash, h.0);
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
            let h = self.tip.height.0.to_le_bytes();
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

        // Rebuild header index from headers.bin.
        self.header_index.clear();
        let header_path = self.header_path();
        if let Ok(mut hf) = File::open(&header_path) {
            let file_len = hf.seek(SeekFrom::End(0)).map_err(io_err)?;
            hf.seek(SeekFrom::Start(0)).map_err(io_err)?;
            let record_count = file_len / HEADER_RECORD_SIZE as u64;
            for i in 0..record_count {
                let mut buf = [0u8; HEADER_RECORD_SIZE];
                if hf.read_exact(&mut buf).is_err() {
                    break;
                }
                let hash = Hash256(buf.read_hash(0));
                let height = buf.read_u32_le(112);
                if height <= checkpoint_height || checkpoint_height == 0 {
                    self.header_index.insert(hash, height);
                    if i + 1 == record_count {
                        self.tip = TipInfo {
                            hash,
                            height: BlockHeight(height),
                        };
                    }
                }
            }
            self.header_write_len = record_count * HEADER_RECORD_SIZE as u64;
        }

        // UTXO table is self-recovering — its state is persisted in the file header.
        // Nothing to rebuild.

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
    let offset = u64::from(h.0) * HEADER_RECORD_SIZE as u64;
    let new_len = if offset > file_len {
        f.set_len(offset).map_err(io_err)?;
        offset
    } else {
        file_len
    };
    f.write_all(&hash.0).map_err(io_err)?;
    f.write_all(&header.version.to_le_bytes()).map_err(io_err)?;
    f.write_all(&header.prev_block.0).map_err(io_err)?;
    f.write_all(&header.merkle_root.0).map_err(io_err)?;
    f.write_all(&header.time.to_le_bytes()).map_err(io_err)?;
    f.write_all(&header.bits.0.to_le_bytes()).map_err(io_err)?;
    f.write_all(&header.nonce.to_le_bytes()).map_err(io_err)?;
    f.write_all(&h.0.to_le_bytes()).map_err(io_err)?;
    Ok(new_len + HEADER_RECORD_SIZE as u64)
}

#[allow(clippy::cast_possible_wrap)]
fn read_header_record(f: &mut File) -> Result<BlockHeader, EngineError> {
    let mut buf = [0u8; HEADER_RECORD_SIZE];
    f.read_exact(&mut buf).map_err(io_err)?;
    Ok(BlockHeader {
        version: buf.read_u32_le(32) as i32,
        prev_block: Hash256(buf.read_hash(36)),
        merkle_root: Hash256(buf.read_hash(68)),
        time: buf.read_u32_le(100),
        bits: CompactBits(buf.read_u32_le(104)),
        nonce: buf.read_u32_le(108),
    })
}

fn io_err(e: impl Into<io::Error>) -> EngineError {
    EngineError::Io(e.into())
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    fn make_coin(value: u64, spk: &[u8]) -> Coin {
        Coin {
            value: Satoshi::new(value),
            script_pubkey: ScriptPubKey::from_slice(spk),
            height: BlockHeight(100),
            coinbase: false,
        }
    }

    fn make_op(i: u8) -> OutPoint {
        OutPoint {
            txid: Hash256([i; 32]),
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
            ("p2pkh", &[0x76, 0xa9, 0x14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
                         11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 0x88, 0xac]),
            ("p2sh", &[0xa9, 0x14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
                        11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 0x87]),
            ("p2wpkh", &[0x00, 0x14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
                          11, 12, 13, 14, 15, 16, 17, 18, 19, 20]),
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
            let op = make_op(i as u8);
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
        table.get_batch_uring(&ops, &mut results, &mut ring, &mut arena).unwrap();

        assert_eq!(results.len(), count as usize);
        for (i, result) in results.iter().enumerate() {
            let coin = result.as_ref().unwrap();
            assert_eq!(coin.value.to_u64(), i as u64 * 100);
        }

        // Also test miss: outpoints not in the table.
        let miss_ops: Vec<OutPoint> = (200..210u8).map(make_op).collect();
        let mut miss_results = Vec::new();
        table.get_batch_uring(&miss_ops, &mut miss_results, &mut ring, &mut arena).unwrap();
        assert!(miss_results.iter().all(Option::is_none));
    }

    #[test]
    fn write_buffer_elides_create_then_spend() {
        let dir = TmpFile::new("wb-elide");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let op = OutPoint { txid: Hash256([10u8; 32]), vout: 0 };

        // Create a coin — goes into write buffer, not disk.
        let req1 = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight(1),
            hash: Hash256([11u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: vec![mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(1000),
                script_pubkey: &[0x51],
                coinbase: false,
                height: BlockHeight(1),
            }],
            spent: vec![],
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
            height: BlockHeight(2),
            hash: Hash256([12u8; 32]),
            prev_hash: Hash256([11u8; 32]),
            header: BlockHeader { prev_block: Hash256([11u8; 32]), ..make_header(0) },
            created: vec![],
            spent: vec![op],
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

        let op = OutPoint { txid: Hash256([20u8; 32]), vout: 0 };
        let req = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight(1),
            hash: Hash256([21u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: vec![mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(2000),
                script_pubkey: &[0x51],
                coinbase: false,
                height: BlockHeight(1),
            }],
            spent: vec![],
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

    fn tmp_dir(name: &str) -> TmpFile {
        TmpFile::new(name)
    }

    fn make_header(time: u32) -> BlockHeader {
        BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time,
            bits: CompactBits(0x1d00_ffff),
            nonce: 0,
        }
    }

    #[test]
    fn engine_write_and_read_coin() {
        let dir = TmpFile::new("eng-coin");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();

        let op = OutPoint { txid: Hash256([1u8; 32]), vout: 0 };
        let req = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight(1),
            hash: Hash256([2u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: vec![mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(5_000_000_000),
                script_pubkey: &[0x51],
                coinbase: true,
                height: BlockHeight(1),
            }],
            spent: vec![],
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

        let op = OutPoint { txid: Hash256([3u8; 32]), vout: 0 };
        let req1 = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight(10),
            hash: Hash256([4u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: vec![mantle_consensus::chain_view::NewCoin {
                outpoint: op,
                value: Satoshi::new(1000),
                script_pubkey: &[],
                coinbase: false,
                height: BlockHeight(10),
            }],
            spent: vec![],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req1).unwrap();
        assert!(eng.read_coin(&op).unwrap().is_some());

        let req2 = CommitBlockReq {
            token: crate::types::ReqToken(2),
            height: BlockHeight(11),
            hash: Hash256([5u8; 32]),
            prev_hash: Hash256([4u8; 32]),
            header: BlockHeader {
                prev_block: Hash256([4u8; 32]),
                ..make_header(0)
            },
            created: vec![],
            spent: vec![op],
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

        let h = BlockHeader {
            version: 0x2000_0007,
            prev_block: Hash256([0x11; 32]),
            merkle_root: Hash256([0x22; 32]),
            time: 12_345,
            bits: CompactBits(0x1d00_ffff),
            nonce: 0xdead_beef,
        };
        eng.write_header(BlockHeight(0), &h).unwrap();
        let got = eng.read_header_at(BlockHeight(0)).unwrap().unwrap();
        assert_eq!(got.version, h.version);
        assert_eq!(got.prev_block, h.prev_block);
        assert_eq!(got.merkle_root, h.merkle_root);
        assert_eq!(got.time, h.time);
        assert_eq!(got.bits.0, h.bits.0);
        assert_eq!(got.nonce, h.nonce);
    }
}
