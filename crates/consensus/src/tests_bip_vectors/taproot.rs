//! BIP340/341/342 end-to-end test vectors.
//!
//! Every case encodes a complete input from first principles so that the
//! test acts as a specification: read this file and you can reconstruct any
//! vector by hand.  No external JSON fixtures; all values are computed inline
//! from the same primitives the implementation uses.
//!
//! Coverage:
//!  - BIP340 key-path: valid / corrupted Schnorr sig, explicit `SIGHASH_ALL`
//!  - BIP341 script-path: `OP_1` leaf, wrong output key, bad control block,
//!    wrong committed script
//!  - BIP342 tapscript CHECKSIG: valid sign + corrupted sig
//!  - Structural: empty witness, non-empty scriptSig, P2SH-wrapped taproot,
//!    TAPROOT flag disabled (unknown witness version tolerance)
//!  - Annex: key-path spend with a valid annex item

use mantle::primitives::{Hash256, OutPoint, Satoshi, Transaction, TxIn, TxOut, Witness};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

use crate::script::sighash::{Bip341Cache, TapHasher, TaprootSigHashType, TxSighash};
use crate::script::{ScriptFlags, verify_script};

// ── helpers ───────────────────────────────────────────────────────────────────

fn taproot_flags() -> ScriptFlags {
    ScriptFlags::P2SH | ScriptFlags::WITNESS | ScriptFlags::TAPROOT
}

fn make_taproot_tx(spk: Vec<u8>, _amount: Satoshi, witness_items: &[Vec<u8>]) -> Transaction {
    Transaction {
        version: 2,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256([0x11; 32]),
                vout: 0,
            },
            script_sig: Vec::<u8>::new().into(),
            sequence: 0xffff_fffd,
            witness: Witness::from_slice(
                &witness_items.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            ),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(900),
            script_pubkey: spk.into(),
        }],
        locktime: 0,
    }
}

/// Build a taproot key-path scriptPubKey: `OP_1 <32-byte x-only pubkey>`.
fn taproot_spk(output_key: &[u8; 32]) -> Vec<u8> {
    let mut s = vec![0x51, 0x20]; // OP_1, push 32
    s.extend_from_slice(output_key);
    s
}

fn key_path_output_key(internal_key: &[u8; 32]) -> [u8; 32] {
    TapHasher::tweak_output_key(internal_key, &[]).expect("valid internal key")
}

fn schnorr_sign(secret: &[u8; 32], msg: &[u8; 32]) -> [u8; 64] {
    use k256::schnorr::SigningKey;
    use k256::schnorr::signature::hazmat::PrehashSigner;
    let sk = SigningKey::from_bytes(secret.into()).expect("nonzero secret");
    let sig: k256::schnorr::Signature = sk.sign_prehash(msg).expect("sign cannot fail");
    sig.to_bytes()
}

fn schnorr_pubkey(secret: &[u8; 32]) -> [u8; 32] {
    use k256::schnorr::SigningKey;
    SigningKey::from_bytes(secret.into())
        .expect("nonzero secret")
        .verifying_key()
        .to_bytes()
        .into()
}

/// Compute the BIP341 tweaked signing key for a key-path spend.
///
/// BIP341 §"Spending using the key path":
///   d = `internal_secret` (negated if `internal_pubkey` has odd Y — k256 does this on construction)
///   t = int(TapTweak(`internal_pubkey` || `merkle_root`))  mod n
///   `tweaked_secret` = (d + t) mod n
fn tweaked_signing_key(internal_secret: &[u8; 32], merkle_root: &[u8]) -> [u8; 32] {
    use k256::elliptic_curve::bigint::Encoding;
    use k256::elliptic_curve::ops::Reduce;
    use k256::schnorr::SigningKey;
    use k256::{NonZeroScalar, Scalar, U256};

    // k256 normalises to even-Y on construction (negates d if pubkey.y is odd).
    let sk = SigningKey::from_bytes(internal_secret.into()).expect("nonzero secret");
    let internal_pubkey: [u8; 32] = sk.verifying_key().to_bytes().into();

    let mut tweak_input = Vec::with_capacity(32 + merkle_root.len());
    tweak_input.extend_from_slice(&internal_pubkey);
    tweak_input.extend_from_slice(merkle_root);
    let tweak_bytes =
        crate::script::consensus_crypto::Schnorr::tagged_hash(b"TapTweak", &tweak_input);
    let t = <Scalar as Reduce<U256>>::reduce(&U256::from_be_bytes(tweak_bytes.into()));

    let d: Scalar = *sk.as_nonzero_scalar().as_ref();
    let tweaked = d + t;
    let tweaked_nz = NonZeroScalar::new(tweaked).expect("tweaked scalar is non-zero");

    tweaked_nz.to_bytes().into()
}

