//! `verify_script` — the public entry point that drives a script verification.
//!
//! This is the single function the rest of `mantle-consensus` calls to
//! verify a transaction input. Internally it dispatches on scriptPubKey
//! shape (legacy vs. BIP16 P2SH) and runs the right sequence of evals on a
//! shared interpreter:
//!
//! 1. Optional `SIGPUSHONLY` check on `script_sig`.
//! 2. Eval `script_sig` — its only job is to leave its pushes on the stack.
//! 3. Snapshot the stack (in case we need it for the P2SH redeem dance).
//! 4. Eval `script_pubkey` — must end with a non-empty stack and a truthy top.
//! 5. If `VERIFY_P2SH` is on and `script_pubkey` matches the BIP16 pattern,
//!    pop the last `script_sig` push as the redeem script, restore the
//!    snapshot minus the redeem, and eval the redeem script.
//! 6. Optional `CLEANSTACK` check — exactly one item must remain.
//!
//! Witness handling, taproot, and BIP143 sighash are M2/M3.

use mantle::primitives::{Satoshi, Transaction};

use super::checker::TxSigChecker;
use super::error::ScriptError;
use super::flags::ScriptFlags;
use super::interpreter::Interpreter;
use super::sig_version::SigVersion;

/// Verify a single transaction input's script pair.
///
/// `script_sig` is the input's scriptSig; `script_pubkey` is the spent
/// output's scriptPubKey. `tx` is the transaction being validated and `n_in`
/// is the index of the input within it. `amount` is the value of the spent
/// output (unused in legacy regime; threaded through for the BIP143 path
/// that lands in M2). `flags` selects which soft-fork rules apply.
///
/// Returns `Ok(())` iff the input authorises spending under those rules.
///
/// # Errors
/// Any of the structured `ScriptError` variants — see that enum's docs.
pub fn verify_script(
    script_sig: &[u8],
    script_pubkey: &[u8],
    tx: &Transaction,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
) -> Result<(), ScriptError> {
    // ── SIGPUSHONLY ─────────────────────────────────────────────────────────
    // BIP62 rule 2. Only push opcodes may appear in scriptSig. Always required
    // for P2SH (Core enforces it implicitly via the redeem-script extraction)
    // and explicitly under SIGPUSHONLY.
    if flags.contains(ScriptFlags::SIGPUSHONLY) && !is_push_only(script_sig) {
        return Err(ScriptError::SigPushOnly);
    }

    let checker = TxSigChecker::new(tx, n_in, amount, flags);
    let mut interp = Interpreter::new(&checker, flags, SigVersion::Base);

    // ── scriptSig ───────────────────────────────────────────────────────────
    interp.eval(script_sig)?;

    // Snapshot the post-scriptSig stack so we can roll back to it if the
    // P2SH redeem path activates. Cheap: `Stack` clone is a Vec clone of
    // small inline-or-heap elements. Skipped when P2SH is off.
    let stack_after_sig =
        if flags.contains(ScriptFlags::P2SH) { Some(interp.stack().clone()) } else { None };

    // ── scriptPubKey ────────────────────────────────────────────────────────
    interp.eval(script_pubkey)?;

    // The scriptPubKey eval must leave at least one truthy item on top.
    if interp.stack().is_empty() {
        return Err(ScriptError::EvalFalse);
    }
    if !interp.stack().peek(0)?.is_truthy() {
        return Err(ScriptError::EvalFalse);
    }

    // ── BIP16 P2SH ──────────────────────────────────────────────────────────
    if let Some(saved_stack) = stack_after_sig
        && is_p2sh(script_pubkey)
    {
        // P2SH requires SIGPUSHONLY-style purity on the unlock — even
        // without the standalone flag — because we extract the redeem
        // script from a single push at the top of `script_sig`.
        if !is_push_only(script_sig) {
            return Err(ScriptError::SigPushOnly);
        }

        // Restore the post-scriptSig stack and pop the last push: that
        // push *is* the redeem script.
        *interp.stack_mut() = saved_stack;
        let redeem = interp.stack_mut().pop()?;

        interp.eval(redeem.as_slice())?;

        if interp.stack().is_empty() {
            return Err(ScriptError::EvalFalse);
        }
        if !interp.stack().peek(0)?.is_truthy() {
            return Err(ScriptError::EvalFalse);
        }
    }

    // ── CLEANSTACK ──────────────────────────────────────────────────────────
    // BIP62 rule 6. Final stack must contain exactly one element. Only meaningful
    // under P2SH (Core gates it accordingly); we follow the same gate.
    if flags.contains(ScriptFlags::CLEANSTACK)
        && flags.contains(ScriptFlags::P2SH)
        && interp.stack().len() != 1
    {
        return Err(ScriptError::CleanStack);
    }

    Ok(())
}

/// `true` if every opcode in `script` is a push (BIP62 rule 2 / SIGPUSHONLY).
///
/// Truncated pushes — a push opcode declaring more bytes than remain — fail
/// the check: the script is malformed regardless of intent. The caller
/// returns `SigPushOnly` for that case (matching Core, where any parse
/// failure during the push-only walk is treated as a non-push opcode).
#[must_use]
fn is_push_only(script: &[u8]) -> bool {
    use super::interpreter::reader::ScriptReader;

    let mut reader = ScriptReader::new(script);
    loop {
        match reader.next_step() {
            Ok(None) => return true,
            Ok(Some(step)) if step.opcode.is_push() => {}
            _ => return false,
        }
    }
}

/// `true` if `script` matches the BIP16 P2SH template:
/// `OP_HASH160 <0x14> <20-byte hash> OP_EQUAL`. Exact byte-pattern match —
/// not a structural check, per BIP16's wording.
#[must_use]
fn is_p2sh(script: &[u8]) -> bool {
    script.len() == 23
        && script[0] == 0xa9      // OP_HASH160
        && script[1] == 0x14      // push 20 bytes
        && script[22] == 0x87 // OP_EQUAL
}

#[cfg(test)]
mod tests {
    use super::*;

    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::{Signature as EcdsaSig, SigningKey};
    use mantle::primitives::{Hash256, OutPoint, TxIn, TxOut};
    use ripemd::Ripemd160;
    use sha2::{Digest, Sha256};

    use crate::script::sighash::{SigHashType, signature_hash};

    // ── End-to-end signing helpers ─────────────────────────────────────────

    fn key(secret: [u8; 32]) -> SigningKey {
        SigningKey::from_bytes(&secret.into()).expect("nonzero secret")
    }

    fn compressed_pubkey(sk: &SigningKey) -> Vec<u8> {
        sk.verifying_key().to_encoded_point(true).as_bytes().to_vec()
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
                previous_output: OutPoint { txid: Hash256([0u8; 32]), vout: 0 },
                script_sig: Vec::<u8>::new().into(),
                sequence: 0xffff_ffff,
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
        let digest = signature_hash(&tx, 0, &spk, SigHashType::ALL);
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
        let digest = signature_hash(&tx, 0, &spk, SigHashType::ALL);
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
        let digest = signature_hash(&tx, 0, &redeem, SigHashType::ALL);
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
        let digest = signature_hash(&tx, 0, &redeem, SigHashType::ALL);
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
        let digest = signature_hash(&tx, 0, &redeem, SigHashType::ALL);
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
        let digest = signature_hash(&tx, 0, &redeem, SigHashType::ALL);
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
}
