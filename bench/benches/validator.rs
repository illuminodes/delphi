use mantle::primitives::{BlockHeader, BlockHeight, Hash256};
use mantle_consensus::script::ScriptExecutionCache;
use mantle_validator::{HeaderSnapshot, PipelineOpts, ReplayBufs};
use mantle_storage::types::{Durability, ReqToken};
use std::hint::black_box;

fn main() {
    divan::main();
}

// ── Helpers ──────────────────────────────────────────────────────────────────

const EASY_BITS: mantle::primitives::CompactBits =
    mantle::primitives::CompactBits::new(0x207f_ffff);
const BASE_TIME: u32 = 1_231_006_505;

fn coinbase_tx(height: u32, value: u64) -> mantle::primitives::Transaction {
    use mantle::primitives::*;
    let h = height.to_le_bytes();
    Transaction {
        version: TxVersion::new(1),
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: vec![0x03, h[0], h[1], h[2]].into(),
            sequence: Sequence::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: Locktime::new(0),
    }
}

fn mine(mut block: mantle::primitives::Block) -> mantle::primitives::Block {
    use mantle::primitives::*;
    let target = block.header.bits().to_target().expect("valid bits");
    for nonce in 0..=u32::MAX {
        block.header.set_nonce(Nonce::new(nonce));
        let mut hash = *Hash256::hash(&block.header.to_bytes()).as_bytes();
        hash.reverse();
        if hash <= target {
            return block;
        }
    }
    panic!("no valid nonce");
}

fn make_block(
    prev_hash: Hash256,
    height: u32,
    txs: Vec<mantle::primitives::Transaction>,
) -> mantle::primitives::Block {
    use mantle::primitives::*;
    use mantle_consensus::merkle::compute_merkle_root;
    let txids: Vec<Hash256> = txs.iter().map(|tx| tx.txid().expect("txid")).collect();
    let merkle_root = compute_merkle_root(&txids).expect("merkle root");
    mine(Block {
        header: BlockHeader::new(
            BlockVersion::new(4), prev_hash, merkle_root,
            BlockTime::new(BASE_TIME + height * 600), EASY_BITS, Nonce::new(0),
        ),
        transactions: txs,
    })
}

fn pre_genesis_header() -> BlockHeader {
    BlockHeader::new(
        mantle::primitives::BlockVersion::new(1), Hash256::ZERO, Hash256::ZERO,
        mantle::primitives::BlockTime::new(BASE_TIME), EASY_BITS,
        mantle::primitives::Nonce::new(0),
    )
}

fn make_snapshot(chain: &[(Hash256, BlockHeader)]) -> HeaderSnapshot {
    let mut entries = Vec::with_capacity(chain.len() + 1);
    entries.push((Hash256::ZERO, pre_genesis_header()));
    entries.extend_from_slice(chain);
    HeaderSnapshot::new(BlockHeight::new(0), entries).expect("snapshot fits u32")
}

struct TmpDir(std::path::PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp_storage() -> (mantle_storage::StorageHandle<'static>, TmpDir) {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = std::env::temp_dir().join(format!(
        "mantle-validator-bench-{}-{}", std::process::id(), ns
    ));
    std::fs::create_dir_all(&path).expect("create tmp dir");
    let handle = mantle_storage::StorageHandle::open(&path).expect("open");
    (handle, TmpDir(path))
}

fn commit(
    bufs: &mut ReplayBufs,
    storage: &mut mantle_storage::StorageHandle<'static>,
    block: &mantle::primitives::Block,
    height: BlockHeight,
    snap: &HeaderSnapshot,
    cache: &mut ScriptExecutionCache,
    token: u64,
) {
    bufs.validate_and_commit(block, height, storage, snap, PipelineOpts {
        skip_script: true,
        exec_cache: Some(cache),
        durability: Durability::Memory,
        token: ReqToken(token),
    }).unwrap_or_else(|e| panic!("commit at height {}: {e}", height.to_u32()));
}

// ── HeaderSnapshot ───────────────────────────────────────────────────────────

mod header_snapshot {
    use super::*;

