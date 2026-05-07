#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mantle::codec::{Decode, Encode};
use mantle::primitives::{Block, BlockHeader, Hash256, Satoshi, Transaction, TxHash, TxOut};
use mantle_conformance::conformance;
use mantle_consensus::script::{ScriptFlags, verify_script};
use mantle_consensus::validation::validate_block_structure;
use std::io::Cursor;

/// Mask to the flag bits we currently assert parity on.
///
/// **M1**: legacy script — P2SH, BIP66 (DERSIG), NULLDUMMY, CLTV, CSV.
/// Witness/taproot bits are still passed *through* the corpus filter (we
/// skip cases that exercise them), but never asserted on.
const M1_FLAGS_MASK: u32 = conformance::VERIFY_P2SH
    | conformance::VERIFY_DERSIG
    | conformance::VERIFY_NULLDUMMY
    | conformance::VERIFY_CHECKLOCKTIMEVERIFY
    | conformance::VERIFY_CHECKSEQUENCEVERIFY;

/// Bits that take this input out of M1 scope. If either side of the diff
/// returns a witness/taproot-shaped error we skip the case rather than
/// trying to compare verdicts on machinery that isn't implemented.
const OUT_OF_SCOPE_FLAGS: u32 = conformance::VERIFY_WITNESS | conformance::VERIFY_TAPROOT;

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
    // Strip witness/taproot — those activate in M2/M3. The mask we *assert*
    // on is the M1 subset; we deliberately don't even pass witness bits to
    // the oracle, because a witness-shaped failure isn't actionable here.
    let flags = input.flags & M1_FLAGS_MASK;
    let mantle_flags = ScriptFlags::from_bits(flags);

    // ── Step 1: decode through Mantle's codec ─────────────────────────────────
    let tx = match Transaction::decode(&mut Cursor::new(&input.tx_bytes)) {
        Ok(tx) => tx,
        Err(_) => return,
    };

    // Coinbase inputs are never script-verified. Their scriptSig is opaque
    // and the oracle / Mantle would diverge meaninglessly on them.
    if tx
        .inputs
        .iter()
        .any(|i| i.previous_output.txid == Hash256::ZERO && i.previous_output.vout == u32::MAX)
    {
        return;
    }

    // ── Step 2: re-encode via Mantle to get canonical wire bytes ──────────────
    // encode(decode(x)) must not panic — that would be a codec bug.
    let mut tx_wire = Vec::new();
    tx.encode(&mut tx_wire)
        .expect("encode must not fail on a decoded Transaction");

    // ── Step 3: Mantle structural check on this transaction ───────────────────
    // Wrap it in a minimal block so validate_block_structure can check it.
    // We deliberately use an easy nBits (regtest) and set the merkle root
    // correctly so only the tx-level checks are exercised here, not PoW.
    let txid = tx.txid().expect("txid encoding cannot fail");
    let merkle_root = mantle_consensus::merkle::compute_merkle_root(&[txid])
        .expect("single-element slice is never empty");

    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 1_231_006_505,
            bits: mantle::primitives::CompactBits(0x207f_ffff), // regtest easiest target
            nonce: 0,
        },
        transactions: vec![tx.clone()],
    };
    // We expect validate_block_structure to either accept or reject — never panic.
    let _mantle_block_verdict = validate_block_structure(&block);

    // ── Step 4: per-input differential ────────────────────────────────────────
    // For every input that has a corresponding spent_output in the corpus,
    // compare oracle vs. mantle on that input's script verification.
    for (i, spent) in input.spent_outputs.iter().enumerate().take(tx.inputs.len()) {
        // Exercise TxOut codec on each spent output.
        let txout = TxOut {
            value: Satoshi::new(spent.amount),
            script_pubkey: spent.script_pubkey.clone().into(),
        };
        let mut txout_wire = Vec::new();
        txout
            .encode(&mut txout_wire)
            .expect("TxOut encode must not fail");
        let txout_rt = TxOut::decode(&mut Cursor::new(&txout_wire))
            .expect("TxOut decode must not fail on freshly encoded bytes");
        assert_eq!(txout.value, txout_rt.value);
        assert_eq!(txout.script_pubkey, txout_rt.script_pubkey);

        // Skip cases that the input *intended* to exercise witness/taproot.
        // We can't tell up-front from a raw scriptPubKey, but we can detect
        // the obvious shapes: 22-byte v0 P2WPKH (`0x00 0x14 …`) and 34-byte
        // v0 P2WSH (`0x00 0x20 …`), 34-byte v1 taproot (`0x51 0x20 …`).
        if is_witness_program(&spent.script_pubkey) {
            continue;
        }

        let oracle = conformance::verify_script(
            &spent.script_pubkey,
            spent.amount,
            &tx_wire,
            None,
            i,
            flags,
        );

        let mantle = verify_script(
            tx.inputs[i].script_sig.as_slice(),
            &spent.script_pubkey,
            &tx,
            i,
            Satoshi::new(spent.amount),
            mantle_flags,
        );

        // If either side reports a witness/taproot-shaped error, treat as
        // out of scope. The oracle's error type is opaque (a `String`) so
        // we substring-match for keywords; mantle has no witness errors
        // reachable in M1 today, but be defensive.
        if let Err(ref e) = oracle {
            let s = format!("{e}");
            if s.contains("witness")
                || s.contains("Witness")
                || s.contains("taproot")
                || s.contains("Taproot")
                || s.contains("Schnorr")
            {
                continue;
            }
        }

        assert_eq!(
            oracle.is_ok(),
            mantle.is_ok(),
            "M1 divergence at input {i}: oracle={oracle:?} mantle={mantle:?}\n\
             tx_wire={tx_wire:02x?}\n\
             script_pubkey={:02x?}\n\
             flags={flags:#010x}",
            spent.script_pubkey,
        );
    }

    // Also ensure we don't silently leak: if an out-of-scope flag bit is
    // set in the *original* input, this assertion never ran for those bits.
    let _ = OUT_OF_SCOPE_FLAGS; // referenced for documentation; behaviour is masked above
});

/// True if `script` looks like a segwit witness program — those activate in
/// M2/M3 and aren't part of M1's assertion surface.
fn is_witness_program(script: &[u8]) -> bool {
    // BIP141: `<version 0x00 or OP_1..OP_16> <push 2..40 bytes>`.
    if script.len() < 4 || script.len() > 42 {
        return false;
    }
    let version_ok = script[0] == 0x00 || (0x51..=0x60).contains(&script[0]);
    if !version_ok {
        return false;
    }
    let push_len = script[1];
    if !(0x02..=0x28).contains(&push_len) {
        return false;
    }
    script.len() == 2 + push_len as usize
}
