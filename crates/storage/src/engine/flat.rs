use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::hash::ByteSliceExt;
use mantle::primitives::{BlockHeader, BlockHeight, Hash256, OutPoint, Satoshi, ScriptPubKey};
use mantle_consensus::chain_view::{Coin, NewCoin, TipInfo};

use super::{Engine, EngineError};
use crate::types::{CommitBlockReq, Durability, LookupResult};

/// Fixed size of a header record:
/// hash(32) + version(4) + `prev_block`(32) + `merkle_root`(32) +
/// time(4) + bits(4) + nonce(4) + height(4) = 116 bytes.
///
/// The `version` field comes after `hash` (not interleaved with the
/// other header fields) so the offsets of `prev_block`, `merkle_root`,
/// time/bits/nonce stay aligned with the BIP144 header byte order, just
/// shifted by 4 bytes; the height tail at offset 112 stays put.
const HEADER_RECORD_SIZE: usize = 116;

/// User-space buffer in front of the append-only logs. Sized to absorb several
/// blocks' worth of UTXO and header writes so the kernel only sees a handful
/// of large `write()` syscalls instead of thousands of tiny ones.
const WRITE_BUF_BYTES: usize = 256 * 1024;

/// Phase-1 flat-file storage engine.
///
/// - `utxo.bin`: append-only log of `(OutPoint, Option<Coin>)` records.
///   In-memory `HashMap<OutPoint, file_offset>` is rebuilt on startup.
/// - `headers.bin`: fixed-width 116-byte records keyed by height.
/// - `checkpoint.bin`: last height for which `Durability::Synced` completed.
pub struct FlatEngine {
    data_dir: PathBuf,

    utxo_file: BufWriter<File>,
    utxo_read: File,
    utxo_write_pos: u64,
    utxo_flushed_pos: u64,
    utxo_index: PrehashedMap<OutPoint, u64>,

    // Headers are tiny (112 B each, one per block) and read by methods that
    // take `&self`. Keeping them unbuffered means reads always observe prior
    // writes without coordination and the syscall savings are not worth the
    // bookkeeping cost.
    header_file: File,
    header_write_len: u64,
    header_index: PrehashedMap<Hash256, u32>,

    checkpoint_file: File,
    tip: TipInfo,
}

impl FlatEngine {
    /// Open (or create) the engine at `data_dir`. Rebuilds the in-memory index
    /// from the log on startup, stopping at the last synced checkpoint.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` if any file I/O fails during open or recovery.
    pub fn open(data_dir: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(data_dir).map_err(|e| io_err(&e))?;

        let utxo_path = data_dir.join("utxo.bin");
        let header_path = data_dir.join("headers.bin");
        let checkpoint_path = data_dir.join("checkpoint.bin");

        let utxo_file = BufWriter::with_capacity(WRITE_BUF_BYTES, open_append(&utxo_path)?);
        let utxo_read = open_read(&utxo_path)?;
        let header_file = open_append(&header_path)?;
        let checkpoint_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&checkpoint_path)
            .map_err(|e| io_err(&e))?;

        let mut engine = Self {
            data_dir: data_dir.to_owned(),
            utxo_file,
            utxo_read,
            utxo_write_pos: 0,
            utxo_flushed_pos: 0,
            utxo_index: PrehashedMap::with_hasher(BuildPrehashedHasher),
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

    /// Drain the UTXO write buffer to the kernel so subsequent reads via
    /// `utxo_read` see all newly written records. Cheap when nothing is
    /// buffered.
    fn flush_utxo_writer(&mut self) -> Result<(), EngineError> {
        if self.utxo_flushed_pos != self.utxo_write_pos {
            self.utxo_file.flush().map_err(|e| io_err(&e))?;
            self.utxo_flushed_pos = self.utxo_write_pos;
        }
        Ok(())
    }

    fn utxo_path(&self) -> PathBuf {
        self.data_dir.join("utxo.bin")
    }

    fn header_path(&self) -> PathBuf {
        self.data_dir.join("headers.bin")
    }
}

impl Engine for FlatEngine {
    fn read_coin(&mut self, op: &OutPoint) -> Result<Option<Coin>, EngineError> {
        let Some(&offset) = self.utxo_index.get(op) else {
            return Ok(None);
        };
        // The index can point into the still-buffered tail of the writer; drain
        // it so the read handle observes the bytes.
        if offset >= self.utxo_flushed_pos {
            self.flush_utxo_writer()?;
        }
        self.utxo_read
            .seek(SeekFrom::Start(offset))
            .map_err(|e| io_err(&e))?;
        read_coin_record(&mut self.utxo_read)
    }