    fn make_chain(n: usize) -> (Vec<(Hash256, BlockHeader)>, HeaderSnapshot) {
        let mut prev = Hash256::ZERO;
        let mut chain = Vec::with_capacity(n);
        for i in 0..n {
            let h = u32::try_from(i + 1).expect("fits");
            let block = make_block(prev, h, vec![coinbase_tx(h, 50_0000_0000)]);
            let hash = block.header.hash();
            chain.push((hash, block.header));
            prev = hash;
        }
        let snap = make_snapshot(&chain);
        (chain, snap)
    }

    #[divan::bench(args = [100usize, 1000, 10_000])]
    fn construction(bencher: divan::Bencher, n: usize) {
        let mut prev = Hash256::ZERO;
        let mut chain = Vec::with_capacity(n);
        for i in 0..n {
            let h = u32::try_from(i + 1).expect("fits");
            let block = make_block(prev, h, vec![coinbase_tx(h, 50_0000_0000)]);
            let hash = block.header.hash();
            chain.push((hash, block.header));
            prev = hash;
        }
        let mut entries = Vec::with_capacity(chain.len() + 1);
        entries.push((Hash256::ZERO, pre_genesis_header()));
        entries.extend_from_slice(&chain);
        bencher.bench(|| {
            HeaderSnapshot::new(BlockHeight::new(0), black_box(entries.clone()))
                .expect("fits");
        });
    }

    #[divan::bench(args = [100usize, 1000, 10_000])]
    fn get_header_by_hash(bencher: divan::Bencher, n: usize) {
        let (chain, snap) = make_chain(n);
        let target = chain[n / 2].0;
        bencher.bench(|| {
            black_box(snap.get_header(black_box(&target)));
        });
    }

    #[divan::bench(args = [100usize, 1000, 10_000])]
    fn get_header_at(bencher: divan::Bencher, n: usize) {
        let (_chain, snap) = make_chain(n);
        let target = BlockHeight::new(u32::try_from(n / 2).expect("fits"));
        bencher.bench(|| {
            black_box(snap.get_header_at(black_box(target)));
        });
    }

    #[divan::bench(args = [100usize, 1000, 10_000])]
    fn median_time_past(bencher: divan::Bencher, n: usize) {
        let (chain, snap) = make_chain(n);
        let hdr = chain[n / 2].1;
        bencher.bench(|| {
            black_box(snap.median_time_past(black_box(&hdr)));
        });
    }
}

// ── Pipeline: validate_and_commit ────────────────────────────────────────────

mod pipeline {
    use super::*;

    #[divan::bench]
    fn coinbase_only(bencher: divan::Bencher) {
        let block = make_block(Hash256::ZERO, 1, vec![coinbase_tx(1, 50_0000_0000)]);
        let hash = block.header.hash();
        let snap = make_snapshot(&[(hash, block.header)]);
        bencher.bench_local(|| {
            let (mut storage, _dir) = tmp_storage();
            let mut cache = ScriptExecutionCache::new(1024);
            let mut bufs = ReplayBufs::new();
            commit(&mut bufs, &mut storage, black_box(&block),
                   BlockHeight::new(1), &snap, &mut cache, 1);
            storage.shutdown().expect("shutdown");
        });
    }

