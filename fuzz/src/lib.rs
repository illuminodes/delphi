//! Shared logic between `fuzz_script_differential` and its regression tests.
//!
//! The fuzz target loops on random inputs; the regression test replays
//! known crash bytes. Both reduce to the same per-input differential
//! between mantle and bitcoinconsensus, so it lives here.

use arbitrary::{Arbitrary, Unstructured};
use mantle::codec::{Decode, Encode};
use mantle::primitives::{Hash256, Satoshi, Transaction, TxOut};
use mantle_conformance::conformance;
use mantle_consensus::script::{ScriptFlags, verify_script};
use std::io::Cursor;

/// Mask to the flag bits we currently assert parity on.
///
/// **M1**: legacy script — P2SH, BIP66 (DERSIG), NULLDUMMY, CLTV, CSV.
pub const M1_FLAGS_MASK: u32 = conformance::VERIFY_P2SH
    | conformance::VERIFY_DERSIG
    | conformance::VERIFY_NULLDUMMY
    | conformance::VERIFY_CHECKLOCKTIMEVERIFY
    | conformance::VERIFY_CHECKSEQUENCEVERIFY;

/// One entry in the spent-outputs table: the script_pubkey and amount for a
/// single input's corresponding UTXO.
#[derive(Arbitrary, Debug)]
pub struct SpentOutput {
    pub script_pubkey: Vec<u8>,
    pub amount: u64,
}

/// Full differential fuzz input.
#[derive(Arbitrary, Debug)]
pub struct FuzzInput {
    pub tx_bytes: Vec<u8>,
    pub spent_outputs: Vec<SpentOutput>,
    pub flags: u32,
}

impl FuzzInput {
    /// Decode an `Arbitrary`-encoded `FuzzInput` from raw bytes — what
    /// libFuzzer hands us, what cargo-fuzz writes to crash files, and
    /// what regression-test fixtures store.
    ///
    /// # Errors
    /// `arbitrary::Error` if the bytes don't satisfy `Arbitrary`'s
    /// derivation (rare — `Vec<u8>` and `u32` are very permissive).
    pub fn from_corpus_bytes(bytes: &[u8]) -> Result<Self, arbitrary::Error> {
        let u = Unstructured::new(bytes);
        Self::arbitrary_take_rest(u)
    }
}

/// Run one differential round on a `FuzzInput`. Used by the fuzz target
/// and by `tests/regression.rs`.
///
/// On any divergence, panics with the same message format the fuzz
/// target produces — so a regression test failure reads exactly like
/// the crash that caused it.
pub fn run_differential(input: &FuzzInput) {
    let flags = input.flags & M1_FLAGS_MASK;
    let mantle_flags = ScriptFlags::from_bits(flags);

    let tx = match Transaction::decode(&mut Cursor::new(&input.tx_bytes)) {
        Ok(tx) => tx,
        Err(_) => return,
    };

    // Coinbase inputs are never script-verified.
    if tx
        .inputs
        .iter()
        .any(|i| i.previous_output.txid == Hash256::ZERO && i.previous_output.vout == u32::MAX)
    {
        return;
    }

    let mut tx_wire = Vec::new();
    tx.encode(&mut tx_wire)
        .expect("encode must not fail on a decoded Transaction");

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
}

/// True if `script` looks like a segwit witness program — those activate
/// in M2/M3 and aren't part of M1's assertion surface.
fn is_witness_program(script: &[u8]) -> bool {
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
