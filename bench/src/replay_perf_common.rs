//! Shared helpers for the per-workload replay-perf binaries.
//!
//! Included into each binary via `#[path = "replay_perf_common.rs"] mod common;`
//! so the three workloads can be run and diagnosed in isolation without a
//! shared library crate.
#![allow(dead_code)]
#![allow(clippy::cast_precision_loss)]

use std::time::{Duration, Instant};

use mantle::primitives::{
    Block, BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, Transaction, TxHash,
    TxIn, TxOut, Witness,
};
use mantle_consensus::merkle::compute_merkle_root;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_storage::StorageHandle;
use mantle_storage::ops::StorageOps;
use mantle_storage::types::{Durability, ReqToken};
use mantle_validator::pipeline::validate_and_commit;
use mantle_validator::prefetched_view::HeaderSnapshot;

pub const EASY_BITS: CompactBits = CompactBits::new(0x207f_ffff);
pub const BASE_TIME: u32 = 1_231_006_505;
pub const SECS: Duration = Duration::from_secs(5);

pub fn coinbase_tx(height: u32, value: u64) -> Transaction {
    let h = height.to_le_bytes();
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256::ZERO,
                vout: u32::MAX,
            },
            script_sig: vec![0x03, h[0], h[1], h[2]].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: 0,
    }
}

pub fn spend_tx(prev_txid: Hash256, vout: u32, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: prev_txid,
                vout,
            },
            script_sig: vec![0x51].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: 0,
    }
}

pub fn mine(mut block: Block) -> Block {
    let target = block
        .header
        .bits()
        .to_target()
        .expect("EASY_BITS always valid");
    for nonce in 0..=u32::MAX {
        block.header.set_nonce(nonce);
        let mut hash = *Hash256::hash(&block.header.to_bytes()).as_bytes();
        hash.reverse();
        if hash <= target {
            return block;
        }
    }
    panic!("no valid nonce");
}

pub fn make_block(prev_hash: Hash256, height: u32, txs: Vec<Transaction>) -> Block {
    let txids: Vec<Hash256> = txs.iter().map(|tx| tx.txid().expect("txid")).collect();
    let merkle_root = compute_merkle_root(&txids).expect("non-empty list has merkle root");
    mine(Block {
        header: BlockHeader::new(
            4,
            prev_hash,
            merkle_root,
            BASE_TIME + height * 600,
            EASY_BITS,
            0,
        ),
        transactions: txs,
    })
}

pub fn pre_genesis_header() -> BlockHeader {
    BlockHeader::new(1, Hash256::ZERO, Hash256::ZERO, BASE_TIME, EASY_BITS, 0)
}

pub fn make_snapshot(chain: &[(Hash256, BlockHeader)]) -> HeaderSnapshot {
    let mut entries = Vec::with_capacity(chain.len() + 1);
    entries.push((Hash256::ZERO, pre_genesis_header()));
    entries.extend_from_slice(chain);
    HeaderSnapshot::new(BlockHeight::new(0), entries).expect("snapshot fits u32")
}

pub struct TmpDir(pub std::path::PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn tmp_storage() -> (StorageHandle<'static>, TmpDir) {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path =
        std::env::temp_dir().join(format!("mantle-replay-perf-{}-{}", std::process::id(), ns));
    std::fs::create_dir_all(&path).expect("create tmp dir");
    let handle = StorageHandle::open(&path).expect("StorageHandle::open");
    (handle, TmpDir(path))
}

pub fn commit<S>(
    storage: &mut S,
    block: &Block,
    height: BlockHeight,
    snap: &HeaderSnapshot,
    cache: &mut ScriptExecutionCache,
    token: u64,
) where
    S: for<'x> StorageOps<'x>,
{
    validate_and_commit(
        block,
        height,
        storage,
        snap,
        true,
        Some(cache),
        Durability::Memory,
        ReqToken(token),
    )
    .unwrap_or_else(|e| panic!("commit at height {}: {e}", height.to_u32()));
}

pub fn run_for(dur: Duration, mut f: impl FnMut()) -> u64 {
    let deadline = Instant::now() + dur;
    let mut n = 0u64;
    while Instant::now() < deadline {
        f();
        n += 1;
    }
    n
}
