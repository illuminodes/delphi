//! `verify_script` — the public entry point that drives a script verification.
//!
//! Dispatches on scriptPubKey shape and runs the right sequence of evals:
//!
//! 1. Optional `SIGPUSHONLY` check on `script_sig`.
//! 2. Eval `script_sig` — its only job is to leave its pushes on the stack.
//! 3. Snapshot the stack (in case we need it for the P2SH redeem dance).
//! 4. Eval `script_pubkey` — must end with a non-empty stack and a truthy top.
//! 5. If `VERIFY_P2SH` is on and `script_pubkey` matches the BIP16 pattern,
//!    pop the last `script_sig` push as the redeem script, restore the
//!    snapshot minus the redeem, and eval the redeem script.
//! 6. If `WITNESS` is on and scriptPubKey (or P2SH redeem) is a witness program,
//!    run the segwit v0 execution path (BIP141/143):
//!    - P2WPKH (`OP_0 <20 bytes>`): synthetic P2PKH over the two-item witness.
//!    - P2WSH (`OP_0 <32 bytes>`): hash-check the witness script, then eval it.
//! 7. Optional `CLEANSTACK` check — exactly one item must remain.

use mantle::primitives::Satoshi;

use super::checker::{SignatureChecker, TxSigChecker};
use super::consensus_crypto::{Ecdsa, Schnorr};
use super::error::ScriptError;
use super::flags::ScriptFlags;
use super::interpreter::{InterpStackElement, Interpreter, TapscriptCtx};
use super::sig_version::SigVersion;
use super::sighash::{Bip143Cache, Bip341Cache, TapHasher, TxSighash};
use crate::tx_view::{InputView, TxView, WitnessView};

/// Verifier for a single transaction input's script pair.
///
/// Build once per input (or share the `Bip143Cache` across inputs of the same
/// transaction with [`ScriptVerifier::with_cache`]), then call [`verify`] or
/// [`verify_with_bufs`].
///
/// [`verify`]: ScriptVerifier::verify
/// [`verify_with_bufs`]: ScriptVerifier::verify_with_bufs
pub struct ScriptVerifier<'a, T: TxView> {
    tx: &'a T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
    cache: Bip143Cache,
}

impl<'a, T: TxView> ScriptVerifier<'a, T> {
    /// Build a verifier for input `n_in` of `tx`. Computes the `Bip143Cache`
    /// immediately; use [`with_cache`] when verifying multiple inputs of the
    /// same transaction.
    ///
    /// [`with_cache`]: ScriptVerifier::with_cache
    #[must_use]
    pub fn new(tx: &'a T, n_in: usize, amount: Satoshi, flags: ScriptFlags) -> Self {
        Self {
            tx,
            n_in,
            amount,
            flags,
            cache: Bip143Cache::new(tx),
        }
    }

    /// Build a verifier reusing a pre-built [`Bip143Cache`].
    ///
    /// Build the cache once with [`Bip143Cache::new`] before verifying any
    /// input, then pass it to this constructor for each input. This avoids
    /// recomputing `hashPrevouts`, `hashSequence`, and `hashOutputs` for
    /// every input of the same transaction.
    #[must_use]
    pub fn with_cache(
        tx: &'a T,
        n_in: usize,
        amount: Satoshi,
        flags: ScriptFlags,
        cache: Bip143Cache,
    ) -> Self {
        Self {
            tx,
            n_in,
            amount,
            flags,
            cache,
        }
    }

    /// Verify the script pair. Returns `Ok(())` iff the input authorises
    /// spending under the configured rules.
    ///
    /// # Errors
    /// Any of the structured `ScriptError` variants — see that enum's docs.
    pub fn verify(&self, script_sig: &[u8], script_pubkey: &[u8]) -> Result<(), ScriptError> {
        let bufs = (Vec::new(), Vec::new());
        let (result, _bufs) = verify_script_impl(
            script_sig,
            script_pubkey,
            self.tx,
            self.n_in,
            self.amount,
            self.flags,
            &self.cache,
            bufs,
        );
        result
    }

