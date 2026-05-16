use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use mantle::codec::Decode;
use mantle::collections::PrehashedSet;
use mantle::primitives::{
    Block, BlockHeader, BlockHeight, BlockTime, BlockVersion, CompactBits, Hash256, Nonce,
    OutPoint, Satoshi, TxHash,
};
use mantle_consensus::chain_view::NewCoin;
use mantle_storage::engine::arena::ArenaEngine;
use mantle_storage::engine::flat::FlatEngine;
use mantle_storage::engine::robin_hood::RobinHoodEngine;
use mantle_storage::engine::tx_grouped::TxGroupedEngine;
use mantle_storage::engine::Engine;
use mantle_storage::types::{CommitBlockReq, Durability, LookupResult, ReqToken};
use mantle_utxo_set::{ArenaUtxoSet, ConnectScratch};

fn main() {
    divan::main();
}

const P2WPKH_SPK: [u8; 22] = [
    0x00, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
    0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
];

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

fn make_txid(i: u32) -> Hash256 {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&i.to_le_bytes());
    Hash256::from_bytes(bytes)
}

fn make_outpoint(i: u32) -> OutPoint {
    OutPoint {
        txid: make_txid(i),
        vout: 0,
    }
}

fn make_outpoint_vout(tx: u32, vout: u32) -> OutPoint {
    OutPoint {
        txid: make_txid(tx),
        vout,
    }
}

fn make_block_hash(i: u32) -> Hash256 {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&(i + 10_000).to_le_bytes());
    Hash256::from_bytes(bytes)
}

struct TmpDir(std::path::PathBuf);
impl TmpDir {
    fn new(name: &str) -> Self {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let path = std::env::temp_dir().join(format!(
            "mantle-utxo-bench-{name}-{}-{ns}",
            std::process::id()
        ));
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

fn seed_engine(eng: &mut dyn Engine, n_blocks: u32) {
    let mut prev = Hash256::ZERO;
    for h in 1..=n_blocks {
        let op = make_outpoint(h);
        let block_hash = make_block_hash(h);
        let created = [NewCoin {
            outpoint: op,
            value: Satoshi::new(5_000_000_000),
            script_pubkey: &P2WPKH_SPK,
            coinbase: true,
            height: BlockHeight::new(h),
        }];
        let mut header = make_header(h);
        header.set_prev_block(prev);
        eng.write_block(&CommitBlockReq {
            token: ReqToken(u64::from(h)),
            height: BlockHeight::new(h),
            hash: block_hash,
            prev_hash: prev,
            header,
            created: &created,
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        })
        .unwrap();
        prev = block_hash;
    }
}

fn seed_engine_multi_output(eng: &mut dyn Engine, n_blocks: u32, outputs_per_tx: u32) {
    let mut prev = Hash256::ZERO;
    for h in 1..=n_blocks {
        let height = BlockHeight::new(h);
        let block_hash = make_block_hash(h);
        let created: Vec<NewCoin<'_>> = (0..outputs_per_tx)
            .map(|vout| NewCoin {
                outpoint: make_outpoint_vout(h, vout),
                value: Satoshi::new(1000),
                script_pubkey: &P2WPKH_SPK,
                coinbase: false,
                height,
            })
            .collect();
        let mut header = make_header(h);
        header.set_prev_block(prev);
        eng.write_block(&CommitBlockReq {
            token: ReqToken(u64::from(h)),
            height,
            hash: block_hash,
            prev_hash: prev,
            header,
            created: &created,
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        })
        .unwrap();
        prev = block_hash;
    }
}

// ── Single-coin lookup ──────────────────────────────────────────────────────

mod lookup {
    use super::*;

    #[divan::bench]
    fn flat_hit(bencher: divan::Bencher) {
        let dir = TmpDir::new("flat-lh");
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let target = make_outpoint(500);
        bencher.bench_local(|| {
            black_box(eng.read_coin(black_box(&target)).unwrap());
        });
    }

    #[divan::bench]
    fn robin_hood_hit(bencher: divan::Bencher) {
        let dir = TmpDir::new("rh-lh");
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let target = make_outpoint(500);
        bencher.bench_local(|| {
            black_box(eng.read_coin(black_box(&target)).unwrap());
        });
    }

    #[divan::bench]
    fn tx_grouped_hit(bencher: divan::Bencher) {
        let dir = TmpDir::new("txg-lh");
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let target = make_outpoint(500);
        bencher.bench_local(|| {
            black_box(eng.read_coin(black_box(&target)).unwrap());
        });
    }

    #[divan::bench]
    fn flat_miss(bencher: divan::Bencher) {
        let dir = TmpDir::new("flat-lm");
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let missing = make_outpoint(99_999);
        bencher.bench_local(|| {
            black_box(eng.read_coin(black_box(&missing)).unwrap());
        });
    }

    #[divan::bench]
    fn robin_hood_miss(bencher: divan::Bencher) {
        let dir = TmpDir::new("rh-lm");
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let missing = make_outpoint(99_999);
        bencher.bench_local(|| {
            black_box(eng.read_coin(black_box(&missing)).unwrap());
        });
    }

    #[divan::bench]
    fn tx_grouped_miss(bencher: divan::Bencher) {
        let dir = TmpDir::new("txg-lm");
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let missing = make_outpoint(99_999);
        bencher.bench_local(|| {
            black_box(eng.read_coin(black_box(&missing)).unwrap());
        });
    }
}

// ── Batched lookup (100 coins) ──────────────────────────────────────────────

mod batch_lookup {
    use super::*;

    #[divan::bench]
    fn flat_100(bencher: divan::Bencher) {
        let dir = TmpDir::new("flat-b100");
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let ops: Vec<OutPoint> = (1..=100).map(make_outpoint).collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&ops), &mut out).unwrap();
        });
    }

    #[divan::bench]
    fn robin_hood_100(bencher: divan::Bencher) {
        let dir = TmpDir::new("rh-b100");
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let ops: Vec<OutPoint> = (1..=100).map(make_outpoint).collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&ops), &mut out).unwrap();
        });
    }

    #[divan::bench]
    fn tx_grouped_100(bencher: divan::Bencher) {
        let dir = TmpDir::new("txg-b100");
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        seed_engine(&mut eng, 1000);
        let ops: Vec<OutPoint> = (1..=100).map(make_outpoint).collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&ops), &mut out).unwrap();
        });
    }
}

