//! dhat heap profiler for the replay path.
//!
//! Run with:
//!   cargo build --release --bin replay-dhat
//!   ./target/release/replay-dhat
//!
//! dhat writes `dhat-heap.json` in the current directory on exit.
//! Open it at <https://nnethercote.github.io/dh_view/dh_view.html>.

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

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const EASY_BITS: CompactBits = CompactBits::new(0x207f_ffff);
const BASE_TIME: u32 = 1_231_006_505;

fn coinbase_tx(height: u32, value: u64) -> Transaction {
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

fn spend_tx(prev_txid: Hash256, vout: u32, value: u64) -> Transaction {
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

fn mine(mut block: Block) -> Block {
    let target = block
        .header
        .bits
        .to_target()
        .expect("EASY_BITS always valid");
    for nonce in 0..=u32::MAX {
        block.header.nonce = nonce;
        let mut hash = *Hash256::hash(&block.header.to_bytes()).as_bytes();
        hash.reverse();
        if hash <= target {
            return block;
        }
    }
    panic!("no valid nonce");
}

fn make_block(prev_hash: Hash256, height: u32, txs: Vec<Transaction>) -> Block {
    let txids: Vec<Hash256> = txs.iter().map(|tx| tx.txid().expect("txid")).collect();
    let merkle_root = compute_merkle_root(&txids).expect("non-empty list has merkle root");
    mine(Block {
        header: BlockHeader {
            version: 4,
            prev_block: prev_hash,
            merkle_root,
            time: BASE_TIME + height * 600,
            bits: EASY_BITS,
            nonce: 0,
        },
        transactions: txs,
    })
}

fn pre_genesis_header() -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256::ZERO,
        merkle_root: Hash256::ZERO,
        time: BASE_TIME,
        bits: EASY_BITS,
        nonce: 0,
    }
}

fn make_snapshot(chain: &[(Hash256, BlockHeader)]) -> HeaderSnapshot {
    let mut entries = Vec::with_capacity(chain.len() + 1);
    entries.push((Hash256::ZERO, pre_genesis_header()));
    entries.extend_from_slice(chain);
    HeaderSnapshot::new(BlockHeight(0), entries).expect("snapshot fits u32")
}

struct TmpDir(std::path::PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp_storage() -> (StorageHandle<'static>, TmpDir) {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path =
        std::env::temp_dir().join(format!("mantle-replay-dhat-{}-{}", std::process::id(), ns));
    std::fs::create_dir_all(&path).expect("create tmp dir");
    let handle = StorageHandle::open(&path).expect("StorageHandle::open");
    (handle, TmpDir(path))
}

fn commit<S>(
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
    .unwrap_or_else(|e| panic!("commit at height {}: {e}", height.0));
}

fn main() {
    let _profiler = dhat::Profiler::new_heap();
    scenario_coinbase_only();
    scenario_spend_blocks();
}

fn scenario_coinbase_only() {
    let (mut storage, _dir) = tmp_storage();
    let mut cache = ScriptExecutionCache::new(1024);
    let mut chain: Vec<(Hash256, BlockHeader)> = Vec::new();
    let mut prev_hash = Hash256::ZERO;
    for i in 1u32..=200 {
        let block = make_block(prev_hash, i, vec![coinbase_tx(i, 50_0000_0000)]);
        let hash = block.header.hash();
        chain.push((hash, block.header));
        let snap = make_snapshot(&chain);
        commit(
            &mut storage,
            &block,
            BlockHeight(i),
            &snap,
            &mut cache,
            u64::from(i),
        );
        prev_hash = hash;
    }
    storage.shutdown().expect("shutdown");
}

fn scenario_spend_blocks() {
    const SPENDS: usize = 20;
    const ROUNDS: usize = 20;
    const MATURITY: u32 = 100;
    const FUND_H: u32 = 1;
    const SPEND_H: u32 = FUND_H + MATURITY + 1;

    let fund_cb = Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256::ZERO,
                vout: u32::MAX,
            },
            script_sig: vec![0x03, 0x01, 0x00, 0x00].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: (0..SPENDS)
            .map(|_| TxOut {
                value: Satoshi::new(1000),
                script_pubkey: vec![0x51].into(),
            })
            .collect(),
        locktime: 0,
    };
    let fund_block = make_block(Hash256::ZERO, FUND_H, vec![fund_cb]);
    let fund_txid = fund_block.transactions[0].txid().expect("txid");
    let fund_hash = fund_block.header.hash();

    let mut prev_hash = fund_hash;
    let mut filler: Vec<Block> = Vec::new();
    for h in 2..SPEND_H {
        let b = make_block(prev_hash, h, vec![coinbase_tx(h, 50_0000_0000)]);
        prev_hash = b.header.hash();
        filler.push(b);
    }

    let spend_txs: Vec<Transaction> = std::iter::once(coinbase_tx(SPEND_H, 50_0000_0000))
        .chain((0..SPENDS).map(|i| spend_tx(fund_txid, u32::try_from(i).expect("fits"), 999)))
        .collect();
    let spend_block = make_block(prev_hash, SPEND_H, spend_txs);
    let spend_hash = spend_block.header.hash();

    let mut chain = vec![(fund_hash, fund_block.header)];
    for b in &filler {
        chain.push((b.header.hash(), b.header));
    }
    chain.push((spend_hash, spend_block.header));
    let snap = make_snapshot(&chain);

    for _ in 0..ROUNDS {
        let (mut storage, _dir) = tmp_storage();
        let mut cache = ScriptExecutionCache::new(1024);
        commit(
            &mut storage,
            &fund_block,
            BlockHeight(FUND_H),
            &snap,
            &mut cache,
            1,
        );
        for (i, b) in filler.iter().enumerate() {
            commit(
                &mut storage,
                b,
                BlockHeight(2 + u32::try_from(i).expect("fits")),
                &snap,
                &mut cache,
                2 + u64::try_from(i).expect("fits"),
            );
        }
        commit(
            &mut storage,
            &spend_block,
            BlockHeight(SPEND_H),
            &snap,
            &mut cache,
            u64::from(SPEND_H),
        );
        storage.shutdown().expect("shutdown");
    }
}