// ── BIP340 key-path ───────────────────────────────────────────────────────────

#[test]
fn bip340_key_path_spend_succeeds() {
    let internal_secret = [0x42u8; 32];
    let internal_key = schnorr_pubkey(&internal_secret);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(100_000);
    let tweaked_secret = tweaked_signing_key(&internal_secret, &[]);

    // Compute sighash on a placeholder tx, sign, then rebuild with the real sig.
    // Two-pass: first pass uses a zeroed sig placeholder to get the tx shape
    // (sighash doesn't commit to witness, so this equals the final tx's sighash).
    let placeholder_tx = make_taproot_tx(spk.clone(), amount, &[[0u8; 64].to_vec()]);
    let cache = Bip341Cache::new(&placeholder_tx, &[amount], &[spk.as_slice()]);
    let msg = placeholder_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache,
        None,
        None,
    );
    let sig64 = schnorr_sign(&tweaked_secret, &msg);

    let tx = make_taproot_tx(spk.clone(), amount, &[sig64.to_vec()]);
    verify_script(&[], &spk, &tx, 0, amount, taproot_flags())
        .expect("BIP340 key-path spend must verify");
}

#[test]
fn bip340_key_path_wrong_sig_fails() {
    let internal_secret = [0x42u8; 32];
    let internal_key = schnorr_pubkey(&internal_secret);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(100_000);

    let tmp_tx = make_taproot_tx(spk.clone(), amount, &[]);
    let cache = Bip341Cache::new(&tmp_tx, &[amount], &[spk.as_slice()]);
    let msg = tmp_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache,
        None,
        None,
    );
    let tweaked_secret = tweaked_signing_key(&internal_secret, &[]);
    let mut sig64 = schnorr_sign(&tweaked_secret, &msg);
    sig64[0] ^= 0xff;

    let tx = make_taproot_tx(spk.clone(), amount, &[sig64.to_vec()]);
    assert!(
        verify_script(&[], &spk, &tx, 0, amount, taproot_flags()).is_err(),
        "corrupted sig must fail"
    );
}

#[test]
fn bip340_key_path_sighash_all_explicit_succeeds() {
    // 65-byte form: append 0x01 (explicit SIGHASH_ALL).
    let internal_secret = [0x43u8; 32];
    let internal_key = schnorr_pubkey(&internal_secret);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(50_000);

    let tmp_tx = make_taproot_tx(spk.clone(), amount, &[]);
    let cache = Bip341Cache::new(&tmp_tx, &[amount], &[spk.as_slice()]);
    let msg = tmp_tx.bip341_sighash(0, amount, &spk, TaprootSigHashType::ALL, &cache, None, None);
    let tweaked_secret = tweaked_signing_key(&internal_secret, &[]);
    let sig64 = schnorr_sign(&tweaked_secret, &msg);
    let mut sig65 = sig64.to_vec();
    sig65.push(0x01);

    let tx = make_taproot_tx(spk.clone(), amount, &[sig65]);
    verify_script(&[], &spk, &tx, 0, amount, taproot_flags())
        .expect("65-byte explicit SIGHASH_ALL must verify");
}

#[test]
fn bip340_key_path_empty_witness_fails() {
    let internal_secret = [0x42u8; 32];
    let internal_key = schnorr_pubkey(&internal_secret);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(100_000);

    let tx = make_taproot_tx(spk.clone(), amount, &[]);
    assert!(
        verify_script(&[], &spk, &tx, 0, amount, taproot_flags()).is_err(),
        "empty witness must fail for taproot"
    );
}

// ── BIP341 script-path ────────────────────────────────────────────────────────

/// Build a script-path spend for a single-leaf tree.
///
/// Returns `(spk, witness_items)` ready for `make_taproot_tx`.
fn script_path_single_leaf(internal_key: &[u8; 32], script: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let leaf_version: u8 = 0xc0;
    let leaf_hash = TapHasher::leaf(leaf_version, script);
    let output_key =
        TapHasher::tweak_output_key(internal_key, &leaf_hash).expect("valid internal key");
    let spk = taproot_spk(&output_key);

    // Control block: (leaf_version | parity=0) || internal_key  (no merkle path)
    let mut control_block = vec![leaf_version];
    control_block.extend_from_slice(internal_key);

    // Witness: [script, control_block]  — no stack inputs for OP_1
    let witness_items = vec![script.to_vec(), control_block];
    (spk, witness_items)
}