    /// Zero-allocation variant of [`verify`].
    ///
    /// The caller passes two pre-allocated, empty `Vec` buffers (for the main
    /// and alt stacks). On return — success *or* error — the same buffers are
    /// returned cleared but with their capacity intact, ready to be reused for
    /// the next input.
    ///
    /// # Errors
    /// Same as [`verify`].
    ///
    /// [`verify`]: ScriptVerifier::verify
    pub fn verify_with_bufs(
        &self,
        script_sig: &[u8],
        script_pubkey: &[u8],
        bufs: (Vec<InterpStackElement>, Vec<InterpStackElement>),
    ) -> (
        Result<(), ScriptError>,
        (Vec<InterpStackElement>, Vec<InterpStackElement>),
    ) {
        verify_script_impl(
            script_sig,
            script_pubkey,
            self.tx,
            self.n_in,
            self.amount,
            self.flags,
            &self.cache,
            bufs,
        )
    }
}

/// Verify a single transaction input's script pair.
///
/// `script_sig` is the input's scriptSig; `script_pubkey` is the spent
/// output's scriptPubKey. `tx` is the transaction being validated and `n_in`
/// is the index of the input within it. `amount` is the value of the spent
/// output (unused in legacy regime; threaded through for the BIP143 path).
/// `flags` selects which soft-fork rules apply.
///
/// Returns `Ok(())` iff the input authorises spending under those rules.
///
/// Mirrors the `bitcoinconsensus` C API for fuzzer differential parity — prefer
/// [`ScriptVerifier`] for new callers.
///
/// # Errors
/// Any of the structured `ScriptError` variants — see that enum's docs.
pub fn verify_script<T: TxView>(
    script_sig: &[u8],
    script_pubkey: &[u8],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
) -> Result<(), ScriptError> {
    ScriptVerifier::new(tx, n_in, amount, flags).verify(script_sig, script_pubkey)
}

/// Cached variant of [`verify_script`].
///
/// Probes `cache` with a key derived from `(script_sig, script_pubkey,
/// witness, amount, flags)` before calling the interpreter. On a hit the
/// stored verdict is returned immediately; on a miss the cold path runs and
/// the verdict is stored for future calls.
///
/// Accepts/rejects map to `Ok(())`/`Err(ScriptError::EvalFalse)` for cached
/// rejects (the exact error variant from a cold path is not stored — callers
/// that need the specific error on rejection should use [`verify_script`]).
///
/// # Errors
/// Same variants as [`verify_script`] on a cold miss. Cached rejects return
/// [`ScriptError::EvalFalse`].
pub fn verify_script_cached<T: TxView>(
    script_sig: &[u8],
    script_pubkey: &[u8],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
    cache: &mut super::cache::ScriptExecutionCache,
) -> Result<(), ScriptError> {
    let witness_items = tx.input(n_in).witness().to_vec();
    let key = super::cache::ScriptExecutionCache::key(
        script_sig,
        script_pubkey,
        &witness_items,
        amount.to_u64(),
        flags.bits(),
    );
    if let Some(verdict) = cache.get(&key) {
        return if verdict {
            Ok(())
        } else {
            Err(ScriptError::EvalFalse)
        };
    }
    let result = verify_script(script_sig, script_pubkey, tx, n_in, amount, flags);
    cache.insert(key, result.is_ok());
    result
}

#[allow(clippy::too_many_arguments)]
fn verify_script_impl<T: TxView>(
    script_sig: &[u8],
    script_pubkey: &[u8],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
    cache: &Bip143Cache,
    mut bufs: (Vec<InterpStackElement>, Vec<InterpStackElement>),
) -> (
    Result<(), ScriptError>,
    (Vec<InterpStackElement>, Vec<InterpStackElement>),
) {
    let result = verify_script_inner(
        script_sig,
        script_pubkey,
        tx,
        n_in,
        amount,
        flags,
        cache,
        &mut bufs,
    );
    (result, bufs)
}

