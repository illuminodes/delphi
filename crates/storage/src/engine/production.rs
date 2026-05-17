//! Production UTXO engine — fully disk-backed with bloom filter and LRU cache.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::hash::ByteSliceExt;
use mantle::primitives::{
    BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, ScriptPubKey,
};
use mantle_bloom::BloomFilter;
use mantle_consensus::chain_view::{Coin, TipInfo};
use mantle_lru::LruCache;
use mantle_utxo_set::disk_arena::{DiskArena, DiskSpan, DiskTxHandle};
use mantle_utxo_set::tx_index::TxIndex;

use super::{Engine, EngineError};
use crate::types::{CommitBlockReq, Durability, LookupResult};

const HEADER_RECORD_SIZE: usize = 116;
const WRITE_BUFFER_CAP: usize = 100_000;
const LRU_CAPACITY: usize = 12_000_000;
const BLOOM_EXPECTED_ITEMS: usize = 85_000_000;
const BLOOM_FP_RATE: f64 = 0.001;
const TX_INDEX_INITIAL_SLOTS: u64 = 1 << 20;

struct BufferedCoin {
    value: Satoshi,
    script_pubkey: ScriptPubKey,
    height: BlockHeight,
    coinbase: bool,
}

pub struct ProductionEngine {
    data_dir: PathBuf,
    arena: DiskArena,
    tx_index: TxIndex,
    bloom: BloomFilter,
    lru: LruCache<DiskTxHandle>,
    ring: ququmatz::IoUring,

    write_buf: PrehashedMap<OutPoint, BufferedCoin>,
    pending_spends: Vec<OutPoint>,

    io_buf: Vec<u8>,
    write_io_buf: Vec<u8>,

    header_file: File,
    header_write_len: u64,
    header_index: PrehashedMap<Hash256, u32>,

    checkpoint_file: File,
    tip: TipInfo,
}

