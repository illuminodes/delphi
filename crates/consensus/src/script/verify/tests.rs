use super::*;

use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature as EcdsaSig, SigningKey};
use mantle::primitives::{Hash256, OutPoint, TxIn, TxOut, Witness};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

use crate::script::sighash::{SigHashType, TxSighash};

// ── End-to-end signing helpers ─────────────────────────────────────────

fn key(secret: [u8; 32]) -> SigningKey {
    SigningKey::from_bytes(&secret.into()).expect("nonzero secret")
}

fn compressed_pubkey(sk: &SigningKey) -> Vec<u8> {
    sk.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn hash160(data: &[u8]) -> [u8; 20] {
    let mid = Sha256::digest(data);
    Ripemd160::digest(mid).into()
}

fn sign(sk: &SigningKey, sighash: &[u8; 32], hashtype: SigHashType) -> Vec<u8> {
    let sig: EcdsaSig = sk
        .sign_prehash(sighash)
        .expect("signing a 32-byte digest cannot fail");
    let sig = sig.normalize_s().unwrap_or(sig); // canonical low-S
    let mut der = sig.to_der().as_bytes().to_vec();
    der.push(u8::try_from(hashtype.0 & 0xff).expect("hashtype fits in u8"));
    der
}

fn make_tx() -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256([0u8; 32]),
                vout: 0,
            },
            script_sig: Vec::<u8>::new().into(),
            sequence: 0xffff_ffff,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(50_000),
            script_pubkey: vec![0x6a].into(),
        }],
        locktime: 0,
    }
}

/// Standard P2PKH: `OP_DUP OP_HASH160 <20> <hash> OP_EQUALVERIFY OP_CHECKSIG`.
fn p2pkh_spk(pkh: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x76, 0xa9, 0x14];
    s.extend_from_slice(pkh);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

fn standard_flags() -> ScriptFlags {
    ScriptFlags::P2SH
        | ScriptFlags::DERSIG
        | ScriptFlags::LOW_S
        | ScriptFlags::STRICTENC
        | ScriptFlags::NULLDUMMY
}

// ── End-to-end ─────────────────────────────────────────────────────────