// ── Connect block (write_block with creates) ────────────────────────────────

mod connect {
    use super::*;

    fn bench_write_block(bencher: divan::Bencher, eng: &mut dyn Engine) {
        let mut h = 1u32;
        let mut prev = Hash256::ZERO;
        bencher.bench_local(|| {
            let op = make_outpoint(h);
            let block_hash = make_block_hash(h);
            let created = [NewCoin {
                outpoint: op,
                value: Satoshi::new(5_000_000_000),
                script_pubkey: &P2WPKH_SPK,
                coinbase: true,
                height: BlockHeight::new(h),
            }];
            let mut header = make_header(h);
            header.set_prev_block(prev);
            eng.write_block(&CommitBlockReq {
                token: ReqToken(u64::from(h)),
                height: BlockHeight::new(h),
                hash: block_hash,
                prev_hash: prev,
                header,
                created: &created,
                spent: &[],
                block_bytes: None,
                durability: Durability::Memory,
            })
            .unwrap();
            prev = block_hash;
            h += 1;
        });
    }

    #[divan::bench]
    fn flat(bencher: divan::Bencher) {
        let dir = TmpDir::new("flat-conn");
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        bench_write_block(bencher, &mut eng);
    }

    #[divan::bench]
    fn robin_hood(bencher: divan::Bencher) {
        let dir = TmpDir::new("rh-conn");
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
        bench_write_block(bencher, &mut eng);
    }

    #[divan::bench]
    fn tx_grouped(bencher: divan::Bencher) {
        let dir = TmpDir::new("txg-conn");
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        bench_write_block(bencher, &mut eng);
    }
}

