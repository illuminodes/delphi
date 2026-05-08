//! Replay perf — workload B only: block with 50 UTXO spends.
//!
//! Fund coinbase at height 1 with 50 outputs; 100 filler blocks for maturity;
//! spend block at height 102.  Setup chain re-committed each iter.
#![allow(clippy::cast_precision_loss)]

use std::hint::black_box;
use std::time::Instant;

use mantle::primitives::{
    BlockHeight, Hash256, OutPoint, Satoshi, Transaction, TxHash, TxIn, TxOut, Witness,
};
use mantle_consensus::script::ScriptExecutionCache;

#[path = "replay_perf_common.rs"]
mod common;
use common::{
    coinbase_tx, commit, make_block, make_snapshot, run_for, spend_tx, tmp_storage, SECS,
};

fn main() {
    const N_SPENDS: usize = 50;
    const MATURITY: u32 = 100;
    const FUND_H: u32 = 1;
    const SPEND_H: u32 = FUND_H + MATURITY + 1;

    let fund_cb = Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: vec![0x03, 0x01, 0x00, 0x00].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: (0..N_SPENDS).map(|_| TxOut {
            value: Satoshi::new(1000),
            script_pubkey: vec![0x51].into(),
        }).collect(),
        locktime: 0,
    };
    let fund_block = make_block(Hash256::ZERO, FUND_H, vec![fund_cb]);
    let fund_txid = fund_block.transactions[0].txid().expect("txid");
    let fund_hash = fund_block.header.hash();

    let mut prev_hash = fund_hash;
    let mut filler: Vec<_> = Vec::new();
    for h in 2..SPEND_H {
        let b = make_block(prev_hash, h, vec![coinbase_tx(h, 50_0000_0000)]);
        prev_hash = b.header.hash();
        filler.push(b);
    }

    let spend_txs: Vec<Transaction> = std::iter::once(coinbase_tx(SPEND_H, 50_0000_0000))
        .chain((0..N_SPENDS).map(|i| spend_tx(fund_txid, u32::try_from(i).expect("fits"), 999)))
        .collect();
    let spend_block = make_block(prev_hash, SPEND_H, spend_txs);
    let spend_hash = spend_block.header.hash();

    let mut chain = vec![(fund_hash, fund_block.header)];
    for b in &filler { chain.push((b.header.hash(), b.header)); }
    chain.push((spend_hash, spend_block.header));
    let snap = make_snapshot(&chain);

    let mut iter_idx = 0u32;
    let workload_start = Instant::now();
    eprintln!("[B] workload begin t=0");
    let n = run_for(SECS, || {
        let it_start = Instant::now();
        eprint!("[B] {iter_idx} ");
        let (mut storage, _dir) = tmp_storage();
        let mut cache = ScriptExecutionCache::new(1024);
        eprint!("F ");
        commit(&mut storage, &fund_block, BlockHeight(FUND_H), &snap, &mut cache, 1);
        for (i, b) in filler.iter().enumerate() {
            eprint!("f{i} ");
            commit(&mut storage, b, BlockHeight(2 + u32::try_from(i).expect("fits")),
                &snap, &mut cache, 2 + i as u64);
        }
        eprint!("S ");
        commit(&mut storage, black_box(&spend_block), BlockHeight(SPEND_H),
            &snap, &mut cache, u64::from(SPEND_H));
        eprint!("X ");
        storage.shutdown().expect("shutdown");
        let dt = it_start.elapsed();
        eprintln!("dt={:.1} wall={:.2}",
            dt.as_secs_f64() * 1000.0,
            workload_start.elapsed().as_secs_f64());
        iter_idx += 1;
    });
    eprintln!("[B] workload end wall={:.2}s n={n}", workload_start.elapsed().as_secs_f64());
    eprintln!("replay_50_spends:     {n} iters  ({:.0} blocks/s)",
        n as f64 / SECS.as_secs_f64());
}