impl ProductionEngine {
    /// # Errors
    /// Returns `EngineError` if any file I/O fails.
    pub fn open(data_dir: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(data_dir).map_err(io_err)?;

        let arena_path = data_dir.join("coins.bin");
        let tx_index_path = data_dir.join("tx_index.bin");
        let header_path = data_dir.join("headers.bin");
        let checkpoint_path = data_dir.join("checkpoint.bin");

        let arena = DiskArena::open(&arena_path).map_err(io_err)?;
        let tx_index = TxIndex::open_with_capacity(&tx_index_path, TX_INDEX_INITIAL_SLOTS)
            .map_err(io_err)?;

        let bloom = BloomFilter::with_fp_rate(BLOOM_EXPECTED_ITEMS, BLOOM_FP_RATE);

        let blank_handle = DiskTxHandle {
            span: DiskSpan { base: 0, count: 0 },
            height: BlockHeight::ZERO,
            live: 0,
            coinbase: false,
        };
        let lru = LruCache::new(LRU_CAPACITY, blank_handle);

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

        let ring = ququmatz::IoUring::builder(256)
            .build()
            .map_err(|e| EngineError::Io(io::Error::other(format!("{e:?}"))))?;

        let mut engine = Self {
            data_dir: data_dir.to_owned(),
            arena,
            tx_index,
            bloom,
            lru,
            ring,
            write_buf: PrehashedMap::with_capacity_and_hasher(WRITE_BUFFER_CAP, BuildPrehashedHasher),
            pending_spends: Vec::new(),
            io_buf: Vec::new(),
            write_io_buf: Vec::new(),
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

    fn lookup_handle(&mut self, txid: &Hash256) -> Result<Option<DiskTxHandle>, EngineError> {
        if let Some(h) = self.lru.get(txid.as_bytes()) {
            return Ok(Some(h));
        }
        if !self.bloom.maybe_contains(txid.as_bytes()) {
            return Ok(None);
        }
        let handle = self.tx_index.get(txid).map_err(io_err)?;
        if let Some(h) = handle {
            self.lru.insert(*txid.as_bytes(), h);
        }
        Ok(handle)
    }

    #[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
    fn flush_write_buffer(&mut self) -> Result<(), EngineError> {
        if self.write_buf.is_empty() && self.pending_spends.is_empty() {
            return Ok(());
        }

        // Group buffered coins by txid.
        let mut tx_groups: PrehashedMap<Hash256, Vec<(u32, BufferedCoin)>> =
            PrehashedMap::default();
        for (op, coin) in self.write_buf.drain() {
            tx_groups.entry(op.txid).or_default().push((op.vout, coin));
        }

        #[allow(clippy::items_after_statements)]
        struct TxFlush {
            txid: Hash256,
            span: DiskSpan,
            outputs: Vec<(u32, BufferedCoin)>,
            live: u64,
            height: BlockHeight,
            coinbase: bool,
        }
        let mut flushes: Vec<TxFlush> = Vec::with_capacity(tx_groups.len());

        for (txid, mut outputs) in tx_groups {
            outputs.sort_by_key(|(vout, _)| *vout);
            let max_vout = outputs.last().map_or(0, |(v, _)| *v);
            let count = max_vout as u16 + 1;
            let span = self.arena.alloc(count);

            let mut live = 0u64;
            let mut height = BlockHeight::ZERO;
            let mut coinbase = false;
            for (vout, coin) in &outputs {
                if *vout < 64 { live |= 1 << vout; }
                height = coin.height;
                coinbase = coin.coinbase;
            }

            flushes.push(TxFlush { txid, span, outputs, live, height, coinbase });
        }

        flushes.sort_by_key(|f| f.span.base);

        // Encode coin data.
        self.io_buf.clear();
        for flush in &flushes {
            let span_bytes = usize::from(flush.span.count) * 56;
            let buf_offset = self.io_buf.len();
            self.io_buf.resize(buf_offset + span_bytes, 0u8);
            for (vout, coin) in &flush.outputs {
                let slot_start = buf_offset + (*vout as usize) * 56;
                let slot_buf: &mut [u8; 56] =
                    (&mut self.io_buf[slot_start..slot_start + 56]).try_into().unwrap();
                mantle_utxo_set::disk_arena::encode_coin_slot(
                    slot_buf, coin.value, coin.height, coin.coinbase,
                    coin.script_pubkey.as_slice(),
                );
            }
        }

        // Write coins to disk.
        if !flushes.is_empty() {
            use std::os::unix::fs::FileExt;

            let all_contiguous = flushes.windows(2).all(|w| {
                w[0].span.base + u32::from(w[0].span.count) == w[1].span.base
            });

            if all_contiguous {
                let start_offset = u64::from(flushes[0].span.base) * 56;
                self.arena.fd().write_all_at(&self.io_buf, start_offset)
                    .map_err(io_err)?;
            } else {
                use std::os::unix::io::AsRawFd;
                #[allow(clippy::cast_sign_loss)]
                let fd = ququmatz::types::RawFd::from_raw(self.arena.fd().as_raw_fd() as usize);
                let mut submitted = 0u32;
                let mut buf_offset = 0usize;

                for flush in &flushes {
                    let span_bytes = usize::from(flush.span.count) * 56;
                    let file_offset = u64::from(flush.span.base) * 56;
                    let sqe = unsafe {
                        ququmatz::Sqe::write_ptr(
                            fd,
                            self.io_buf[buf_offset..].as_ptr(),
                            span_bytes as u32,
                            file_offset,
                        )
                    }.user_data(u64::MAX);
                    self.ring.push(sqe)
                        .map_err(|e| EngineError::Io(io::Error::other(format!("{e:?}"))))?;
                    submitted += 1;
                    buf_offset += span_bytes;

                    if submitted >= 200 {
                        self.ring.submit_and_wait(submitted)
                            .map_err(|e| EngineError::Io(io::Error::other(format!("{e:?}"))))?;
                        while self.ring.complete().is_some() {}
                        submitted = 0;
                    }
                }

                if submitted > 0 {
                    self.ring.submit_and_wait(submitted)
                        .map_err(|e| EngineError::Io(io::Error::other(format!("{e:?}"))))?;
                    while self.ring.complete().is_some() {}
                }
            }
        }

        // Insert handles into tx_index, bloom, and LRU.
        let pairs: Vec<(Hash256, DiskTxHandle)> = flushes.iter().map(|f| {
            (f.txid, DiskTxHandle {
                span: f.span,
                height: f.height,
                live: f.live,
                coinbase: f.coinbase,
            })
        }).collect();

        self.tx_index.insert_batch_uring(
            &pairs,
            &mut self.ring,
            &mut self.io_buf,
            &mut self.write_io_buf,
        ).map_err(io_err)?;

        for (txid, handle) in &pairs {
            self.bloom.insert(txid.as_bytes());
            self.lru.insert(*txid.as_bytes(), *handle);
        }

        // Process spends.
        let spends: Vec<OutPoint> = self.pending_spends.drain(..).collect();
        for op in spends {
            let Some(mut handle) = self.lookup_handle(&op.txid)? else {
                continue;
            };
            handle.mark_spent(op.vout);
            if handle.is_fully_spent() {
                self.tx_index.remove(&op.txid).map_err(io_err)?;
                self.lru.remove(op.txid.as_bytes());
                self.arena.free(handle.span);
            } else {
                self.tx_index.update(&op.txid, &handle).map_err(io_err)?;
                self.lru.insert(*op.txid.as_bytes(), handle);
            }
        }

        Ok(())
    }
}

impl Engine for ProductionEngine {
    fn read_coin(&mut self, op: &OutPoint) -> Result<Option<Coin>, EngineError> {
        if let Some(bc) = self.write_buf.get(op) {
            return Ok(Some(Coin {
                value: bc.value,
                script_pubkey: bc.script_pubkey.clone(),
                height: bc.height,
                coinbase: bc.coinbase,
            }));
        }

        let Some(handle) = self.lookup_handle(&op.txid)? else {
            return Ok(None);
        };
        if !handle.is_unspent(op.vout) {
            return Ok(None);
        }
        self.arena.read_coin(handle.span.base + op.vout).map_err(io_err)
    }

    fn read_coins(
        &mut self,
        ops: &[OutPoint],
        out: &mut Vec<LookupResult>,
    ) -> Result<(), EngineError> {
        for &op in ops {
            let coin = self.read_coin(&op)?;
            out.push(LookupResult { outpoint: op, coin });
        }
        Ok(())
    }

    fn write_block(&mut self, req: &CommitBlockReq) -> Result<(), EngineError> {
        for nc in req.created {
            self.write_buf.insert(nc.outpoint, BufferedCoin {
                value: nc.value,
                script_pubkey: ScriptPubKey::from_slice(nc.script_pubkey),
                height: nc.height,
                coinbase: nc.coinbase,
            });
        }

        for op in req.spent {
            if self.write_buf.remove(op).is_none() {
                self.pending_spends.push(*op);
            }
        }

        if self.write_buf.len() >= WRITE_BUFFER_CAP {
            self.flush_write_buffer()?;
        }

        debug_assert_eq!(req.header.prev_block(), req.prev_hash);
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
        self.header_index.insert(hash, h.to_u32());
        Ok(())
    }

    fn flush(&mut self, d: Durability) -> Result<(), EngineError> {
        self.flush_write_buffer()?;
        if d == Durability::Synced {
            self.arena.fd().sync_all().map_err(io_err)?;
            self.tx_index.sync().map_err(io_err)?;
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
        let mut cp_buf = [0u8; 4];
        let n = self.checkpoint_file.read(&mut cp_buf).map_err(io_err)?;
        let _checkpoint_height = if n == 4 {
            u32::from_le_bytes(cp_buf)
        } else {
            0
        };

        // Rebuild header index.
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
                let hash = Hash256::from_bytes(buf.read_hash(0));
                let height = buf.read_u32_le(112);
                self.header_index.insert(hash, height);
                if i + 1 == record_count {
                    self.tip = TipInfo {
                        hash,
                        height: BlockHeight::new(height),
                    };
                }
            }
            self.header_write_len = record_count * HEADER_RECORD_SIZE as u64;
        }

        // Rebuild bloom filter from tx_index scan.
        self.bloom.clear();
        self.tx_index.for_each(|txid, handle| {
            self.bloom.insert(txid.as_bytes());
            self.lru.insert(*txid.as_bytes(), handle);
        }).map_err(io_err)?;

        Ok(self.tip.clone())
    }

    fn tip(&self) -> TipInfo {
        self.tip.clone()
    }
}

// ── Header I/O ──────────────────────────────────────────────────────────────

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
    f.write_all(&header.version().to_le_bytes()).map_err(io_err)?;
    f.write_all(header.prev_block().as_bytes()).map_err(io_err)?;
    f.write_all(header.merkle_root().as_bytes()).map_err(io_err)?;
    f.write_all(&header.time().to_le_bytes()).map_err(io_err)?;
    f.write_all(&header.bits().to_le_bytes()).map_err(io_err)?;
    f.write_all(&header.nonce().to_le_bytes()).map_err(io_err)?;
    f.write_all(&h.to_u32().to_le_bytes()).map_err(io_err)?;
    Ok(new_len + HEADER_RECORD_SIZE as u64)
}

