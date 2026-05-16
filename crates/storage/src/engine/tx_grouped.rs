use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::hash::ByteSliceExt;
use mantle::primitives::{
    BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi,
};
use mantle_consensus::chain_view::{Coin, NewCoin, TipInfo};
use mantle_utxo_set::{ConnectScratch, UtxoSet};

use super::{Engine, EngineError};
use crate::types::{CommitBlockReq, Durability, LookupResult};

const HEADER_RECORD_SIZE: usize = 116;
const WRITE_BUF_BYTES: usize = 256 * 1024;

pub struct TxGroupedEngine {
    data_dir: PathBuf,
    utxo_set: UtxoSet,
    scratch: ConnectScratch,

    wal_file: BufWriter<File>,
    wal_write_pos: u64,

    header_file: File,
    header_write_len: u64,
    header_index: PrehashedMap<Hash256, u32>,

    checkpoint_file: File,
    tip: TipInfo,
}

impl TxGroupedEngine {
    /// # Errors
    /// Returns `EngineError` if any file I/O fails during open or recovery.
    pub fn open(data_dir: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(data_dir).map_err(io_err)?;

        let wal_path = data_dir.join("utxo_wal.bin");
        let header_path = data_dir.join("headers.bin");
        let checkpoint_path = data_dir.join("checkpoint.bin");

        let wal_file = BufWriter::with_capacity(WRITE_BUF_BYTES, open_append(&wal_path)?);
        let header_file = open_append(&header_path)?;
        let checkpoint_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&checkpoint_path)
            .map_err(io_err)?;

        let mut engine = Self {
            data_dir: data_dir.to_owned(),
            utxo_set: UtxoSet::new(),
            scratch: ConnectScratch::new(),
            wal_file,
            wal_write_pos: 0,
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

    fn wal_path(&self) -> PathBuf {
        self.data_dir.join("utxo_wal.bin")
    }
}

impl Engine for TxGroupedEngine {
    fn read_coin(&mut self, op: &OutPoint) -> Result<Option<Coin>, EngineError> {
        Ok(self.utxo_set.get(op))
    }

    fn read_coins(
        &mut self,
        ops: &[OutPoint],
        out: &mut Vec<LookupResult>,
    ) -> Result<(), EngineError> {
        for &op in ops {
            out.push(LookupResult {
                outpoint: op,
                coin: self.utxo_set.get(&op),
            });
        }
        Ok(())
    }