#[test]
fn bip341_script_path_op1_leaf_succeeds() {
    let internal_key = schnorr_pubkey(&[0x55u8; 32]);
    let script = vec![0x51]; // OP_1
    let (spk, witness_items) = script_path_single_leaf(&internal_key, &script);
    let amount = Satoshi::new(200_000);

    let tx = make_taproot_tx(spk.clone(), amount, &witness_items);
    verify_script(&[], &spk, &tx, 0, amount, taproot_flags())
        .expect("OP_1 tapscript leaf must verify");
}

#[test]
fn bip341_script_path_wrong_output_key_fails() {
    let internal_key = schnorr_pubkey(&[0x55u8; 32]);
    let script = vec![0x51]; // OP_1
    let (correct_spk, witness_items) = script_path_single_leaf(&internal_key, &script);

    let mut wrong_spk = correct_spk.clone();
    wrong_spk[2] ^= 0xff;

    let amount = Satoshi::new(200_000);
    let tx = make_taproot_tx(wrong_spk.clone(), amount, &witness_items);
    assert!(
        verify_script(&[], &wrong_spk, &tx, 0, amount, taproot_flags()).is_err(),
        "wrong output key must fail"
    );
}

#[test]
fn bip341_script_path_bad_control_block_fails() {
    let internal_key = schnorr_pubkey(&[0x55u8; 32]);
    let script = vec![0x51]; // OP_1
    let (spk, _) = script_path_single_leaf(&internal_key, &script);

    let bad_control = vec![0xc0]; // only leaf_version byte, no internal key
    let witness_items = vec![script.clone(), bad_control];
    let amount = Satoshi::new(200_000);
    let tx = make_taproot_tx(spk.clone(), amount, &witness_items);
    assert!(
        verify_script(&[], &spk, &tx, 0, amount, taproot_flags()).is_err(),
        "truncated control block must fail"
    );
}

#[test]
fn bip341_script_path_wrong_leaf_script_fails() {
    let internal_key = schnorr_pubkey(&[0x55u8; 32]);
    let committed_script = vec![0x51]; // OP_1
    let (spk, mut witness_items) = script_path_single_leaf(&internal_key, &committed_script);

    witness_items[0] = vec![0x00]; // swap to OP_0 — leaf hash won't match

    let amount = Satoshi::new(200_000);
    let tx = make_taproot_tx(spk.clone(), amount, &witness_items);
    assert!(
        verify_script(&[], &spk, &tx, 0, amount, taproot_flags()).is_err(),
        "mismatched committed script must fail"
    );
}

// ── BIP342 tapscript CHECKSIG ─────────────────────────────────────────────────

fn checksig_tapscript(pubkey32: &[u8; 32]) -> Vec<u8> {
    let mut s = vec![0x20]; // push 32 bytes
    s.extend_from_slice(pubkey32);
    s.push(0xac); // OP_CHECKSIG
    s
}

#[test]
fn bip342_tapscript_checksig_succeeds() {
    let secret = [0x77u8; 32];
    let pubkey32 = schnorr_pubkey(&secret);
    let internal_key = schnorr_pubkey(&[0x56u8; 32]);

    let script = checksig_tapscript(&pubkey32);
    let leaf_version: u8 = 0xc0;
    let leaf_hash = TapHasher::leaf(leaf_version, &script);
    let output_key =
        TapHasher::tweak_output_key(&internal_key, &leaf_hash).expect("valid internal key");
    let spk = taproot_spk(&output_key);

    let mut control_block = vec![leaf_version];
    control_block.extend_from_slice(&internal_key);

    let amount = Satoshi::new(300_000);
    let tmp_tx = make_taproot_tx(spk.clone(), amount, &[]);
    let cache = Bip341Cache::new(&tmp_tx, &[amount], &[spk.as_slice()]);
    let msg = tmp_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache,
        None,
        Some((leaf_hash, 0xffff_ffff)),
    );
    let sig64 = schnorr_sign(&secret, &msg);

    let witness_items = vec![sig64.to_vec(), script.clone(), control_block];
    let tx = make_taproot_tx(spk.clone(), amount, &witness_items);
    verify_script(&[], &spk, &tx, 0, amount, taproot_flags())
        .expect("BIP342 CHECKSIG tapscript must verify");
}

