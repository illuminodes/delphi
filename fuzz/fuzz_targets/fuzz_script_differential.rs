#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mantle_conformance::conformance;
use mantle_consensus::validation::validate_block_structure;
use mantle::codec::{Decode, Encode};
use mantle::primitives::{Block, BlockHeader, Hash256, Transaction, TxOut};
use std::io::Cursor;

/// Mask to the flag bits bitcoinconsensus v26 accepts.
const VALID_FLAGS_MASK: u32 = conformance::VERIFY_P2SH
    | conformance::VERIFY_DERSIG
    | conformance::VERIFY_NULLDUMMY
    | conformance::VERIFY_CHECKLOCKTIMEVERIFY
    | conformance::VERIFY_CHECKSEQUENCEVERIFY
    | conformance::VERIFY_WITNESS
    | conformance::VERIFY_TAPROOT;

/// One entry in the spent-outputs table: the script_pubkey and amount for a
/// single input's corresponding UTXO.
#[derive(Arbitrary, Debug)]
struct SpentOutput {
    script_pubkey: Vec<u8>,
    amount: u64,
}

/// Full differential fuzz input.
///
/// `tx_bytes` is fed through Mantle's `Transaction` codec first. If it parses,
/// the re-encoded bytes are used as the oracle input — exercising both
/// codec correctness and the conformance oracle on every valid input.
///
/// `flags` is masked to bits bitcoinconsensus v26 accepts.
#[derive(Arbitrary, Debug)]
struct FuzzInput {
    tx_bytes: Vec<u8>,
    spent_outputs: Vec<SpentOutput>,
    flags: u32,
}

fuzz_target!(|input: FuzzInput| {
    let flags = input.flags & VALID_FLAGS_MASK;

    // ── Step 1: decode through Mantle's codec ─────────────────────────────────
    let tx = match Transaction::decode(&mut Cursor::new(&input.tx_bytes)) {
        Ok(tx) => tx,
        Err(_) => return,
    };

    // ── Step 2: re-encode via Mantle to get canonical wire bytes ──────────────
    // encode(decode(x)) must not panic — that would be a codec bug.
    let mut tx_wire = Vec::new();
    tx.encode(&mut tx_wire)
        .expect("encode must not fail on a decoded Transaction");

    // ── Step 3: Mantle structural check on this transaction ───────────────────
    // Wrap it in a minimal block so validate_block_structure can check it.
    // We deliberately use an easy nBits (regtest) and set the merkle root
    // correctly so only the tx-level checks are exercised here, not PoW.
    let txid = mantle_consensus::validation::txid(&tx);
    let merkle_root = mantle_consensus::merkle::compute_merkle_root(&[txid])
        .expect("single-element slice is never empty");

    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 1_231_006_505,
            bits: 0x207f_ffff, // regtest easiest target
            nonce: 0,
        },
        transactions: vec![tx.clone()],
    };
    // We expect validate_block_structure to either accept or reject — never panic.
    let _mantle_block_verdict = validate_block_structure(&block);

    // ── Step 4: oracle (libbitcoinconsensus) per-input verification ───────────
    for (i, spent) in input.spent_outputs.iter().enumerate().take(tx.inputs.len()) {
        // Exercise TxOut codec on each spent output.
        let txout = TxOut {
            value: spent.amount,
            script_pubkey: spent.script_pubkey.clone(),
        };
        let mut txout_wire = Vec::new();
        txout
            .encode(&mut txout_wire)
            .expect("TxOut encode must not fail");
        let txout_rt = TxOut::decode(&mut Cursor::new(&txout_wire))
            .expect("TxOut decode must not fail on freshly encoded bytes");
        assert_eq!(txout.value, txout_rt.value);
        assert_eq!(txout.script_pubkey, txout_rt.script_pubkey);

        let oracle = conformance::verify_script(
            &spent.script_pubkey,
            spent.amount,
            &tx_wire,
            None, // taproot inputs return ERR_SPENT_OUTPUTS_REQUIRED from both sides
            i,
            flags,
        );

        // ── Mantle script interpreter (not yet implemented) ───────────────────
        // Once mantle-consensus exposes verify_script, replace this with:
        //
        //   let mantle = mantle_consensus::verify_script(
        //       &spent.script_pubkey,
        //       spent.amount,
        //       &tx,
        //       i,
        //       flags,
        //   );
        //   assert_eq!(
        //       oracle.is_ok(),
        //       mantle.is_ok(),
        //       "divergence at input {i}: oracle={oracle:?} mantle={mantle:?}\n\
        //        tx={tx:?} flags={flags:#010x}",
        //   );

        let _ = oracle;
    }
});