// ── Connect + spend (create then consume via write_block) ───────────────────

mod connect_spend {
    use super::*;

    fn bench_write_and_spend(bencher: divan::Bencher, eng: &mut dyn Engine) {
        let mut h = 1u32;
        let mut prev = Hash256::ZERO;
        bencher.bench_local(|| {
            let op = make_outpoint(h);
            let block_hash = make_block_hash(h);
            let height = BlockHeight::new(h);
            let created = [NewCoin {
                outpoint: op,
                value: Satoshi::new(1000),
                script_pubkey: &P2WPKH_SPK,
                coinbase: false,
                height,
            }];
            let mut header = make_header(h);
            header.set_prev_block(prev);
            eng.write_block(&CommitBlockReq {
                token: ReqToken(u64::from(h)),
                height,
                hash: block_hash,
                prev_hash: prev,
                header,
                created: &created,
                spent: &[],
                block_bytes: None,
                durability: Durability::Memory,
            })
            .unwrap();

            let spend_hash = make_block_hash(h + 100_000);
            let spend_height = BlockHeight::new(h + 1);
            let mut spend_header = make_header(h + 1);
            spend_header.set_prev_block(block_hash);
            eng.write_block(&CommitBlockReq {
                token: ReqToken(u64::from(h) + 100_000),
                height: spend_height,
                hash: spend_hash,
                prev_hash: block_hash,
                header: spend_header,
                created: &[],
                spent: &[op],
                block_bytes: None,
                durability: Durability::Memory,
            })
            .unwrap();
            prev = spend_hash;
            h += 2;
        });
    }

    #[divan::bench]
    fn flat(bencher: divan::Bencher) {
        let dir = TmpDir::new("flat-cs");
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        bench_write_and_spend(bencher, &mut eng);
    }

    #[divan::bench]
    fn robin_hood(bencher: divan::Bencher) {
        let dir = TmpDir::new("rh-cs");
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
        bench_write_and_spend(bencher, &mut eng);
    }

    #[divan::bench]
    fn tx_grouped(bencher: divan::Bencher) {
        let dir = TmpDir::new("txg-cs");
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        bench_write_and_spend(bencher, &mut eng);
    }
}

// ── Multi-output transaction grouping ───────────────────────────────────────

mod multi_output {
    use super::*;

    #[divan::bench(args = [2, 5, 10])]
    fn flat_lookup_same_tx(bencher: divan::Bencher, n_outputs: u32) {
        let dir = TmpDir::new("flat-mo");
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        seed_engine_multi_output(&mut eng, 1000, n_outputs);
        let ops: Vec<OutPoint> = (0..n_outputs)
            .map(|vout| make_outpoint_vout(500, vout))
            .collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&ops), &mut out).unwrap();
        });
    }

    #[divan::bench(args = [2, 5, 10])]
    fn robin_hood_lookup_same_tx(bencher: divan::Bencher, n_outputs: u32) {
        let dir = TmpDir::new("rh-mo");
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
        seed_engine_multi_output(&mut eng, 1000, n_outputs);
        let ops: Vec<OutPoint> = (0..n_outputs)
            .map(|vout| make_outpoint_vout(500, vout))
            .collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&ops), &mut out).unwrap();
        });
    }

    #[divan::bench(args = [2, 5, 10])]
    fn tx_grouped_lookup_same_tx(bencher: divan::Bencher, n_outputs: u32) {
        let dir = TmpDir::new("txg-mo");
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        seed_engine_multi_output(&mut eng, 1000, n_outputs);
        let ops: Vec<OutPoint> = (0..n_outputs)
            .map(|vout| make_outpoint_vout(500, vout))
            .collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&ops), &mut out).unwrap();
        });
    }
}

// ── Ordinals replay (real mainnet blocks) ───────────────────────────────────

mod ordinals_replay {
    use super::*;

    const START_HEIGHT: u32 = 767_430;
    const END_HEIGHT: u32 = 768_430;