    #[divan::bench(args = [10usize, 50, 100])]
    fn n_spends(bencher: divan::Bencher, n: usize) {
        use mantle::primitives::*;

        let fund_cb = Transaction {
            version: TxVersion::new(1),
            inputs: vec![TxIn {
                previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
                script_sig: vec![0x03, 0x01, 0x00, 0x00].into(),
                sequence: Sequence::MAX,
                witness: Witness::empty(),
            }],
            outputs: (0..n).map(|_| TxOut {
                value: Satoshi::new(1000),
                script_pubkey: vec![0x51].into(),
            }).collect(),
            locktime: Locktime::new(0),
        };
        let fund_block = make_block(Hash256::ZERO, 1, vec![fund_cb]);
        let fund_txid = fund_block.transactions[0].txid().expect("txid");
        let fund_hash = fund_block.header.hash();

        let mut prev = fund_hash;
        let mut filler = Vec::new();
        for h in 2..=101u32 {
            let b = make_block(prev, h, vec![coinbase_tx(h, 50_0000_0000)]);
            prev = b.header.hash();
            filler.push(b);
        }

        let spend_txs: Vec<Transaction> =
            std::iter::once(coinbase_tx(102, 50_0000_0000))
                .chain((0..n).map(|i| {
                    Transaction {
                        version: TxVersion::new(1),
                        inputs: vec![TxIn {
                            previous_output: OutPoint {
                                txid: fund_txid, vout: u32::try_from(i).unwrap(),
                            },
                            script_sig: vec![0x51].into(),
                            sequence: Sequence::MAX,
                            witness: Witness::empty(),
                        }],
                        outputs: vec![TxOut {
                            value: Satoshi::new(999),
                            script_pubkey: vec![0x51].into(),
                        }],
                        locktime: Locktime::new(0),
                    }
                }))
                .collect();
        let spend_block = make_block(prev, 102, spend_txs);
        let spend_hash = spend_block.header.hash();

        let mut chain = vec![(fund_hash, fund_block.header)];
        for b in &filler {
            chain.push((b.header.hash(), b.header));
        }
        chain.push((spend_hash, spend_block.header));
        let snap = make_snapshot(&chain);

        bencher.bench_local(|| {
            let (mut storage, _dir) = tmp_storage();
            let mut cache = ScriptExecutionCache::new(1024);
            let mut bufs = ReplayBufs::new();
            commit(&mut bufs, &mut storage, &fund_block,
                   BlockHeight::new(1), &snap, &mut cache, 1);
            for (i, b) in filler.iter().enumerate() {
                commit(&mut bufs, &mut storage, b,
                       BlockHeight::new(2 + u32::try_from(i).unwrap()),
                       &snap, &mut cache, 2 + i as u64);
            }
            commit(&mut bufs, &mut storage, black_box(&spend_block),
                   BlockHeight::new(102), &snap, &mut cache, 102);
            storage.shutdown().expect("shutdown");
        });
    }

    #[divan::bench(args = [10usize, 50, 100])]
    fn chain_k(bencher: divan::Bencher, k: usize) {
        let mut prev = Hash256::ZERO;
        let mut chain = Vec::with_capacity(k);
        let mut blocks = Vec::with_capacity(k);
        for i in 0..k {
            let h = u32::try_from(i + 1).expect("fits");
            let block = make_block(prev, h, vec![coinbase_tx(h, 50_0000_0000)]);
            let hash = block.header.hash();
            chain.push((hash, block.header));
            prev = hash;
            blocks.push(block);
        }
        let snap = make_snapshot(&chain);

        bencher.bench_local(|| {
            let (mut storage, _dir) = tmp_storage();
            let mut cache = ScriptExecutionCache::new(1024);
            let mut bufs = ReplayBufs::new();
            for (i, block) in blocks.iter().enumerate() {
                commit(&mut bufs, &mut storage, black_box(block),
                       BlockHeight::new(u32::try_from(i + 1).expect("fits")),
                       &snap, &mut cache, i as u64);
            }
            storage.shutdown().expect("shutdown");
        });
    }

    #[divan::bench]
    fn bufs_reuse_across_100_blocks(bencher: divan::Bencher) {
        let mut prev = Hash256::ZERO;
        let mut chain = Vec::with_capacity(100);
        let mut blocks = Vec::with_capacity(100);
        for i in 0..100usize {
            let h = u32::try_from(i + 1).expect("fits");
            let block = make_block(prev, h, vec![coinbase_tx(h, 50_0000_0000)]);
            let hash = block.header.hash();
            chain.push((hash, block.header));
            prev = hash;
            blocks.push(block);
        }
        let snap = make_snapshot(&chain);

        bencher.bench_local(|| {
            let (mut storage, _dir) = tmp_storage();
            let mut cache = ScriptExecutionCache::new(1024);
            let mut bufs = ReplayBufs::new();
            for (i, block) in blocks.iter().enumerate() {
                bufs.validate_and_commit(block, BlockHeight::new(u32::try_from(i + 1).unwrap()),
                    &mut storage, &snap, PipelineOpts {
                        skip_script: true,
                        exec_cache: Some(&mut cache),
                        durability: Durability::Memory,
                        token: ReqToken(i as u64),
                    },
                ).expect("validate_and_commit");
            }
            storage.shutdown().expect("shutdown");
        });
    }
}