#[test]
fn p2pkh_succeeds() {
    let sk = key([7u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let tx = make_tx();
    let spk = p2pkh_spk(&pkh);
    let digest = tx.legacy_sighash(0, &spk, SigHashType::ALL);
    let sig = sign(&sk, &digest.0, SigHashType::ALL);

    let mut script_sig = vec![u8::try_from(sig.len()).expect("DER ≤ 72")];
    script_sig.extend_from_slice(&sig);
    script_sig.push(u8::try_from(pk.len()).expect("compressed ≤ 33"));
    script_sig.extend_from_slice(&pk);

    verify_script(&script_sig, &spk, &tx, 0, Satoshi::ZERO, standard_flags())
        .expect("p2pkh should verify");
}

#[test]
fn p2pkh_fails_with_wrong_signature() {
    let sk = key([7u8; 32]);
    let attacker_sk = key([8u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let tx = make_tx();
    let spk = p2pkh_spk(&pkh);

    // Sign with the *attacker* key but submit the real pubkey — CHECKSIG
    // verifies against the real pubkey and fails.
    let digest = tx.legacy_sighash(0, &spk, SigHashType::ALL);
    let sig = sign(&attacker_sk, &digest.0, SigHashType::ALL);

    let mut script_sig = vec![u8::try_from(sig.len()).expect("DER ≤ 72")];
    script_sig.extend_from_slice(&sig);
    script_sig.push(u8::try_from(pk.len()).expect("compressed ≤ 33"));
    script_sig.extend_from_slice(&pk);

    // Without NULLFAIL the failure is `EvalFalse` (CHECKSIG pushes false,
    // OP_EQUALVERIFY long since passed because the pubkey hash matched).
    // Actually CHECKSIG runs last → false on top → EvalFalse.
    let r = verify_script(&script_sig, &spk, &tx, 0, Satoshi::ZERO, standard_flags());
    assert!(matches!(r, Err(ScriptError::EvalFalse)));
}

#[test]
fn p2pkh_fails_with_empty_stack() {
    // Empty scriptSig + non-trivial scriptPubKey → final stack falsey.
    let pkh = [0u8; 20];
    let spk = p2pkh_spk(&pkh);
    let tx = make_tx();
    let r = verify_script(&[], &spk, &tx, 0, Satoshi::ZERO, standard_flags());
    // OP_DUP on empty stack → InvalidStackOperation.
    assert!(matches!(r, Err(ScriptError::InvalidStackOperation)));
}

#[test]
fn sigpushonly_rejects_non_push_in_scriptsig() {
    // scriptSig contains OP_NOP (0x61) — not a push.
    let pkh = [0u8; 20];
    let spk = p2pkh_spk(&pkh);
    let tx = make_tx();
    let r = verify_script(
        &[0x61],
        &spk,
        &tx,
        0,
        Satoshi::ZERO,
        standard_flags() | ScriptFlags::SIGPUSHONLY,
    );
    assert!(matches!(r, Err(ScriptError::SigPushOnly)));
}

// ── P2SH ───────────────────────────────────────────────────────────────

#[test]
fn p2sh_wrapped_p2pkh_succeeds() {
    // Redeem script is a standard P2PKH; outer scriptPubKey is its P2SH wrap.
    let sk = key([42u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let redeem = p2pkh_spk(&pkh);
    let redeem_hash = hash160(&redeem);

    let mut spk = vec![0xa9, 0x14];
    spk.extend_from_slice(&redeem_hash);
    spk.push(0x87);
    assert!(is_p2sh(&spk));

    let tx = make_tx();
    // For BIP16, sighash is computed against the *redeem* script, not
    // the P2SH-wrapped outer script.
    let digest = tx.legacy_sighash(0, &redeem, SigHashType::ALL);
    let sig = sign(&sk, &digest.0, SigHashType::ALL);

    // scriptSig = <sig> <pk> <redeem>
    let mut script_sig = vec![u8::try_from(sig.len()).expect("DER ≤ 72")];
    script_sig.extend_from_slice(&sig);
    script_sig.push(u8::try_from(pk.len()).expect("compressed ≤ 33"));
    script_sig.extend_from_slice(&pk);
    script_sig.push(u8::try_from(redeem.len()).expect("redeem ≤ 75 here"));
    script_sig.extend_from_slice(&redeem);

    verify_script(&script_sig, &spk, &tx, 0, Satoshi::ZERO, standard_flags())
        .expect("p2sh-wrapped p2pkh should verify");
}

#[test]
fn p2sh_fails_when_redeem_hash_mismatches() {
    let sk = key([42u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let redeem = p2pkh_spk(&pkh);
    // Use a *different* hash than the actual redeem to break the OP_EQUAL.
    let bogus_hash = [0u8; 20];

    let mut spk = vec![0xa9, 0x14];
    spk.extend_from_slice(&bogus_hash);
    spk.push(0x87);

    let tx = make_tx();
    let digest = tx.legacy_sighash(0, &redeem, SigHashType::ALL);
    let sig = sign(&sk, &digest.0, SigHashType::ALL);

    let mut script_sig = vec![u8::try_from(sig.len()).expect("DER ≤ 72")];
    script_sig.extend_from_slice(&sig);
    script_sig.push(u8::try_from(pk.len()).expect("compressed ≤ 33"));
    script_sig.extend_from_slice(&pk);
    script_sig.push(u8::try_from(redeem.len()).expect("redeem fits"));
    script_sig.extend_from_slice(&redeem);

    let r = verify_script(&script_sig, &spk, &tx, 0, Satoshi::ZERO, standard_flags());
    // OP_HASH160 == bogus → false on top → EvalFalse before P2SH path runs.
    assert!(matches!(r, Err(ScriptError::EvalFalse)));
}

#[test]
fn p2sh_off_falls_back_to_legacy_eval() {
    // Without P2SH, the scriptPubKey is treated as a literal script. With
    // a redeem-script-shaped scriptSig pushing a valid P2PKH-shaped redeem
    // and the outer P2SH template, the `OP_HASH160 <hash> OP_EQUAL` *just
    // checks the hash equality* and pushes true — no inner eval.
    let sk = key([42u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let redeem = p2pkh_spk(&pkh);
    let redeem_hash = hash160(&redeem);

    let mut spk = vec![0xa9, 0x14];
    spk.extend_from_slice(&redeem_hash);
    spk.push(0x87);

    let tx = make_tx();
    // scriptSig = <redeem> only — push the redeem, scriptPubKey hashes
    // and compares it. No signature verification.
    let mut script_sig = vec![u8::try_from(redeem.len()).expect("redeem fits")];
    script_sig.extend_from_slice(&redeem);

    // P2SH off; legacy semantics. The OP_EQUAL pushes true, end of
    // story.
    let flags = standard_flags() - ScriptFlags::P2SH;
    verify_script(&script_sig, &spk, &tx, 0, Satoshi::ZERO, flags)
        .expect("legacy eval of P2SH-shaped scriptPubKey is just hash equality");
}

// ── CLEANSTACK ─────────────────────────────────────────────────────────

#[test]
fn cleanstack_passes_when_only_one_item_remains_after_p2sh() {
    // Reuse the P2SH-wrapped P2PKH success case and turn CLEANSTACK on.
    let sk = key([42u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let redeem = p2pkh_spk(&pkh);
    let redeem_hash = hash160(&redeem);

    let mut spk = vec![0xa9, 0x14];
    spk.extend_from_slice(&redeem_hash);
    spk.push(0x87);

    let tx = make_tx();
    let digest = tx.legacy_sighash(0, &redeem, SigHashType::ALL);
    let sig = sign(&sk, &digest.0, SigHashType::ALL);

    let mut script_sig = vec![u8::try_from(sig.len()).expect("DER ≤ 72")];
    script_sig.extend_from_slice(&sig);
    script_sig.push(u8::try_from(pk.len()).expect("compressed ≤ 33"));
    script_sig.extend_from_slice(&pk);
    script_sig.push(u8::try_from(redeem.len()).expect("redeem fits"));
    script_sig.extend_from_slice(&redeem);

    verify_script(
        &script_sig,
        &spk,
        &tx,
        0,
        Satoshi::ZERO,
        standard_flags() | ScriptFlags::CLEANSTACK,
    )
    .expect("clean stack at end");
}

#[test]
fn cleanstack_fails_when_extra_item_remains() {
    // Push a junk item before the legitimate P2PKH unlock — the final
    // stack will have 2 items, violating CLEANSTACK.
    let sk = key([42u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let redeem = p2pkh_spk(&pkh);
    let redeem_hash = hash160(&redeem);

    let mut spk = vec![0xa9, 0x14];
    spk.extend_from_slice(&redeem_hash);
    spk.push(0x87);

    let tx = make_tx();
    let digest = tx.legacy_sighash(0, &redeem, SigHashType::ALL);
    let sig = sign(&sk, &digest.0, SigHashType::ALL);

    // Junk push at the front of scriptSig. Pure data push so SIGPUSHONLY
    // doesn't catch it — the offence is purely "extra item left over".
    let mut script_sig = vec![0x01, 0xff];
    script_sig.push(u8::try_from(sig.len()).expect("DER ≤ 72"));
    script_sig.extend_from_slice(&sig);
    script_sig.push(u8::try_from(pk.len()).expect("compressed ≤ 33"));
    script_sig.extend_from_slice(&pk);
    script_sig.push(u8::try_from(redeem.len()).expect("redeem fits"));
    script_sig.extend_from_slice(&redeem);

    let r = verify_script(
        &script_sig,
        &spk,
        &tx,
        0,
        Satoshi::ZERO,
        standard_flags() | ScriptFlags::CLEANSTACK,
    );
    assert!(matches!(r, Err(ScriptError::CleanStack)));
}

// ── is_push_only ────────────────────────────────────────────────────────

#[test]
fn push_only_accepts_data_push() {
    // OP_PUSHBYTES_2 0xab 0xcd
    assert!(is_push_only(&[0x02, 0xab, 0xcd]));
}

#[test]
fn push_only_accepts_op0_op1_op16() {
    assert!(is_push_only(&[0x00])); // OP_0
    assert!(is_push_only(&[0x4f])); // OP_1NEGATE
    assert!(is_push_only(&[0x51, 0x60])); // OP_1, OP_16
}

#[test]
fn push_only_rejects_arithmetic() {
    // OP_1 OP_2 OP_ADD — has a non-push.
    assert!(!is_push_only(&[0x51, 0x52, 0x93]));
}

#[test]
fn push_only_rejects_truncated_push() {
    // OP_PUSHBYTES_3 0xab — declares 3 bytes, only 1 supplied.
    assert!(!is_push_only(&[0x03, 0xab]));
}

#[test]
fn push_only_accepts_pushdata1() {
    assert!(is_push_only(&[0x4c, 0x01, 0xaa]));
}

// ── is_p2sh ─────────────────────────────────────────────────────────────

#[test]
fn p2sh_template_match() {
    let mut s = vec![0xa9, 0x14];
    s.extend_from_slice(&[0xab; 20]);
    s.push(0x87);
    assert!(is_p2sh(&s));
}

#[test]
fn p2sh_template_wrong_length() {
    // 22 bytes — short by one.
    let mut s = vec![0xa9, 0x14];
    s.extend_from_slice(&[0xab; 19]);
    s.push(0x87);
    assert!(!is_p2sh(&s));
}

#[test]
fn p2sh_template_wrong_first_byte() {
    let mut s = vec![0x76, 0x14]; // OP_DUP instead of OP_HASH160
    s.extend_from_slice(&[0xab; 20]);
    s.push(0x87);
    assert!(!is_p2sh(&s));
}

#[test]
fn p2sh_template_wrong_last_byte() {
    let mut s = vec![0xa9, 0x14];
    s.extend_from_slice(&[0xab; 20]);
    s.push(0x88); // OP_EQUALVERIFY instead of OP_EQUAL
    assert!(!is_p2sh(&s));
}

#[test]
fn p2sh_template_wrong_push_length() {
    let mut s = vec![0xa9, 0x15]; // 21-byte push
    s.extend_from_slice(&[0xab; 20]);
    s.push(0x87);
    assert!(!is_p2sh(&s));
}

// ── Segwit v0 ──────────────────────────────────────────────────────────

fn segwit_flags() -> ScriptFlags {
    standard_flags() | ScriptFlags::WITNESS
}

/// Build a tx whose input has `witness` set.
fn make_witness_tx(witness_items: &[Vec<u8>]) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256([0u8; 32]),
                vout: 0,
            },
            script_sig: Vec::<u8>::new().into(),
            sequence: 0xffff_ffff,
            witness: Witness::from_slice(
                &witness_items.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            ),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(100_000),
            script_pubkey: vec![0x6a].into(),
        }],
        locktime: 0,
    }
}

/// P2WPKH scriptPubKey: `OP_0 <20-byte key hash>`.
fn p2wpkh_spk(pkh: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![0x00, 0x14];
    s.extend_from_slice(pkh);
    s
}

#[test]
fn p2wpkh_succeeds() {
    let sk = key([11u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let spk = p2wpkh_spk(&pkh);
    let amount = Satoshi::new(100_000);

    // The BIP143 sighash uses a synthetic P2PKH scriptCode.
    let mut script_code = vec![0x76u8, 0xa9, 0x14];
    script_code.extend_from_slice(&pkh);
    script_code.extend_from_slice(&[0x88, 0xac]);

    // Build a temporary tx with empty witness to compute the sighash.
    let tmp_tx = make_witness_tx(&[]);
    let cache = crate::script::sighash::Bip143Cache::new(&tmp_tx);
    let digest = tmp_tx.bip143_sighash(0, &script_code, amount, SigHashType::ALL, &cache);
    let sig = sign(&sk, &digest.0, SigHashType::ALL);

    let tx = make_witness_tx(&[sig, pk]);
    verify_script(&[], &spk, &tx, 0, amount, segwit_flags()).expect("p2wpkh should verify");
}

#[test]
fn p2wpkh_wrong_sig_fails() {
    let sk = key([11u8; 32]);
    let attacker = key([12u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let spk = p2wpkh_spk(&pkh);
    let amount = Satoshi::new(100_000);

    let mut script_code = vec![0x76u8, 0xa9, 0x14];
    script_code.extend_from_slice(&pkh);
    script_code.extend_from_slice(&[0x88, 0xac]);

    let tmp_tx = make_witness_tx(&[]);
    let cache = crate::script::sighash::Bip143Cache::new(&tmp_tx);
    let digest = tmp_tx.bip143_sighash(0, &script_code, amount, SigHashType::ALL, &cache);
    // Sign with the attacker key.
    let sig = sign(&attacker, &digest.0, SigHashType::ALL);

    let tx = make_witness_tx(&[sig, pk]);
    let r = verify_script(&[], &spk, &tx, 0, amount, segwit_flags());
    assert!(r.is_err(), "wrong sig must fail");
}

#[test]
fn p2wpkh_nonempty_scriptsig_is_malleable() {
    let sk = key([11u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let spk = p2wpkh_spk(&pkh);
    let amount = Satoshi::new(100_000);

    let tx = make_witness_tx(&[vec![0xab], pk.clone()]);
    // Supply a non-empty scriptSig — must be rejected.
    let r = verify_script(&[0x01, 0xff], &spk, &tx, 0, amount, segwit_flags());
    assert!(matches!(r, Err(ScriptError::WitnessMalleated)));
}

#[test]
fn p2wsh_succeeds() {
    // Witness script: OP_DUP OP_HASH160 <pkh> OP_EQUALVERIFY OP_CHECKSIG (P2PKH inside P2WSH).
    let sk = key([13u8; 32]);
    let pk = compressed_pubkey(&sk);
    let pkh = hash160(&pk);
    let witness_script = p2pkh_spk(&pkh);

    // P2WSH scriptPubKey: OP_0 <SHA256(witness_script)>
    let script_hash: [u8; 32] = Sha256::digest(&witness_script).into();
    let mut spk = vec![0x00, 0x20];
    spk.extend_from_slice(&script_hash);

    let amount = Satoshi::new(200_000);

    // BIP143 sighash: scriptCode = witness_script.
    let tmp_tx = make_witness_tx(&[]);
    let cache = crate::script::sighash::Bip143Cache::new(&tmp_tx);
    let digest = tmp_tx.bip143_sighash(0, &witness_script, amount, SigHashType::ALL, &cache);
    let sig = sign(&sk, &digest.0, SigHashType::ALL);

    // Witness stack: [sig, pubkey, witness_script]
    let tx = make_witness_tx(&[sig, pk, witness_script]);
    verify_script(&[], &spk, &tx, 0, amount, segwit_flags()).expect("p2wsh should verify");
}

#[test]
fn p2wsh_wrong_script_hash_fails() {
    let witness_script = vec![0x51]; // OP_1 — trivially true
    // Correct SHA256 of the witness script, but we use a *different* hash
    // in the scriptPubKey so the commitment check must fail.
    // The scriptPubKey program bytes must be non-zero so the initial
    // scriptPubKey eval leaves a truthy top-of-stack (all-zero = falsey).
    let correct_hash: [u8; 32] = Sha256::digest(&witness_script).into();
    let mut wrong_hash = correct_hash;
    wrong_hash[0] ^= 0xff; // flip one byte

    let mut spk = vec![0x00, 0x20];
    spk.extend_from_slice(&wrong_hash);

    let tx = make_witness_tx(&[witness_script]);
    let r = verify_script(&[], &spk, &tx, 0, Satoshi::new(0), segwit_flags());
    assert!(matches!(r, Err(ScriptError::WitnessProgramMismatch)));
}

#[test]
fn witness_unexpected_on_legacy_input() {
    // A P2WPKH input with only 1 witness item (should be 2).
    let pkh = [0xabu8; 20]; // non-zero so eval leaves truthy top
    let spk = p2wpkh_spk(&pkh);
    let tx = make_witness_tx(&[vec![0xab]]); // 1 item instead of 2
    let r = verify_script(&[], &spk, &tx, 0, Satoshi::new(0), segwit_flags());
    assert!(matches!(r, Err(ScriptError::WitnessProgramMismatch)));
}