fn read_header_record(f: &mut File) -> Result<BlockHeader, EngineError> {
    let mut buf = [0u8; HEADER_RECORD_SIZE];
    f.read_exact(&mut buf).map_err(io_err)?;
    Ok(parse_header_record(&buf))
}

#[allow(clippy::cast_possible_wrap)]
fn parse_header_record(buf: &[u8; HEADER_RECORD_SIZE]) -> BlockHeader {
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

fn io_err(e: impl Into<io::Error>) -> EngineError {
    EngineError::Io(e.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mantle::primitives::{BlockTime, BlockVersion, Nonce};
    use mantle_consensus::chain_view::NewCoin;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ns = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
            let path = std::env::temp_dir()
                .join(format!("mantle-prod-eng-{}-{}", std::process::id(), ns));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    fn make_header(time: u32) -> BlockHeader {
        BlockHeader::new(
            BlockVersion::new(1), Hash256::ZERO, Hash256::ZERO,
            BlockTime::new(time), CompactBits::new(0x1d00_ffff), Nonce::new(0),
        )
    }

    fn make_outpoint(i: u32) -> OutPoint {
        let mut b = [0u8; 32];
        b[..4].copy_from_slice(&i.to_le_bytes());
        OutPoint { txid: Hash256::from_bytes(b), vout: 0 }
    }

    fn make_block_hash(i: u32) -> Hash256 {
        let mut b = [0u8; 32];
        b[..4].copy_from_slice(&(i + 10_000).to_le_bytes());
        Hash256::from_bytes(b)
    }

    #[test]
    fn write_and_read_from_buffer() {
        let dir = TmpDir::new();
        let mut eng = ProductionEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(5_000_000_000),
            script_pubkey: &[0x00, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                             0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                             0x0f, 0x10, 0x11, 0x12, 0x13, 0x14],
            coinbase: true,
            height: BlockHeight::new(1),
        };
        eng.write_block(&CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: make_block_hash(1),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[nc],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        }).unwrap();

        let coin = eng.read_coin(&op).unwrap().unwrap();
        assert_eq!(coin.value.to_u64(), 5_000_000_000);
        assert!(coin.coinbase);
    }

    #[test]
    fn flush_moves_to_disk_and_recovers() {
        let dir = TmpDir::new();
        let op = make_outpoint(1);
        {
            let mut eng = ProductionEngine::open(dir.path()).unwrap();
            let nc = NewCoin {
                outpoint: op,
                value: Satoshi::new(1000),
                script_pubkey: &[0x51],
                coinbase: false,
                height: BlockHeight::new(5),
            };
            eng.write_block(&CommitBlockReq {
                token: crate::types::ReqToken(1),
                height: BlockHeight::new(5),
                hash: make_block_hash(5),
                prev_hash: Hash256::ZERO,
                header: make_header(0),
                created: &[nc],
                spent: &[],
                block_bytes: None,
                durability: Durability::Memory,
            }).unwrap();
            eng.flush(Durability::Synced).unwrap();
        }

        // Reopen — should recover from disk.
        let mut eng = ProductionEngine::open(dir.path()).unwrap();
        let coin = eng.read_coin(&op).unwrap().unwrap();
        assert_eq!(coin.value.to_u64(), 1000);
        assert_eq!(eng.tip().height, BlockHeight::new(5));
    }

    #[test]
    fn spend_elides_buffered_coin() {
        let dir = TmpDir::new();
        let mut eng = ProductionEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(1000),
            script_pubkey: &[0x51],
            coinbase: false,
            height: BlockHeight::new(1),
        };
        eng.write_block(&CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: make_block_hash(1),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[nc],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        }).unwrap();

        let mut h2 = make_header(1);
        h2.set_prev_block(make_block_hash(1));
        eng.write_block(&CommitBlockReq {
            token: crate::types::ReqToken(2),
            height: BlockHeight::new(2),
            hash: make_block_hash(2),
            prev_hash: make_block_hash(1),
            header: h2,
            created: &[],
            spent: &[op],
            block_bytes: None,
            durability: Durability::Memory,
        }).unwrap();

        assert!(eng.read_coin(&op).unwrap().is_none());
        assert!(eng.write_buf.is_empty());
        assert!(eng.pending_spends.is_empty());
    }

    #[test]
    fn spend_on_disk_after_flush() {
        let dir = TmpDir::new();
        let mut eng = ProductionEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(2000),
            script_pubkey: &[0x51],
            coinbase: false,
            height: BlockHeight::new(1),
        };
        eng.write_block(&CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: make_block_hash(1),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[nc],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        }).unwrap();
        eng.flush_write_buffer().unwrap();

        let mut h2 = make_header(1);
        h2.set_prev_block(make_block_hash(1));
        eng.write_block(&CommitBlockReq {
            token: crate::types::ReqToken(2),
            height: BlockHeight::new(2),
            hash: make_block_hash(2),
            prev_hash: make_block_hash(1),
            header: h2,
            created: &[],
            spent: &[op],
            block_bytes: None,
            durability: Durability::Memory,
        }).unwrap();
        eng.flush_write_buffer().unwrap();

        assert!(eng.read_coin(&op).unwrap().is_none());
        assert!(eng.arena.free_list_len() > 0);
    }

    #[test]
    fn bloom_filters_misses() {
        let dir = TmpDir::new();
        let mut eng = ProductionEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(1000),
            script_pubkey: &[0x51],
            coinbase: false,
            height: BlockHeight::new(1),
        };
        eng.write_block(&CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: make_block_hash(1),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[nc],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        }).unwrap();
        eng.flush_write_buffer().unwrap();

        // Existing coin found.
        assert!(eng.read_coin(&op).unwrap().is_some());

        // Non-existent txid — bloom should reject quickly.
        let missing = make_outpoint(9999);
        assert!(eng.read_coin(&missing).unwrap().is_none());
    }
}