/// Fast path for the P2PKH pattern (`OP_DUP OP_HASH160 <20> <pkh> OP_EQUALVERIFY OP_CHECKSIG`).
///
/// Returns `Some(result)` when the scriptPubKey matches and scriptSig is two
/// canonical pushes (`<sig> <pubkey>`). Returns `None` to fall through to the
/// full interpreter for any other shape.
///
/// Correctness invariants vs. the full interpreter path:
/// - `SIGPUSHONLY`: P2PKH scriptSig is always two push opcodes — the check
///   passes trivially for any well-formed two-push scriptSig and we never admit
///   non-push bytes, so we skip the explicit scan.
/// - `CLEANSTACK`: final stack always has exactly one item → always satisfied.
/// - `WITNESS`: if the flag is set and the input carries a witness, that is
///   `WitnessUnexpected` — checked here.
/// - `NULLFAIL`: if sig is non-empty and CHECKSIG fails, reject.
/// - `DERSIG`/`STRICTENC`/`LOW_S`: delegated to `check_signature_encoding` and
///   `check_pubkey_encoding` (same code the opcode handler calls).
fn try_verify_p2pkh<T: TxView>(
    script_sig: &[u8],
    script_pubkey: &[u8],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
    cache: &Bip143Cache,
) -> Option<Result<(), ScriptError>> {
    use super::interpreter::reader::ScriptReader;
    use ripemd::Ripemd160;
    use sha2::{Digest, Sha256};

    // ── Pattern match on scriptPubKey ────────────────────────────────────────
    // 25 bytes: OP_DUP(76) OP_HASH160(a9) OP_PUSHBYTES_20(14) <20b> OP_EQUALVERIFY(88) OP_CHECKSIG(ac)
    if script_pubkey.len() != 25
        || script_pubkey[0] != 0x76
        || script_pubkey[1] != 0xa9
        || script_pubkey[2] != 0x14
        || script_pubkey[23] != 0x88
        || script_pubkey[24] != 0xac
    {
        return None;
    }
    let pkh: &[u8; 20] = script_pubkey[3..23].try_into().ok()?;

    // ── Parse scriptSig as <push sig> <push pubkey> ─────────────────────────
    // Decode with the same ScriptReader the interpreter uses; accept exactly
    // two push steps and nothing else.
    let mut reader = ScriptReader::new(script_sig);
    let sig_step = reader.next_step().ok()??;
    let pk_step = reader.next_step().ok()??;
    if reader.next_step().ok()?.is_some() {
        return None; // more than two items — fall through
    }
    if !sig_step.opcode.is_push() || !pk_step.opcode.is_push() {
        return None;
    }
    let sig = sig_step.data;
    let pubkey = pk_step.data;

    // ── WITNESS: no witness allowed on legacy P2PKH ──────────────────────────
    if flags.contains(ScriptFlags::WITNESS) && !tx.input(n_in).witness().is_empty() {
        return Some(Err(ScriptError::WitnessUnexpected));
    }

    // ── Encoding checks (DERSIG / STRICTENC / LOW_S) ─────────────────────────
    if let Err(e) = Ecdsa::check_sig_encoding(sig, flags) {
        return Some(Err(e));
    }
    if let Err(e) = Ecdsa::check_pubkey_encoding(pubkey, flags) {
        return Some(Err(e));
    }

    // ── OP_HASH160: RIPEMD160(SHA256(pubkey)) must match pkh ─────────────────
    let computed_pkh: [u8; 20] = Ripemd160::digest(Sha256::digest(pubkey)).into();
    if &computed_pkh != pkh {
        return Some(Err(ScriptError::EvalFalse));
    }

    // ── OP_CHECKSIG ─────────────────────────────────────────────────────────
    // Empty sig → CHECKSIG pushes false. With NULLFAIL, a *non-empty* sig that
    // fails verification is also an error.
    if sig.is_empty() {
        return Some(Err(ScriptError::EvalFalse));
    }

    // scriptCode = the full scriptPubKey (no OP_CODESEPARATOR in P2PKH).
    let checker = TxSigChecker::with_cache(tx, n_in, amount, flags, cache.clone());
    let ok = checker.check_ecdsa_sig(sig, pubkey, script_pubkey, SigVersion::Base);
    if !ok {
        if flags.contains(ScriptFlags::NULLFAIL) {
            return Some(Err(ScriptError::SigNullFail));
        }
        return Some(Err(ScriptError::EvalFalse));
    }

    Some(Ok(()))
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn verify_script_inner<T: TxView>(
    script_sig: &[u8],
    script_pubkey: &[u8],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
    cache: &Bip143Cache,
    bufs: &mut (Vec<InterpStackElement>, Vec<InterpStackElement>),
) -> Result<(), ScriptError> {
    // ── P2PKH fast path ─────────────────────────────────────────────────────
    // Handles ~60–70 % of mainnet inputs without touching the interpreter,
    // ConditionStack, or any other interpreter state.
    if let Some(result) =
        try_verify_p2pkh(script_sig, script_pubkey, tx, n_in, amount, flags, cache)
    {
        return result;
    }

    // ── SIGPUSHONLY ─────────────────────────────────────────────────────────
    // BIP62 rule 2. Only push opcodes may appear in scriptSig. Always required
    // for P2SH (Core enforces it implicitly via the redeem-script extraction)
    // and explicitly under SIGPUSHONLY.
    if flags.contains(ScriptFlags::SIGPUSHONLY) && !is_push_only(script_sig) {
        return Err(ScriptError::SigPushOnly);
    }

    let checker = TxSigChecker::with_cache(tx, n_in, amount, flags, cache.clone());
    let stack_buf = std::mem::take(&mut bufs.0);
    let altstack_buf = std::mem::take(&mut bufs.1);
    let mut interp =
        Interpreter::new_with_bufs(&checker, flags, SigVersion::Base, stack_buf, altstack_buf);

    // ── scriptSig ───────────────────────────────────────────────────────────
    let r = interp.eval(script_sig);
    if let Err(e) = r {
        let (s, a) = interp.into_bufs();
        bufs.0 = s;
        bufs.1 = a;
        return Err(e);
    }

    // Snapshot the post-scriptSig stack so we can roll back to it if the
    // P2SH redeem path activates. Cheap: `Stack` clone is a Vec clone of
    // small inline-or-heap elements. Skipped when P2SH is off.
    let stack_after_sig = if flags.contains(ScriptFlags::P2SH) {
        Some(interp.stack().clone())
    } else {
        None
    };

    // ── scriptPubKey ────────────────────────────────────────────────────────
    macro_rules! try_interp {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(e) => {
                    let (s, a) = interp.into_bufs();
                    bufs.0 = s;
                    bufs.1 = a;
                    return Err(e);
                }
            }
        };
    }

    try_interp!(interp.eval(script_pubkey));

    // The scriptPubKey eval must leave at least one truthy item on top.
    if interp.stack().is_empty() {
        let (s, a) = interp.into_bufs();
        bufs.0 = s;
        bufs.1 = a;
        return Err(ScriptError::EvalFalse);
    }
    if !try_interp!(interp.stack().peek(0)).is_truthy() {
        let (s, a) = interp.into_bufs();
        bufs.0 = s;
        bufs.1 = a;
        return Err(ScriptError::EvalFalse);
    }

    // ── BIP16 P2SH ──────────────────────────────────────────────────────────
    let mut witness_program: Option<(u8, Vec<u8>)> = None;

    if let Some(saved_stack) = stack_after_sig
        && is_p2sh(script_pubkey)
    {
        // P2SH requires SIGPUSHONLY-style purity on the unlock — even
        // without the standalone flag — because we extract the redeem
        // script from a single push at the top of `script_sig`.
        if !is_push_only(script_sig) {
            let (s, a) = interp.into_bufs();
            bufs.0 = s;
            bufs.1 = a;
            return Err(ScriptError::SigPushOnly);
        }

        // Restore the post-scriptSig stack and pop the last push: that
        // push *is* the redeem script.
        *interp.stack_mut() = saved_stack;
        let redeem = try_interp!(interp.stack_mut().pop());

        // If the redeem script is itself a witness program, record it for
        // the segwit path below. The P2SH eval is still required to pass
        // first — that verifies the hash commitment.
        if flags.contains(ScriptFlags::WITNESS) {
            witness_program = witness_program_of(redeem.as_slice());
        }

        try_interp!(interp.eval(redeem.as_slice()));

        if interp.stack().is_empty() {
            let (s, a) = interp.into_bufs();
            bufs.0 = s;
            bufs.1 = a;
            return Err(ScriptError::EvalFalse);
        }
        if !try_interp!(interp.stack().peek(0)).is_truthy() {
            let (s, a) = interp.into_bufs();
            bufs.0 = s;
            bufs.1 = a;
            return Err(ScriptError::EvalFalse);
        }
    } else if flags.contains(ScriptFlags::WITNESS) {
        witness_program = witness_program_of(script_pubkey);
    }

    // ── CLEANSTACK (legacy path) ─────────────────────────────────────────────
    // BIP62 rule 6. Final stack must contain exactly one element. Only meaningful
    // under P2SH (Core gates it accordingly); we follow the same gate.
    let cleanstack_len = interp.stack().len();

    // ── BIP141 segwit v0 ────────────────────────────────────────────────────
    // Recover bufs before potentially branching into witness execution, which
    // creates its own interpreter.
    let (s, a) = interp.into_bufs();
    bufs.0 = s;
    bufs.1 = a;

    if flags.contains(ScriptFlags::WITNESS) {
        if let Some((version, program)) = witness_program {
            // Native segwit: scriptSig must be empty (otherwise it's malleable).
            // For P2SH-wrapped witness the outer scriptSig is allowed, but must
            // be exactly `<push of redeem>` — already checked above by is_push_only.
            let is_p2sh_wrapped = is_p2sh(script_pubkey);
            if !is_p2sh_wrapped && !script_sig.is_empty() {
                return Err(ScriptError::WitnessMalleated);
            }
            if is_p2sh_wrapped && !is_push_only(script_sig) {
                return Err(ScriptError::WitnessMalleatedP2sh);
            }

            let witness_items = tx.input(n_in).witness().to_vec();

            if version == 0 {
                run_witness_v0(&program, &witness_items, tx, n_in, amount, flags, bufs)?;
            } else if version == 1 && program.len() == 32 && flags.contains(ScriptFlags::TAPROOT) {
                // BIP341 taproot (witness v1, 32-byte program).
                // P2SH-wrapped taproot is not valid — BIP341 §"Script validation rules".
                if is_p2sh(script_pubkey) {
                    return Err(ScriptError::WitnessMalleatedP2sh);
                }
                run_taproot(
                    program.as_slice().try_into().expect("len==32"),
                    &witness_items,
                    tx,
                    n_in,
                    amount,
                    script_pubkey,
                    flags,
                    bufs,
                )?;
            } else {
                // Unknown witness version.
                if flags.contains(ScriptFlags::DISCOURAGE_UPGRADABLE_WITNESS_PROGRAM) {
                    return Err(ScriptError::DiscourageUpgradableWitnessProgram);
                }
                // Future soft-fork: accept (no witness items required).
            }

            // Witness execution consumes the witness; nothing further to check
            // except that the witness stack was empty if no witness program ran.
            return Ok(());
        }

        // No witness program was found — if the input still carries a witness,
        // that's unexpected (BIP141 rule).
        if !tx.input(n_in).witness().is_empty() {
            return Err(ScriptError::WitnessUnexpected);
        }
    }

    if flags.contains(ScriptFlags::CLEANSTACK)
        && flags.contains(ScriptFlags::P2SH)
        && cleanstack_len != 1
    {
        return Err(ScriptError::CleanStack);
    }

    Ok(())
}