#[test]
fn bip342_tapscript_checksig_wrong_sig_fails() {
    let secret = [0x77u8; 32];
    let pubkey32 = schnorr_pubkey(&secret);
    let internal_key = schnorr_pubkey(&[0x56u8; 32]);

    let script = checksig_tapscript(&pubkey32);
    let leaf_version: u8 = 0xc0;
    let leaf_hash = TapHasher::leaf(leaf_version, &script);
    let output_key =
        TapHasher::tweak_output_key(&internal_key, &leaf_hash).expect("valid internal key");
    let spk = taproot_spk(&output_key);

    let mut control_block = vec![leaf_version];
    control_block.extend_from_slice(&internal_key);

    let amount = Satoshi::new(300_000);
    let tmp_tx = make_taproot_tx(spk.clone(), amount, &[]);
    let cache = Bip341Cache::new(&tmp_tx, &[amount], &[spk.as_slice()]);
    let msg = tmp_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache,
        None,
        Some((leaf_hash, 0xffff_ffff)),
    );
    let mut sig64 = schnorr_sign(&secret, &msg);
    sig64[0] ^= 0xff;

    let witness_items = vec![sig64.to_vec(), script.clone(), control_block];
    let tx = make_taproot_tx(spk.clone(), amount, &witness_items);
    assert!(
        verify_script(&[], &spk, &tx, 0, amount, taproot_flags()).is_err(),
        "wrong sig in tapscript CHECKSIG must fail"
    );
}

// ── Structural rejections ─────────────────────────────────────────────────────

#[test]
fn taproot_nonempty_scriptsig_is_malleated() {
    let secret = [0x42u8; 32];
    let internal_key = schnorr_pubkey(&secret);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(100_000);

    let tmp_tx = make_taproot_tx(spk.clone(), amount, &[]);
    let cache = Bip341Cache::new(&tmp_tx, &[amount], &[spk.as_slice()]);
    let msg = tmp_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache,
        None,
        None,
    );
    let sig64 = schnorr_sign(&secret, &msg);

    let mut tx = make_taproot_tx(spk.clone(), amount, &[sig64.to_vec()]);
    tx.inputs[0].script_sig = vec![0x51].into();

    // Non-empty scriptSig on a native segwit input → WitnessMalleated.
    assert!(
        verify_script(&[0x51], &spk, &tx, 0, amount, taproot_flags()).is_err(),
        "non-empty scriptSig on taproot must be rejected"
    );
}

#[test]
fn p2sh_wrapped_taproot_is_rejected() {
    // BIP341: taproot MAY NOT be wrapped in P2SH.
    let internal_key = schnorr_pubkey(&[0x42u8; 32]);
    let output_key = key_path_output_key(&internal_key);
    let taproot_spk_bytes = taproot_spk(&output_key);

    let h160: [u8; 20] = Ripemd160::digest(Sha256::digest(&taproot_spk_bytes)).into();
    let mut p2sh_spk = vec![0xa9, 0x14];
    p2sh_spk.extend_from_slice(&h160);
    p2sh_spk.push(0x87);

    let mut script_sig = vec![u8::try_from(taproot_spk_bytes.len()).expect("≤ 75")];
    script_sig.extend_from_slice(&taproot_spk_bytes);

    let tx = Transaction {
        version: 2,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256([0x11; 32]),
                vout: 0,
            },
            script_sig: script_sig.clone().into(),
            sequence: 0xffff_ffff,
            witness: Witness::from_slice(&[&[][..]]),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(900),
            script_pubkey: vec![0x6a].into(),
        }],
        locktime: 0,
    };

    assert!(
        verify_script(
            &script_sig,
            &p2sh_spk,
            &tx,
            0,
            Satoshi::new(100_000),
            taproot_flags()
        )
        .is_err(),
        "P2SH-wrapped taproot must be rejected"
    );
}

#[test]
fn taproot_disabled_treats_as_unknown_witness_version() {
    // Without TAPROOT flag a v1 witness program is an unknown version.
    // Unknown versions succeed unless DISCOURAGE_UPGRADABLE_WITNESS_PROGRAM is set.
    let internal_key = schnorr_pubkey(&[0x42u8; 32]);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(100_000);

    let tx = make_taproot_tx(spk.clone(), amount, &[vec![0xde, 0xad]]);

    let flags_no_taproot = ScriptFlags::P2SH | ScriptFlags::WITNESS;
    verify_script(&[], &spk, &tx, 0, amount, flags_no_taproot)
        .expect("unknown witness version without DISCOURAGE flag must succeed");
}

// ── Annex handling ────────────────────────────────────────────────────────────