    fn write_block(&mut self, req: &CommitBlockReq) -> Result<(), EngineError> {
        for nc in req.created {
            write_wal_create(&mut self.wal_file, nc)?;
            let spk_len = u64::try_from(nc.script_pubkey.len())
                .map_err(|_| EngineError::ScriptPubKeyTooLarge)?;
            self.wal_write_pos += 36 + 1 + 8 + 4 + 1 + 4 + spk_len;
        }
        for op in req.spent {
            write_wal_spend(&mut self.wal_file, op)?;
            self.wal_write_pos += 37;
        }

        let undo = self
            .utxo_set
            .connect_block_with(&mut self.scratch, req.height, req.created, req.spent)
            .map_err(|e| EngineError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
        self.scratch.reclaim(undo);

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
        if d == Durability::Synced {
            self.wal_file.flush().map_err(io_err)?;
            self.wal_file.get_ref().sync_all().map_err(io_err)?;
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
        let checkpoint_height = if n == 4 {
            u32::from_le_bytes(cp_buf)
        } else {
            0
        };

        self.header_index.clear();
        let header_path = self.header_path();
        let mut hf = File::open(&header_path).map_err(io_err)?;
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
            if height <= checkpoint_height || checkpoint_height == 0 {
                self.header_index.insert(hash, height);
                if i + 1 == record_count {
                    self.tip = TipInfo {
                        hash,
                        height: BlockHeight::new(height),
                    };
                }
            }
        }

        self.utxo_set = UtxoSet::new();
        self.wal_write_pos = 0;
        let wal_path = self.wal_path();
        if let Ok(mut wf) = File::open(&wal_path) {
            let mut offset: u64 = 0;
            loop {
                match read_wal_entry(&mut wf, offset) {
                    Ok(Some(WalEntry::Create(nc, record_end))) => {
                        let _ = self.utxo_set.connect_block(
                            nc.height,
                            &[nc],
                            &[],
                        );
                        offset = record_end;
                    }
                    Ok(Some(WalEntry::Spend(op, record_end))) => {
                        let _ = self.utxo_set.connect_block(
                            self.tip.height,
                            &[],
                            &[op],
                        );
                        offset = record_end;
                    }
                    Ok(None) | Err(_) => {
                        self.wal_write_pos = offset;
                        break;
                    }
                }
            }
        }

        self.header_write_len = record_count * HEADER_RECORD_SIZE as u64;
        Ok(self.tip.clone())
    }

    fn tip(&self) -> TipInfo {
        self.tip.clone()
    }
}

// ── WAL record format ────────────────────────────────────────────────────────
//
// Identical wire format to FlatEngine's UTXO log so recovery is compatible
// with the same tooling.
//
// Create record:
//   outpoint (36) | tag=1 (1) | value (8) | height (4) | coinbase (1) | spk_len (4) | spk
// Spend record:
//   outpoint (36) | tag=0 (1)

fn write_wal_create<W: Write>(f: &mut W, nc: &NewCoin) -> Result<(), EngineError> {
    f.write_all(nc.outpoint.txid.as_bytes()).map_err(io_err)?;
    f.write_all(&nc.outpoint.vout.to_le_bytes())
        .map_err(io_err)?;
    f.write_all(&[1u8]).map_err(io_err)?;
    f.write_all(&nc.value.to_le_bytes()).map_err(io_err)?;
    f.write_all(&nc.height.to_u32().to_le_bytes())
        .map_err(io_err)?;
    f.write_all(&[u8::from(nc.coinbase)]).map_err(io_err)?;
    let spk_len = u32::try_from(nc.script_pubkey.len())
        .map_err(|_| EngineError::ScriptPubKeyTooLarge)?;
    f.write_all(&spk_len.to_le_bytes()).map_err(io_err)?;
    f.write_all(nc.script_pubkey).map_err(io_err)?;
    Ok(())
}

fn write_wal_spend<W: Write>(f: &mut W, op: &OutPoint) -> Result<(), EngineError> {
    f.write_all(op.txid.as_bytes()).map_err(io_err)?;
    f.write_all(&op.vout.to_le_bytes()).map_err(io_err)?;
    f.write_all(&[0u8]).map_err(io_err)?;
    Ok(())
}

enum WalEntry<'a> {
    Create(NewCoin<'a>, u64),
    Spend(OutPoint, u64),
}

fn read_wal_entry(f: &mut File, offset: u64) -> Result<Option<WalEntry<'static>>, EngineError> {
    f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
    let mut op_buf = [0u8; 36];
    if f.read_exact(&mut op_buf).is_err() {
        return Ok(None);
    }
    let txid = Hash256::from_bytes(op_buf.read_hash(0));
    let vout = op_buf.read_u32_le(32);
    let outpoint = OutPoint { txid, vout };

    let mut tag = [0u8; 1];
    f.read_exact(&mut tag).map_err(io_err)?;
    if tag[0] == 0 {
        return Ok(Some(WalEntry::Spend(outpoint, offset + 37)));
    }

    let mut buf8 = [0u8; 8];
    f.read_exact(&mut buf8).map_err(io_err)?;
    let value = u64::from_le_bytes(buf8);
    let mut buf4 = [0u8; 4];
    f.read_exact(&mut buf4).map_err(io_err)?;
    let height = u32::from_le_bytes(buf4);
    let mut tag1 = [0u8; 1];
    f.read_exact(&mut tag1).map_err(io_err)?;
    let coinbase = tag1[0] != 0;
    f.read_exact(&mut buf4).map_err(io_err)?;
    let spk_len =
        usize::try_from(u32::from_le_bytes(buf4)).map_err(|_| EngineError::ScriptLenOverflow)?;