/// Dispatch a taproot (witness v1) spend.
///
/// `program32` is the 32-byte witness program (the x-only output key).
/// `witness_items` is the full witness stack. `spent_spk` is the scriptPubKey
/// of the UTXO being spent (needed for BIP341 sighash).
#[allow(clippy::too_many_arguments)]
fn run_taproot<T: TxView>(
    program32: &[u8; 32],
    witness_items: &[Vec<u8>],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    spent_spk: &[u8],
    flags: ScriptFlags,
    bufs: &mut (Vec<InterpStackElement>, Vec<InterpStackElement>),
) -> Result<(), ScriptError> {
    // Annex: the last witness item starting with 0x50 is the annex.
    // It is committed to in the sighash but not pushed to the script stack.
    let (annex, effective_witness): (Option<&[u8]>, &[Vec<u8>]) = if witness_items.len() >= 2
        && witness_items
            .last()
            .is_some_and(|w| w.first() == Some(&0x50))
    {
        let annex_item = witness_items.last().expect("checked len");
        (
            Some(annex_item.as_slice()),
            &witness_items[..witness_items.len() - 1],
        )
    } else {
        (None, witness_items)
    };

    // Build the BIP341 cache (needed for both key-path and script-path sighash).
    let bip341_cache = Bip341Cache::new(tx, &[amount], &[spent_spk]);

    match effective_witness.len() {
        // ── Key-path spend ──────────────────────────────────────────────────
        // Exactly one item: the Schnorr signature (64 or 65 bytes).
        1 => {
            let sig = &effective_witness[0];

            let Ok((sig64, hash_type_byte)) = Schnorr::check_sig_encoding(sig) else {
                return Err(ScriptError::SchnorrSigInvalidFormat);
            };
            let hash_type = super::sighash::TaprootSigHashType(hash_type_byte);
            if !hash_type.is_valid() {
                return Err(ScriptError::SchnorrSigHashType);
            }

            // Annex is included in sighash for key-path too.
            let annex_body = annex.map(|a| &a[1..]); // strip 0x50 prefix byte
            let digest = tx.bip341_sighash(
                n_in,
                amount,
                spent_spk,
                hash_type,
                &bip341_cache,
                annex_body,
                None, // key-path: no leaf_hash
            );

            let pubkey_arr: &[u8; 32] = program32;
            if !Schnorr::verify(sig64, pubkey_arr, &digest) {
                return Err(ScriptError::TaprootVerify);
            }
            Ok(())
        }

        // ── Script-path spend ────────────────────────────────────────────────
        // Two or more items: [...stack inputs..., script, control_block].
        n if n >= 2 => {
            let control_block = effective_witness.last().expect("n>=2");
            let script_item = &effective_witness[effective_witness.len() - 2];
            let stack_items = &effective_witness[..effective_witness.len() - 2];

            // Control block: 1 byte (leaf_version | parity) + 32 bytes (internal key)
            // + 32*m bytes (merkle path, 0 ≤ m ≤ 128).
            if control_block.len() < 33 || (control_block.len() - 33) % 32 != 0 {
                return Err(ScriptError::TaprootWrongControlSize);
            }
            let path_len = (control_block.len() - 33) / 32;
            if path_len > 128 {
                return Err(ScriptError::TaprootWrongControlSize);
            }

            let leaf_version_parity = control_block[0];
            let leaf_version = leaf_version_parity & 0xfe; // strip parity bit
            let internal_key: &[u8; 32] = control_block[1..33].try_into().expect("slice is 32");

            if leaf_version != 0xc0
                && flags.contains(ScriptFlags::DISCOURAGE_UPGRADABLE_TAPROOT_VERSION)
            {
                return Err(ScriptError::DiscourageUpgradableTaprootVersion);
            }

            // Compute leaf hash.
            let leaf_hash = TapHasher::leaf(leaf_version, script_item);

            // Compute merkle root by walking the path.
            let merkle_root: [u8; 32] = {
                let mut current = leaf_hash;
                for i in 0..path_len {
                    let sibling: &[u8; 32] = control_block[33 + i * 32..33 + (i + 1) * 32]
                        .try_into()
                        .expect("slice is 32");
                    // BIP341: sort by value for branch hashing.
                    current = if current <= *sibling {
                        TapHasher::branch(&current, sibling)
                    } else {
                        TapHasher::branch(sibling, &current)
                    };
                }
                current
            };

            // Verify the tweaked output key commits to (internal_key, merkle_root).
            let expected_output_key = TapHasher::tweak_output_key(internal_key, &merkle_root)
                .ok_or(ScriptError::XOnlyPubkeyTweak)?;
            if &expected_output_key != program32 {
                return Err(ScriptError::WitnessProgramMismatchTaproot);
            }

            // Execute the tapscript.
            run_tapscript(
                script_item,
                stack_items,
                leaf_hash,
                tx,
                n_in,
                amount,
                spent_spk,
                flags,
                bip341_cache,
                bufs,
            )
        }

        _ => Err(ScriptError::WitnessProgramMismatch),
    }
}