#[test]
fn bip341_key_path_with_annex_succeeds() {
    // The annex (last witness item starting with 0x50) is committed to in the
    // sighash but otherwise ignored during key-path execution.
    let internal_secret = [0x42u8; 32];
    let internal_key = schnorr_pubkey(&internal_secret);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(100_000);

    let annex = vec![0x50, 0xde, 0xad];

    // Use the final tx layout (sig placeholder + annex) so the sighash sees
    // the right annex commitment.
    let tmp_tx = make_taproot_tx(spk.clone(), amount, &[vec![0u8; 64], annex.clone()]);
    let cache = Bip341Cache::new(&tmp_tx, &[amount], &[spk.as_slice()]);
    let annex_body = &annex[1..]; // strip 0x50 prefix byte
    let msg = tmp_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache,
        Some(annex_body),
        None,
    );
    // Key-path sigs are produced with the *tweaked* secret so the resulting
    // signature verifies against the output (tweaked) key in the scriptPubKey.
    let tweaked_secret = tweaked_signing_key(&internal_secret, &[]);
    let sig64 = schnorr_sign(&tweaked_secret, &msg);

    let tx = make_taproot_tx(spk.clone(), amount, &[sig64.to_vec(), annex]);
    verify_script(&[], &spk, &tx, 0, amount, taproot_flags())
        .expect("key-path spend with annex must verify");
}

#[test]
fn diag_schnorr_sign_verify_roundtrip() {
    use k256::schnorr::SigningKey;
    use k256::schnorr::signature::hazmat::{PrehashSigner, PrehashVerifier};

    let secret = [0x42u8; 32];
    let sk = SigningKey::from_bytes((&secret).into()).expect("valid");
    let msg = [0xabu8; 32];
    let sig: k256::schnorr::Signature = sk.sign_prehash(&msg).expect("sign");
    let vk = sk.verifying_key();
    // PrehashVerifier uses the raw bytes (no re-hash) — must match sign_prehash.
    vk.verify_prehash(&msg, &sig)
        .expect("k256 prehash verify must pass");

    let sig_bytes: [u8; 64] = sig.to_bytes();
    let pk_bytes: [u8; 32] = vk.to_bytes().into();
    let ok = crate::script::consensus_crypto::Schnorr::verify(&sig_bytes, &pk_bytes, &msg);
    assert!(ok, "Schnorr::verify must pass for k256-signed msg");
}

#[test]
fn diag_sighash_determinism() {
    let secret = [0x42u8; 32];
    let internal_key = schnorr_pubkey(&secret);
    let output_key = key_path_output_key(&internal_key);
    let spk = taproot_spk(&output_key);
    let amount = Satoshi::new(100_000);

    let tmp_tx = make_taproot_tx(spk.clone(), amount, &[]);
    let cache1 = Bip341Cache::new(&tmp_tx, &[amount], &[spk.as_slice()]);
    let msg1 = tmp_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache1,
        None,
        None,
    );

    // The real tx has a witness — sighash must be the same.
    let sig64 = [0u8; 64];
    let real_tx = make_taproot_tx(spk.clone(), amount, &[sig64.to_vec()]);
    let cache2 = Bip341Cache::new(&real_tx, &[amount], &[spk.as_slice()]);
    let msg2 = real_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &cache2,
        None,
        None,
    );

    // Also compute what run_taproot computes: it builds cache from the REAL tx.
    assert_eq!(msg1, msg2, "sighash must not depend on witness content");

    let tweaked_secret = tweaked_signing_key(&secret, &[]);
    let sig = schnorr_sign(&tweaked_secret, &msg1);

    // Mimic exactly what run_taproot does: build cache from the REAL tx (with sig in witness).
    let real_tx = make_taproot_tx(spk.clone(), amount, &[sig.to_vec()]);
    let rt_cache = Bip341Cache::new(&real_tx, &[amount], &[spk.as_slice()]);
    let rt_msg = real_tx.bip341_sighash(
        0,
        amount,
        &spk,
        TaprootSigHashType::DEFAULT,
        &rt_cache,
        None,
        None,
    );

    eprintln!("msg1 (from placeholder_tx): {msg1:02x?}");
    eprintln!("rt_msg (from real_tx):      {rt_msg:02x?}");
    eprintln!("msgs_match: {}", msg1 == rt_msg);

    // run_taproot verifies: Schnorr::verify(sig64, program32=output_key, &rt_msg)
    let ok = crate::script::consensus_crypto::Schnorr::verify(&sig, &output_key, &rt_msg);
    eprintln!("verify_schnorr(tweaked_sig, output_key, rt_msg): {ok}");
}