    struct RealBlock {
        height: u32,
        header: BlockHeader,
        hash: Hash256,
        prev_hash: Hash256,
        created: Vec<(OutPoint, Satoshi, Vec<u8>, bool)>,
        spent: Vec<OutPoint>,
        new_coins: Vec<NewCoin<'static>>,
    }

    struct Corpus {
        blocks: Vec<RealBlock>,
        #[allow(dead_code)]
        pre_window: Vec<(OutPoint, Satoshi, Vec<u8>)>,
        seed_coins: Vec<NewCoin<'static>>,
    }

    fn ordinals_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("ordinals")
    }

    fn build_corpus() -> Corpus {
        let dir = ordinals_dir();
        let mut blocks = Vec::with_capacity(1001);
        let mut created_set: PrehashedSet<OutPoint> = PrehashedSet::default();
        let mut pre_window: Vec<(OutPoint, Satoshi, Vec<u8>)> = Vec::new();
        let mut pre_window_set: PrehashedSet<OutPoint> = PrehashedSet::default();
        let mut txid_buf = Vec::new();

        for h in START_HEIGHT..=END_HEIGHT {
            let path = dir.join("blocks").join(format!("{h:07}.bin"));
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => break,
            };
            let block = Block::decode(&mut std::io::Cursor::new(&bytes))
                .unwrap_or_else(|e| panic!("decode block {h}: {e}"));

            let header = block.header;
            let hash = header.hash();
            let prev_hash = header.prev_block();

            let mut created = Vec::new();
            let mut spent = Vec::new();

            for (tx_idx, tx) in block.transactions.iter().enumerate() {
                let txid = tx.txid_into(&mut txid_buf).expect("txid");
                let coinbase = tx_idx == 0;

                for (vout, out) in tx.outputs.iter().enumerate() {
                    let op = OutPoint {
                        txid,
                        vout: vout as u32,
                    };
                    created.push((op, out.value, out.script_pubkey.to_vec(), coinbase));
                    created_set.insert(op);
                }

                if !coinbase {
                    for inp in &tx.inputs {
                        let op = inp.previous_output;
                        spent.push(op);
                        if !created_set.contains(&op) && pre_window_set.insert(op) {
                            pre_window.push((
                                op,
                                Satoshi::new(1000),
                                vec![0x00, 0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                                     0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                                     0x0f, 0x10, 0x11, 0x12, 0x13, 0x14],
                            ));
                        }
                    }
                }
            }

            blocks.push(RealBlock {
                height: h,
                header,
                hash,
                prev_hash,
                created,
                spent,
                new_coins: Vec::new(),
            });
        }

        // Second pass: blocks vec is stable, build NewCoin slices that
        // borrow script bytes from the now-pinned created tuples.
        // SAFETY: the Corpus lives in a LazyLock — the blocks and their
        // created vecs are never moved or freed. The transmuted 'static
        // lifetime is valid for the program's duration.
        for blk in &mut blocks {
            blk.new_coins = blk
                .created
                .iter()
                .map(|(op, value, spk, coinbase)| {
                    let spk_ref: &[u8] = spk;
                    let spk_static: &'static [u8] =
                        unsafe { std::mem::transmute::<&[u8], &'static [u8]>(spk_ref) };
                    NewCoin {
                        outpoint: *op,
                        value: *value,
                        script_pubkey: spk_static,
                        coinbase: *coinbase,
                        height: BlockHeight::new(blk.height),
                    }
                })
                .collect();
        }

        let seed_coins: Vec<NewCoin<'static>> = pre_window
            .iter()
            .map(|(op, value, spk)| {
                let spk_ref: &[u8] = spk;
                let spk_static: &'static [u8] =
                    unsafe { std::mem::transmute::<&[u8], &'static [u8]>(spk_ref) };
                NewCoin {
                    outpoint: *op,
                    value: *value,
                    script_pubkey: spk_static,
                    coinbase: false,
                    height: BlockHeight::new(1),
                }
            })
            .collect();

        eprintln!(
            "ordinals corpus: {} blocks, {} pre-window UTXOs to seed",
            blocks.len(),
            pre_window.len()
        );
        Corpus { blocks, pre_window, seed_coins }
    }

    static CORPUS: LazyLock<Corpus> = LazyLock::new(build_corpus);

    fn seed_and_replay(eng: &mut dyn Engine) {
        let corpus = &*CORPUS;
        if corpus.blocks.is_empty() {
            return;
        }

        let seed_hash = corpus.blocks[0].prev_hash;
        let seed_height = BlockHeight::new(START_HEIGHT - 1);
        let seed_header = BlockHeader::new(
            BlockVersion::new(1),
            Hash256::ZERO,
            Hash256::ZERO,
            BlockTime::new(1),
            CompactBits::new(0x1d00_ffff),
            Nonce::new(0),
        );
        eng.write_block(&CommitBlockReq {
            token: ReqToken(0),
            height: seed_height,
            hash: seed_hash,
            prev_hash: Hash256::ZERO,
            header: seed_header,
            created: &corpus.seed_coins,
            spent: &[],
            block_bytes: None,
            durability: Durability::Memory,
        })
        .unwrap();

        for (i, rb) in corpus.blocks.iter().enumerate() {
            eng.write_block(&CommitBlockReq {
                #[allow(clippy::cast_possible_truncation)]
                token: ReqToken(i as u64 + 1),
                height: BlockHeight::new(rb.height),
                hash: rb.hash,
                prev_hash: rb.prev_hash,
                header: rb.header,
                created: &rb.new_coins,
                spent: &rb.spent,
                block_bytes: None,
                durability: Durability::Memory,
            })
            .unwrap();
        }
    }

    fn has_fixtures() -> bool {
        let dir = ordinals_dir();
        dir.join("blocks").is_dir()
            && dir
                .join("blocks")
                .join(format!("{START_HEIGHT:07}.bin"))
                .exists()
    }

    #[divan::bench(sample_count = 3)]
    fn flat(bencher: divan::Bencher) {
        if !has_fixtures() {
            eprintln!("ordinals fixtures missing, skipping");
            return;
        }
        LazyLock::force(&CORPUS);
        bencher.bench_local(|| {
            let dir = TmpDir::new("flat-ord");
            let mut eng = FlatEngine::open(dir.path()).unwrap();
            seed_and_replay(&mut eng);
        });
    }

    #[divan::bench(sample_count = 3)]
    fn robin_hood(bencher: divan::Bencher) {
        if !has_fixtures() {
            eprintln!("ordinals fixtures missing, skipping");
            return;
        }
        LazyLock::force(&CORPUS);
        bencher.bench_local(|| {
            let dir = TmpDir::new("rh-ord");
            let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
            seed_and_replay(&mut eng);
        });
    }

    #[divan::bench(sample_count = 3)]
    fn tx_grouped(bencher: divan::Bencher) {
        if !has_fixtures() {
            eprintln!("ordinals fixtures missing, skipping");
            return;
        }
        LazyLock::force(&CORPUS);
        bencher.bench_local(|| {
            let dir = TmpDir::new("txg-ord");
            let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
            seed_and_replay(&mut eng);
        });
    }

    #[divan::bench(sample_count = 3)]
    fn arena(bencher: divan::Bencher) {
        if !has_fixtures() {
            eprintln!("ordinals fixtures missing, skipping");
            return;
        }
        let corpus = &*CORPUS;
        bencher.bench_local(|| {
            let mut set = ArenaUtxoSet::new();
            let mut scratch = ConnectScratch::new();

            let undo = set
                .connect_block_with(&mut scratch, BlockHeight::new(1), &corpus.seed_coins, &[])
                .unwrap();
            scratch.reclaim(undo);

            for rb in &corpus.blocks {
                let undo = set.connect_block_best_effort(
                    &mut scratch,
                    BlockHeight::new(rb.height),
                    &rb.new_coins,
                    &rb.spent,
                );
                scratch.reclaim(undo);
            }
            black_box(&set);
        });
    }

    #[divan::bench(sample_count = 3)]
    fn arena_engine(bencher: divan::Bencher) {
        if !has_fixtures() {
            eprintln!("ordinals fixtures missing, skipping");
            return;
        }
        LazyLock::force(&CORPUS);
        bencher.bench_local(|| {
            let dir = TmpDir::new("arena-eng-ord");
            let mut eng = ArenaEngine::open(dir.path()).unwrap();
            seed_and_replay(&mut eng);
        });
    }

    #[divan::bench(sample_count = 10)]
    fn flat_read_after_replay(bencher: divan::Bencher) {
        if !has_fixtures() {
            return;
        }
        let corpus = &*CORPUS;
        if corpus.blocks.len() < 500 {
            return;
        }
        let dir = TmpDir::new("flat-ord-r");
        let mut eng = FlatEngine::open(dir.path()).unwrap();
        seed_and_replay(&mut eng);
        let lookup_ops: Vec<OutPoint> = corpus.blocks[500]
            .spent
            .iter()
            .take(100)
            .copied()
            .collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&lookup_ops), &mut out).unwrap();
        });
    }

    #[divan::bench(sample_count = 10)]
    fn robin_hood_read_after_replay(bencher: divan::Bencher) {
        if !has_fixtures() {
            return;
        }
        let corpus = &*CORPUS;
        if corpus.blocks.len() < 500 {
            return;
        }
        let dir = TmpDir::new("rh-ord-r");
        let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
        seed_and_replay(&mut eng);
        let lookup_ops: Vec<OutPoint> = corpus.blocks[500]
            .spent
            .iter()
            .take(100)
            .copied()
            .collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&lookup_ops), &mut out).unwrap();
        });
    }

    #[divan::bench(sample_count = 10)]
    fn tx_grouped_read_after_replay(bencher: divan::Bencher) {
        if !has_fixtures() {
            return;
        }
        let corpus = &*CORPUS;
        if corpus.blocks.len() < 500 {
            return;
        }
        let dir = TmpDir::new("txg-ord-r");
        let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
        seed_and_replay(&mut eng);
        let lookup_ops: Vec<OutPoint> = corpus.blocks[500]
            .spent
            .iter()
            .take(100)
            .copied()
            .collect();
        let mut out: Vec<LookupResult> = Vec::new();
        bencher.bench_local(|| {
            out.clear();
            eng.read_coins(black_box(&lookup_ops), &mut out).unwrap();
        });
    }
}