/// Execute a tapscript (BIP342).
///
/// `stack_items` are the witness items that become the initial stack.
/// `leaf_hash` is the `TapLeaf` hash for this leaf. The script runs under
/// `SigVersion::Tapscript` rules.
#[allow(clippy::too_many_arguments)]
fn run_tapscript<T: TxView>(
    script: &[u8],
    stack_items: &[Vec<u8>],
    leaf_hash: [u8; 32],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    spent_spk: &[u8],
    flags: ScriptFlags,
    bip341_cache: Bip341Cache,
    bufs: &mut (Vec<InterpStackElement>, Vec<InterpStackElement>),
) -> Result<(), ScriptError> {
    let checker = TxSigChecker::new(tx, n_in, amount, flags);
    let stack_buf = std::mem::take(&mut bufs.0);
    let altstack_buf = std::mem::take(&mut bufs.1);
    let mut interp = Interpreter::new_with_bufs(
        &checker,
        flags,
        SigVersion::Tapscript,
        stack_buf,
        altstack_buf,
    );
    interp.tapscript = Some(TapscriptCtx {
        leaf_hash,
        spent_amount: amount,
        spent_spk: spent_spk.to_vec(),
        bip341_cache,
    });

    let result = (|| {
        // Push the initial stack items bottom-to-top.
        for item in stack_items {
            interp.stack_mut().push_checked(item, 0)?;
        }
        interp.eval(script)?;
        if interp.stack().is_empty() {
            return Err(ScriptError::EvalFalse);
        }
        if !interp.stack().peek(0)?.is_truthy() {
            return Err(ScriptError::EvalFalse);
        }
        // BIP342: tapscript execution requires a clean stack.
        if interp.stack().len() != 1 {
            return Err(ScriptError::CleanStack);
        }
        Ok(())
    })();

    let (s, a) = interp.into_bufs();
    bufs.0 = s;
    bufs.1 = a;
    result
}