    fn read_coins(
        &mut self,
        ops: &[OutPoint],
        out: &mut Vec<LookupResult>,
    ) -> Result<(), EngineError> {
        // If any requested outpoint maps into the still-buffered tail, flush
        // once up-front instead of branching per-record.
        let needs_flush = ops
            .iter()
            .filter_map(|op| self.utxo_index.get(op).copied())
            .any(|off| off >= self.utxo_flushed_pos);
        if needs_flush {
            self.flush_utxo_writer()?;
        }
        for &op in ops {
            let coin = if let Some(&offset) = self.utxo_index.get(&op) {
                self.utxo_read
                    .seek(SeekFrom::Start(offset))
                    .map_err(|e| io_err(&e))?;
                read_coin_record(&mut self.utxo_read)?
            } else {
                None
            };
            out.push(LookupResult { outpoint: op, coin });
        }
        Ok(())
    }

    fn write_block(&mut self, req: &CommitBlockReq) -> Result<(), EngineError> {
        for nc in req.created {
            let record_start = self.utxo_write_pos;
            write_coin_record(&mut self.utxo_file, nc)?;
            let spk_len = u64::try_from(nc.script_pubkey.len())
                .map_err(|_| EngineError::ScriptPubKeyTooLarge)?;
            // 36 (outpoint) + 1 (tag) + 8 (value) + 4 (height) + 1 (coinbase) + 4 (spk_len) + spk
            self.utxo_write_pos += 36 + 1 + 8 + 4 + 1 + 4 + spk_len;
            // Index points past the 36-byte outpoint to where read_coin_record starts (tag byte).
            self.utxo_index.insert(nc.outpoint, record_start + 36);
        }
        for op in req.spent {
            write_spend_record(&mut self.utxo_file, op)?;
            // 36 (outpoint) + 1 (tag)
            self.utxo_write_pos += 37;
            self.utxo_index.remove(op);
        }

        // Persist the full header so `read_header*` round-trips. MTP and
        // retarget lookups during incremental replay need every field.
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
        let mut f = File::open(self.header_path()).map_err(|e| io_err(&e))?;
        let file_len = f.seek(SeekFrom::End(0)).map_err(|e| io_err(&e))?;
        if offset + HEADER_RECORD_SIZE as u64 > file_len {
            return Ok(None);
        }
        f.seek(SeekFrom::Start(offset)).map_err(|e| io_err(&e))?;
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
            self.utxo_file.flush().map_err(|e| io_err(&e))?;
            self.utxo_file
                .get_ref()
                .sync_all()
                .map_err(|e| io_err(&e))?;
            self.utxo_flushed_pos = self.utxo_write_pos;
            self.header_file.flush().map_err(|e| io_err(&e))?;
            self.header_file.sync_all().map_err(|e| io_err(&e))?;
            self.checkpoint_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| io_err(&e))?;
            let h = self.tip.height.to_u32().to_le_bytes();
            self.checkpoint_file.write_all(&h).map_err(|e| io_err(&e))?;
            self.checkpoint_file.sync_all().map_err(|e| io_err(&e))?;
        }
        Ok(())
    }

    fn recover(&mut self) -> Result<TipInfo, EngineError> {
        let mut cp_buf = [0u8; 4];
        let n = self
            .checkpoint_file
            .read(&mut cp_buf)
            .map_err(|e| io_err(&e))?;
        let checkpoint_height = if n == 4 {
            u32::from_le_bytes(cp_buf)
        } else {
            0
        };

        // Rebuild header index.
        self.header_index.clear();
        let header_path = self.header_path();
        let mut hf = File::open(&header_path).map_err(|e| io_err(&e))?;
        let file_len = hf.seek(SeekFrom::End(0)).map_err(|e| io_err(&e))?;
        hf.seek(SeekFrom::Start(0)).map_err(|e| io_err(&e))?;
        let record_count = file_len / HEADER_RECORD_SIZE as u64;
        for i in 0..record_count {
            let mut buf = [0u8; HEADER_RECORD_SIZE];
            if hf.read_exact(&mut buf).is_err() {
                break;
            }
            let hash = Hash256::from_bytes(buf.read_hash(0));
            // Height tail sits at offset 112 — see HEADER_RECORD_SIZE doc.
            let height = buf.read_u32_le(112);
            if height <= checkpoint_height || checkpoint_height == 0 {
                self.header_index.insert(hash, height);
                if i + 1 == record_count {
                    // Only need the tip hash/height; the parsed header itself is not kept.
                    self.tip = TipInfo {
                        hash,
                        height: BlockHeight::new(height),
                    };
                }
            }
        }

        // Rebuild UTXO index.
        self.utxo_index.clear();
        self.utxo_write_pos = 0;
        let utxo_path = self.utxo_path();
        if let Ok(mut uf) = File::open(&utxo_path) {
            let mut offset: u64 = 0;
            loop {
                match read_utxo_log_entry(&mut uf, offset) {
                    Ok(Some((op, Some(_), record_end))) => {
                        self.utxo_index.insert(op, offset + 36);
                        offset = record_end;
                    }
                    Ok(Some((op, None, record_end))) => {
                        self.utxo_index.remove(&op);
                        offset = record_end;
                    }
                    Ok(None) | Err(_) => {
                        self.utxo_write_pos = offset;
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

// ── UTXO log serialization ────────────────────────────────────────────────────
//
// Each record:
//   OutPoint (32 + 4 = 36 bytes)
//   tag: u8  (1 = create, 0 = spend)
//   if create:
//     value: u64 (8)
//     height: u32 (4)
//     coinbase: u8 (1)
//     script_pubkey_len: u32 (4)
//     script_pubkey: [u8; len]

fn write_coin_record<W: Write>(f: &mut W, nc: &NewCoin) -> Result<(), EngineError> {
    write_outpoint(f, &nc.outpoint)?;
    f.write_all(&[1u8]).map_err(|e| io_err(&e))?;
    f.write_all(&nc.value.to_le_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(&nc.height.to_u32().to_le_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(&[u8::from(nc.coinbase)])
        .map_err(|e| io_err(&e))?;
    let spk = nc.script_pubkey;
    let spk_len = u32::try_from(spk.len()).map_err(|_| EngineError::ScriptPubKeyTooLarge)?;
    f.write_all(&spk_len.to_le_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(spk).map_err(|e| io_err(&e))?;
    Ok(())
}

fn write_spend_record<W: Write>(f: &mut W, op: &OutPoint) -> Result<(), EngineError> {
    write_outpoint(f, op)?;
    f.write_all(&[0u8]).map_err(|e| io_err(&e))?;
    Ok(())
}

fn write_outpoint<W: Write>(f: &mut W, op: &OutPoint) -> Result<(), EngineError> {
    f.write_all(op.txid.as_bytes()).map_err(|e| io_err(&e))?;
    f.write_all(&op.vout.to_le_bytes())
        .map_err(|e| io_err(&e))?;
    Ok(())
}

fn read_coin_record(f: &mut File) -> Result<Option<Coin>, EngineError> {
    let mut tag = [0u8; 1];
    if f.read_exact(&mut tag).is_err() {
        return Ok(None);
    }
    if tag[0] == 0 {
        return Ok(None);
    }
    let mut buf8 = [0u8; 8];
    f.read_exact(&mut buf8).map_err(|e| io_err(&e))?;
    let value = u64::from_le_bytes(buf8);
    let mut buf4 = [0u8; 4];
    f.read_exact(&mut buf4).map_err(|e| io_err(&e))?;
    let height = u32::from_le_bytes(buf4);
    let mut tag1 = [0u8; 1];
    f.read_exact(&mut tag1).map_err(|e| io_err(&e))?;
    let coinbase = tag1[0] != 0;
    f.read_exact(&mut buf4).map_err(|e| io_err(&e))?;
    let spk_len =
        usize::try_from(u32::from_le_bytes(buf4)).map_err(|_| EngineError::ScriptLenOverflow)?;
    let mut spk = vec![0u8; spk_len];
    f.read_exact(&mut spk).map_err(|e| io_err(&e))?;
    Ok(Some(Coin {
        value: Satoshi::new(value),
        script_pubkey: ScriptPubKey::from(spk),
        height: BlockHeight::new(height),
        coinbase,
    }))
}

fn read_utxo_log_entry(
    f: &mut File,
    offset: u64,
) -> Result<Option<(OutPoint, Option<Coin>, u64)>, EngineError> {
    f.seek(SeekFrom::Start(offset)).map_err(|e| io_err(&e))?;
    let mut op_buf = [0u8; 36];
    if f.read_exact(&mut op_buf).is_err() {
        return Ok(None);
    }
    let txid = Hash256::from_bytes(op_buf.read_hash(0));
    let vout = op_buf.read_u32_le(32);
    let op = OutPoint { txid, vout };

    let mut tag = [0u8; 1];
    f.read_exact(&mut tag).map_err(|e| io_err(&e))?;
    if tag[0] == 0 {
        return Ok(Some((op, None, offset + 37)));
    }
    let mut buf8 = [0u8; 8];
    f.read_exact(&mut buf8).map_err(|e| io_err(&e))?;
    let value = u64::from_le_bytes(buf8);
    let mut buf4 = [0u8; 4];
    f.read_exact(&mut buf4).map_err(|e| io_err(&e))?;
    let height = u32::from_le_bytes(buf4);
    let mut tag1 = [0u8; 1];
    f.read_exact(&mut tag1).map_err(|e| io_err(&e))?;
    let coinbase = tag1[0] != 0;
    f.read_exact(&mut buf4).map_err(|e| io_err(&e))?;
    let spk_len =
        usize::try_from(u32::from_le_bytes(buf4)).map_err(|_| EngineError::ScriptLenOverflow)?;
    let mut spk = vec![0u8; spk_len];
    f.read_exact(&mut spk).map_err(|e| io_err(&e))?;
    let coin = Coin {
        value: Satoshi::new(value),
        script_pubkey: ScriptPubKey::from(spk),
        height: BlockHeight::new(height),
        coinbase,
    };
    // 36 (outpoint) + 1 (tag) + 8 + 4 + 1 + 4 + spk_len
    let record_end = offset
        + 36
        + 1
        + 8
        + 4
        + 1
        + 4
        + u64::try_from(spk_len).map_err(|_| EngineError::ScriptLenOverflow)?;
    Ok(Some((op, Some(coin), record_end)))
}

// ── Header record serialization ───────────────────────────────────────────────
//
// Fixed 116 bytes: hash(32) + prev_block(32) + merkle_root(32) +
//   time(4) + bits(4) + nonce(4) + height(4)

fn write_header_record(
    f: &mut File,
    h: BlockHeight,
    header: &BlockHeader,
    hash: Hash256,
    file_len: u64,
) -> Result<u64, EngineError> {
    // Files are opened with O_APPEND, so every write goes to EOF regardless
    // of any prior seek. We only have to extend the file to `offset` when
    // there is a gap; the actual record then appends naturally.
    let offset = u64::from(h.to_u32()) * HEADER_RECORD_SIZE as u64;
    let new_len = if offset > file_len {
        // Use ftruncate to extend the file; the gap becomes a sparse hole on
        // every Linux filesystem we support (ext4/xfs/btrfs/tmpfs) — no zero
        // blocks are written. Reads from the hole return zeros, which is the
        // semantics we need. In production each commit is at `tip+1` so this
        // branch never fires; the bench harness commits sample blocks at high
        // heights to fresh storage, which previously issued ~1300 small write
        // syscalls per commit padding the gap.
        f.set_len(offset).map_err(|e| io_err(&e))?;
        offset
    } else {
        file_len
    };
    // Layout: hash(32) ‖ version(4) ‖ prev(32) ‖ merkle(32) ‖
    //         time(4) ‖ bits(4) ‖ nonce(4) ‖ height(4) = 116 bytes.
    f.write_all(hash.as_bytes()).map_err(|e| io_err(&e))?;
    f.write_all(&header.version().to_le_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(header.prev_block().as_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(header.merkle_root().as_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(&header.time().to_le_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(&header.bits().to_le_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(&header.nonce().to_le_bytes())
        .map_err(|e| io_err(&e))?;
    f.write_all(&h.to_u32().to_le_bytes())
        .map_err(|e| io_err(&e))?;
    Ok(new_len + HEADER_RECORD_SIZE as u64)
}

fn read_header_record(f: &mut File) -> Result<BlockHeader, EngineError> {
    let mut buf = [0u8; HEADER_RECORD_SIZE];
    f.read_exact(&mut buf).map_err(|e| io_err(&e))?;
    Ok(parse_header_record(&buf))
}

#[allow(clippy::cast_possible_wrap)]
fn parse_header_record(buf: &[u8; HEADER_RECORD_SIZE]) -> BlockHeader {
    // Mirror of `write_header_record`. Offsets must stay in lockstep
    // because existing data dirs are read by this exact scheme.
    BlockHeader::new(
        buf.read_u32_le(32) as i32,
        Hash256::from_bytes(buf.read_hash(36)),
        Hash256::from_bytes(buf.read_hash(68)),
        buf.read_u32_le(100),
        mantle::primitives::CompactBits::new(buf.read_u32_le(104)),
        buf.read_u32_le(108),
    )
}

fn open_append(path: &Path) -> Result<File, EngineError> {
    OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .map_err(|e| io_err(&e))
}

fn open_read(path: &Path) -> Result<File, EngineError> {
    OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|e| io_err(&e))
}

fn io_err(e: &io::Error) -> EngineError {
    EngineError::Io(io::Error::from(e.kind()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mantle::primitives::{CompactBits, Hash256};

    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn new() -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos();
            let path =
                std::env::temp_dir().join(format!("mantle-test-{}-{}", std::process::id(), ns));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp() -> TmpDir {
        TmpDir::new()
    }

    fn make_header(time: u32) -> BlockHeader {
        BlockHeader::new(
            1,
            Hash256::ZERO,
            Hash256::ZERO,
            time,
            CompactBits::new(0x1d00_ffff),
            0,
        )
    }

    #[test]
    fn write_and_read_header_roundtrips_every_field() {
        let dir = tmp();
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        let h = BlockHeader::new(
            0x2000_0007_i32,
            Hash256::from_bytes([0x11; 32]),
            Hash256::from_bytes([0x22; 32]),
            12_345,
            CompactBits::new(0x1d00_ffff),
            0xdead_beef,
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
    fn write_and_read_coin() {
        let dir = tmp();
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        let op = OutPoint {
            txid: Hash256::from_bytes([1u8; 32]),
            vout: 0,
        };
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(5_000_000_000),
            script_pubkey: &[0xac],
            coinbase: true,
            height: BlockHeight::new(1),
        };
        let req = crate::types::CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(1),
            hash: Hash256::from_bytes([2u8; 32]),
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
        let dir = tmp();
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        let op = OutPoint {
            txid: Hash256::from_bytes([3u8; 32]),
            vout: 0,
        };
        let nc = NewCoin {
            outpoint: op,
            value: Satoshi::new(1000),
            script_pubkey: &[],
            coinbase: false,
            height: BlockHeight::new(10),
        };
        let req1 = crate::types::CommitBlockReq {
            token: crate::types::ReqToken(1),
            height: BlockHeight::new(10),
            hash: Hash256::from_bytes([4u8; 32]),
            prev_hash: Hash256::ZERO,
            header: make_header(0),
            created: &[nc],
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        };
        eng.write_block(&req1).unwrap();
        assert!(eng.read_coin(&op).unwrap().is_some());

        let req2 = crate::types::CommitBlockReq {
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
    fn recover_rebuilds_index() {
        let dir = tmp();
        let op = OutPoint {
            txid: Hash256::from_bytes([7u8; 32]),
            vout: 1,
        };
        {
            let mut eng = FlatEngine::open(dir.path()).unwrap();
            let nc = NewCoin {
                outpoint: op,
                value: Satoshi::new(999),
                script_pubkey: &[0x51],
                coinbase: false,
                height: BlockHeight::new(5),
            };
            let req = crate::types::CommitBlockReq {
                token: crate::types::ReqToken(1),
                height: BlockHeight::new(5),
                hash: Hash256::from_bytes([8u8; 32]),
                prev_hash: Hash256::ZERO,
                header: make_header(0),
                created: &[nc],
                spent: &[],
                block_bytes: None,
                durability: Durability::Memory,
            };
            eng.write_block(&req).unwrap();
        }
        let mut eng2 = FlatEngine::open(dir.path()).unwrap();
        let coin = eng2.read_coin(&op).unwrap().unwrap();
        assert_eq!(coin.value.to_u64(), 999);
    }
}
