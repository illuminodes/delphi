//! Disk-backed arena engine: compact in-memory `TxHandle` index + file-backed
//! coin storage with `io_uring` batch I/O and free-list reclamation.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::hash::ByteSliceExt;
use mantle::primitives::{
    BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, ScriptPubKey,
};
use mantle_consensus::chain_view::{Coin, TipInfo};
use mantle_utxo_set::disk_arena::{DiskArena, DiskTxHandle};

use super::{Engine, EngineError};
use crate::types::{CommitBlockReq, Durability, LookupResult};

const HEADER_RECORD_SIZE: usize = 116;
const WRITE_BUFFER_CAP: usize = 4096;

/// In-memory write buffer entry. Coins stay here until flushed to the
/// disk arena. Short-lived coins (created and spent before flush) are
/// elided entirely — never touch disk.
struct BufferedCoin {
    value: Satoshi,
    script_pubkey: ScriptPubKey,
    height: BlockHeight,
    coinbase: bool,
}

pub struct DiskArenaEngine {
    data_dir: PathBuf,
    arena: DiskArena,
    #[allow(dead_code)]
    ring: ququmatz::IoUring,
    txs: PrehashedMap<Hash256, DiskTxHandle>,

    // Write buffer: recent coins not yet on disk.
    write_buf: PrehashedMap<OutPoint, BufferedCoin>,
    pending_spends: Vec<OutPoint>,

    // Scratch for uring I/O.
    #[allow(dead_code)]
    io_buf: Vec<u8>,

    header_file: File,
    header_write_len: u64,
    header_index: PrehashedMap<Hash256, u32>,

    checkpoint_file: File,
    tip: TipInfo,
}

impl DiskArenaEngine {
    /// # Errors
    /// Returns `EngineError` if any file I/O fails.
    pub fn open(data_dir: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(data_dir).map_err(io_err)?;

        let arena_path = data_dir.join("coins.bin");
        let header_path = data_dir.join("headers.bin");
        let checkpoint_path = data_dir.join("checkpoint.bin");

        let arena = DiskArena::open(&arena_path).map_err(io_err)?;

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
            ring,
            txs: PrehashedMap::with_hasher(BuildPrehashedHasher),
            write_buf: PrehashedMap::with_capacity_and_hasher(WRITE_BUFFER_CAP, BuildPrehashedHasher),
            pending_spends: Vec::new(),
            io_buf: Vec::new(),
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
        if self.write_buf.is_empty() && self.pending_spends.is_empty() {
            return Ok(());
        }

        // Group buffered coins by txid for batch-writing spans.
        let mut tx_groups: PrehashedMap<Hash256, Vec<(u32, BufferedCoin)>> =
            PrehashedMap::default();
        for (op, coin) in self.write_buf.drain() {
            tx_groups.entry(op.txid).or_default().push((op.vout, coin));
        }

        for (txid, mut outputs) in tx_groups {
            outputs.sort_by_key(|(vout, _)| *vout);
            let max_vout = outputs.last().map_or(0, |(v, _)| *v);
            #[allow(clippy::cast_possible_truncation)]
            let count = max_vout as u16 + 1;
            let span = self.arena.alloc(count);

            let mut live = 0u64;
            let mut height = BlockHeight::ZERO;
            let mut coinbase = false;

            for (vout, coin) in &outputs {
                if *vout < 64 { live |= 1 << vout; }
                height = coin.height;
                coinbase = coin.coinbase;
                let ce = mantle_utxo_set::CoinEntry {
                    value: coin.value,
                    script_pubkey: coin.script_pubkey.clone(),
                };
                self.arena
                    .write_coin(span.base + vout, &ce, coin.height, coin.coinbase)
                    .map_err(io_err)?;
            }

            self.txs.insert(txid, DiskTxHandle {
                span,
                height,
                live,
                coinbase,
            });
        }

        // Process spends.
        for op in self.pending_spends.drain(..) {
            if let Some(handle) = self.txs.get_mut(&op.txid) {
                handle.mark_spent(op.vout);
                if handle.is_fully_spent() {
                    let span = handle.span;
                    self.txs.remove(&op.txid);
                    self.arena.free(span);
                }
            }
        }

        Ok(())
    }
}

impl Engine for DiskArenaEngine {
    fn read_coin(&mut self, op: &OutPoint) -> Result<Option<Coin>, EngineError> {
        // Check write buffer first.
        if let Some(bc) = self.write_buf.get(op) {
            return Ok(Some(Coin {
                value: bc.value,
                script_pubkey: bc.script_pubkey.clone(),
                height: bc.height,
                coinbase: bc.coinbase,
            }));
        }

        // Check on-disk index.
        let Some(handle) = self.txs.get(&op.txid) else {
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
                // Coin is on disk — mark for spend on next flush.
                self.pending_spends.push(*op);
            }
        }

        // Flush if buffer is getting large.
        if self.write_buf.len() >= WRITE_BUFFER_CAP {
            self.flush_write_buffer()?;
        }

        // Header.
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

        // Note: the coin arena and tx index are not recovered from disk
        // in this implementation. A production version would need a
        // separate tx-index file or WAL to rebuild the PrehashedMap on
        // restart. For now, recovery only restores headers.
        Ok(self.tip.clone())
    }

    fn tip(&self) -> TipInfo {
        self.tip.clone()
    }
}

// ── Header I/O ───────────────────────────────────────────────────────────────

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
                .join(format!("mantle-disk-arena-eng-{}-{}", std::process::id(), ns));
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
        let mut eng = DiskArenaEngine::open(dir.path()).unwrap();
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

        // Should be in write buffer.
        let coin = eng.read_coin(&op).unwrap().unwrap();
        assert_eq!(coin.value.to_u64(), 5_000_000_000);
        assert!(coin.coinbase);
    }

    #[test]
    fn flush_moves_to_disk() {
        let dir = TmpDir::new();
        let mut eng = DiskArenaEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
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

        eng.flush_write_buffer().unwrap();

        // Buffer should be empty, but coin readable from disk.
        assert!(eng.write_buf.is_empty());
        let coin = eng.read_coin(&op).unwrap().unwrap();
        assert_eq!(coin.value.to_u64(), 1000);
    }

    #[test]
    fn spend_elides_buffered_coin() {
        let dir = TmpDir::new();
        let mut eng = DiskArenaEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(1000),
            script_pubkey: &[0x51],
            coinbase: false,
            height: BlockHeight::new(1),
        };
        // Create.
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

        // Spend in next block (still in buffer).
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

        // Coin should be gone (elided in buffer, never hit disk).
        assert!(eng.read_coin(&op).unwrap().is_none());
        assert!(eng.write_buf.is_empty());
        assert!(eng.pending_spends.is_empty());
    }

    #[test]
    fn spend_on_disk_after_flush() {
        let dir = TmpDir::new();
        let mut eng = DiskArenaEngine::open(dir.path()).unwrap();
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

        // Now spend from disk.
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
        // Span should have been freed.
        assert!(eng.arena.free_list_len() > 0);
    }
}