/// Run a segwit v0 script against a pre-loaded stack under `WitnessV0` sighash rules.
///
/// `stack_items` are pushed bottom-to-top before `script` is evaluated.
/// The interpreter runs under `SigVersion::WitnessV0` so `check_ecdsa_sig`
/// will use the BIP143 sighash path.
fn eval_witness_script<T: TxView>(
    stack_items: &[&[u8]], // caller builds this on the stack — no heap alloc
    script: &[u8],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
    bufs: &mut (Vec<InterpStackElement>, Vec<InterpStackElement>),
) -> Result<(), ScriptError> {
    let checker = TxSigChecker::new(tx, n_in, amount, flags);
    let stack_buf = std::mem::take(&mut bufs.0);
    let altstack_buf = std::mem::take(&mut bufs.1);
    let mut interp = Interpreter::new_with_bufs(
        &checker,
        flags,
        SigVersion::WitnessV0,
        stack_buf,
        altstack_buf,
    );

    let result = (|| {
        for item in stack_items {
            interp.stack_mut().push_checked(item, 0)?;
        }
        interp.eval(script)?;
        if interp.stack().is_empty() {
            return Err(ScriptError::EvalFalse);
        }
        if !interp.stack().peek(0)?.is_truthy() {
            return Err(ScriptError::EvalFalse);
        }
        // BIP141: witness execution always requires a clean stack.
        if interp.stack().len() != 1 {
            return Err(ScriptError::CleanStack);
        }
        Ok(())
    })();

    let (s, a) = interp.into_bufs();
    bufs.0 = s;
    bufs.1 = a;
    result
}