// ── Recovery (startup from WAL/log) ─────────────────────────────────────────

mod recover {
    use super::*;

    #[divan::bench]
    fn flat_500(bencher: divan::Bencher) {
        let dir = TmpDir::new("flat-rec");
        {
            let mut eng = FlatEngine::open(dir.path()).unwrap();
            seed_engine(&mut eng, 500);
            eng.flush(Durability::Synced).unwrap();
        }
        bencher.bench(|| {
            black_box(FlatEngine::open(dir.path()).unwrap());
        });
    }

    #[divan::bench]
    fn robin_hood_500(bencher: divan::Bencher) {
        let dir = TmpDir::new("rh-rec");
        {
            let mut eng = RobinHoodEngine::open(dir.path()).unwrap();
            seed_engine(&mut eng, 500);
            eng.flush(Durability::Synced).unwrap();
        }
        bencher.bench(|| {
            black_box(RobinHoodEngine::open(dir.path()).unwrap());
        });
    }

    #[divan::bench]
    fn tx_grouped_500(bencher: divan::Bencher) {
        let dir = TmpDir::new("txg-rec");
        {
            let mut eng = TxGroupedEngine::open(dir.path()).unwrap();
            seed_engine(&mut eng, 500);
            eng.flush(Durability::Synced).unwrap();
        }
        bencher.bench(|| {
            black_box(TxGroupedEngine::open(dir.path()).unwrap());
        });
    }
}