    // Leak the script bytes so they live as 'static for the NewCoin.
    // This only runs during recovery (once at startup).
    let mut spk = vec![0u8; spk_len];
    f.read_exact(&mut spk).map_err(io_err)?;
    let spk_static: &'static [u8] = Vec::leak(spk);

    let record_end = offset
        + 36
        + 1
        + 8
        + 4
        + 1
        + 4
        + u64::try_from(spk_len).map_err(|_| EngineError::ScriptLenOverflow)?;

    Ok(Some(WalEntry::Create(
        NewCoin {
            outpoint,
            value: Satoshi::new(value),
            script_pubkey: spk_static,
            coinbase,
            height: BlockHeight::new(height),
        },
        record_end,
    )))
}

// ── Header I/O (shared format with FlatEngine) ──────────────────────────────

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

fn open_append(path: &Path) -> Result<File, EngineError> {
    OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .map_err(io_err)
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
            let ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos();
            let path = std::env::temp_dir()
                .join(format!("mantle-txg-test-{}-{}", std::process::id(), ns));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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

    fn make_outpoint(i: u32) -> OutPoint {
        let mut txid_bytes = [0u8; 32];
        txid_bytes[..4].copy_from_slice(&i.to_le_bytes());
        OutPoint {
            txid: Hash256::from_bytes(txid_bytes),
            vout: 0,
        }
    }

    fn make_block_hash(i: u32) -> Hash256 {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(&(i + 10_000).to_le_bytes());
        Hash256::from_bytes(bytes)
    }

    #[test]
    fn write_and_read_coin() {
        let dir = TmpDir::new();
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(5_000_000_000),
            script_pubkey: &[0xac],
            coinbase: true,
            height: BlockHeight::new(1),
        };
        let req = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: make_block_hash(1),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[nc],
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
    fn spend_removes_coin() {
        let dir = TmpDir::new();
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        let op = make_outpoint(1);
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(1000),
            script_pubkey: &[],
            coinbase: false,
            height: BlockHeight::new(10),
        };
        let req1 = CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(10),
            hash: make_block_hash(10),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[nc],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req1).unwrap();
        assert!(eng.read_coin(&op).unwrap().is_some());

        let mut h2 = make_header(1);
        h2.set_prev_block(make_block_hash(10));
        let req2 = CommitBlockReq {
            token: crate::types::ReqToken(2),
            height: BlockHeight::new(11),
            hash: make_block_hash(11),
            prev_hash: make_block_hash(10),
            header: h2,
            created: &[],
            spent: &[op],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req2).unwrap();
        assert!(eng.read_coin(&op).unwrap().is_none());
    }

    #[test]
    fn write_and_read_header() {
        let dir = TmpDir::new();
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
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
    fn recover_rebuilds_utxo_set() {
        let dir = TmpDir::new();
        let op = make_outpoint(1);
        {
            let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
            let nc = NewCoin {
                outpoint: op,
                value: Satoshi::new(999),
                script_pubkey: &[0x51],
                coinbase: false,
                height: BlockHeight::new(5),
            };
            let req = CommitBlockReq {
                token: crate::types::ReqToken(1),
                height: BlockHeight::new(5),
                hash: make_block_hash(5),
                prev_hash: Hash256::ZERO,
                header: make_header(0),
                created: &[nc],
                spent: &[],
                block_bytes: None,
                durability: Durability::Memory,
            };
            eng.write_block(&req).unwrap();
            eng.flush(Durability::Synced).unwrap();
        }
        let mut eng2 = TxGroupedEngine::open(dir.path()).unwrap();
        let coin = eng2.read_coin(&op).unwrap().unwrap();
        assert_eq!(coin.value.to_u64(), 999);
    }
}
