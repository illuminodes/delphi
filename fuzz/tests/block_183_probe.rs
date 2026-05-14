//! Diagnose the script-verification failure at mainnet block 183.
//!
//! `mantle-replay --start-height 1 --stop-height 100000` against a freshly-
//! downloaded mainnet cache fails at:
//!
//!   fatal: validate_and_commit at 183: validation: ScriptVerifyFailed
//!     { tx_index: 1, input_index: 0, kind: EvalFalse }
//!
//! This test loads the cached block files from `/tmp/mantle-smoke/blocks/`
//! (the same data dir the smoke run used), walks the chain to build a
//! `txid → (vout → TxOut)` map, then for *every* non-coinbase input in
//! block 183 calls the same differential machinery the fuzz target uses.
//! Whichever input diverges, the assertion in `run_differential` will fire
//! with a printable wire dump.
//!
//! Running:
//!     cargo test -p mantle-fuzz --test block_183_probe -- --ignored --nocapture
//!
//! Marked `#[ignore]` because it depends on an external fixture cache. CI
//! does not run it; we use it locally to walk a known divergence into a
//! crash file we then drop into `tests/regression/`.

use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use mantle::codec::{Decode, Encode};
use mantle::primitives::{Block, BlockHeight, Hash256, OutPoint, TxHash, TxOut};
use mantle_consensus::script::ScriptFlags;
use mantle_fuzz::{FuzzInput, SpentOutput, run_differential};

const SMOKE_DATA_DIR: &str = "/tmp/mantle-smoke";

fn block_path(height: u32) -> PathBuf {
    PathBuf::from(SMOKE_DATA_DIR)
        .join("blocks")
        .join(format!("{height:07}.bin"))
}

fn read_block(height: u32) -> Block {
    let bytes = fs::read(block_path(height)).unwrap_or_else(|e| panic!("read block {height}: {e}"));
    Block::decode(&mut Cursor::new(&bytes)).unwrap_or_else(|e| panic!("decode block {height}: {e}"))
}

#[test]
#[ignore = "depends on /tmp/mantle-smoke/blocks/ — see file header"]
fn block_183_input_divergence() {
    // Build a UTXO map by replaying blocks 1..=182 (without script verification)
    // so we have the spent_outputs for block 183's inputs. Apply spends as we go.
    let mut utxos: HashMap<OutPoint, TxOut> = HashMap::new();

    for h in 1u32..=182 {
        let block = read_block(h);
        for tx in &block.transactions {
            // Apply spends.
            let txid = tx.txid().expect("txid");
            // Coinbase inputs reference 0/u32::MAX — skip them.
            for inp in &tx.inputs {
                if inp.previous_output.txid == Hash256::ZERO && inp.previous_output.vout == u32::MAX
                {
                    continue;
                }
                utxos.remove(&inp.previous_output);
            }
            // Add new outputs.
            for (vout, out) in tx.outputs.iter().enumerate() {
                let op = OutPoint {
                    txid,
                    vout: u32::try_from(vout).expect("vout < u32::MAX"),
                };
                utxos.insert(op, out.clone());
            }
        }
    }

    // Now block 183.
    let block = read_block(183);
    assert!(
        block.transactions.len() >= 2,
        "block 183 has at least one non-coinbase tx"
    );

    let flags_bits = ScriptFlags::for_height(BlockHeight::new(183)).bits();
    eprintln!("block 183 script flags = {flags_bits:#010x}");

    for (tx_idx, tx) in block.transactions.iter().enumerate().skip(1) {
        eprintln!(
            "── block 183 tx {tx_idx} (txid {})",
            tx.txid().expect("txid").to_display_hex()
        );

        let mut tx_wire = Vec::new();
        tx.encode(&mut tx_wire).expect("encode");

        // Build the spent_outputs slice in input order.
        let mut spent_outputs: Vec<SpentOutput> = Vec::with_capacity(tx.inputs.len());
        for (i, inp) in tx.inputs.iter().enumerate() {
            let utxo = utxos.get(&inp.previous_output).unwrap_or_else(|| {
                panic!(
                    "block 183 tx {tx_idx} input {i}: prevout {}:{} not in our UTXO set",
                    inp.previous_output.txid.to_display_hex(),
                    inp.previous_output.vout,
                )
            });
            eprintln!(
                "  input {i}: spends {}:{} value={} spk_len={} spk={:02x?}",
                inp.previous_output.txid.to_display_hex(),
                inp.previous_output.vout,
                utxo.value.to_u64(),
                utxo.script_pubkey.as_slice().len(),
                utxo.script_pubkey.as_slice(),
            );
            spent_outputs.push(SpentOutput {
                script_pubkey: utxo.script_pubkey.as_slice().to_vec(),
                amount: utxo.value.to_u64(),
            });
        }

        let input = FuzzInput {
            tx_bytes: tx_wire,
            spent_outputs,
            flags: flags_bits,
        };

        // run_differential will panic loudly on divergence with the same
        // format the fuzz target uses. If it returns cleanly, no divergence.
        run_differential(&input);
    }

    eprintln!("no divergence — surprising; the smoke run reported one");
}