/// Dispatch a segwit v0 witness program (P2WPKH or P2WSH).
///
/// `program` is the 20- or 32-byte witness program extracted from scriptPubKey.
/// `witness_items` is the full witness stack for this input as owned byte vectors.
fn run_witness_v0<T: TxView>(
    program: &[u8],
    witness_items: &[Vec<u8>],
    tx: &T,
    n_in: usize,
    amount: Satoshi,
    flags: ScriptFlags,
    bufs: &mut (Vec<InterpStackElement>, Vec<InterpStackElement>),
) -> Result<(), ScriptError> {
    match program.len() {
        20 => {
            // P2WPKH: witness must be exactly [<sig>, <pubkey>].
            if witness_items.len() != 2 {
                return Err(ScriptError::WitnessProgramMismatch);
            }
            // Synthetic P2PKH scriptCode over the 20-byte key hash.
            let mut script_code = [0u8; 25];
            script_code[0] = 0x76; // OP_DUP
            script_code[1] = 0xa9; // OP_HASH160
            script_code[2] = 0x14; // push 20
            script_code[3..23].copy_from_slice(program);
            script_code[23] = 0x88; // OP_EQUALVERIFY
            script_code[24] = 0xac; // OP_CHECKSIG

            let refs: [&[u8]; 2] = [&witness_items[0], &witness_items[1]];
            eval_witness_script(&refs, &script_code, tx, n_in, amount, flags, bufs)
        }
        32 => {
            use sha2::{Digest, Sha256};

            // For the common case (≤ INLINE items) build the &[u8] slice on the
            // stack to avoid a heap allocation; fall back to Vec for deep witness
            // stacks (unusual in practice).
            const INLINE: usize = 8;

            // P2WSH: last item is the witness script; the rest are stack inputs.
            let Some((script_item, stack_items)) = witness_items.split_last() else {
                return Err(ScriptError::WitnessProgramMismatch);
            };

            // SHA256 commitment (single, not double).
            let script_hash: [u8; 32] = Sha256::digest(script_item).into();
            if script_hash != program {
                return Err(ScriptError::WitnessProgramMismatch);
            }

            if stack_items.len() <= INLINE {
                let mut buf = [<&[u8]>::default(); INLINE];
                for (i, item) in stack_items.iter().enumerate() {
                    buf[i] = item.as_slice();
                }
                eval_witness_script(
                    &buf[..stack_items.len()],
                    script_item,
                    tx,
                    n_in,
                    amount,
                    flags,
                    bufs,
                )
            } else {
                let refs: Vec<&[u8]> = stack_items.iter().map(Vec::as_slice).collect();
                eval_witness_script(&refs, script_item, tx, n_in, amount, flags, bufs)
            }
        }
        _ => Err(ScriptError::WitnessProgramMismatch),
    }
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

/// If `script` is a BIP141 witness program, return `(version, program_bytes)`.
///
/// A witness program is `OP_n <push of 2..=40 bytes>` where `n` is 0–16.
/// `OP_0` is byte `0x00`; `OP_1`–`OP_16` are `0x51`–`0x60`.
#[must_use]
fn witness_program_of(script: &[u8]) -> Option<(u8, Vec<u8>)> {
    if script.len() < 4 || script.len() > 42 {
        return None;
    }
    let version = match script[0] {
        0x00 => 0u8,
        v @ 0x51..=0x60 => v - 0x50,
        _ => return None,
    };
    let push_len = usize::from(script[1]);
    // The push opcode must be a direct push (0x02..=0x28 = 2..=40 bytes).
    if !(2..=40).contains(&push_len) {
        return None;
    }
    if script.len() != 2 + push_len {
        return None;
    }
    Some((version, script[2..].to_vec()))
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
mod tests;
