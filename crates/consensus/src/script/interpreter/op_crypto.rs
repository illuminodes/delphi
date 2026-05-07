//! Hashing and signature opcodes.
//!
//! Hash opcodes — `OP_RIPEMD160`, `OP_SHA1`, `OP_SHA256`, `OP_HASH160`,
//! `OP_HASH256` — pop one byte string and push its digest. They have no
//! interpretation rules: any byte string is a valid input.
//!
//! Signature opcodes — `OP_CHECKSIG`, `OP_CHECKSIGVERIFY`,
//! `OP_CHECKMULTISIG`, `OP_CHECKMULTISIGVERIFY` — and the `OP_CODESEPARATOR`
//! / `OP_CHECKLOCKTIMEVERIFY` / `OP_CHECKSEQUENCEVERIFY` opcodes are stubbed
//! for M1.1; M1.2 implements them. They return `ScriptError::SigDer` until
//! then so any reachable use of them fails the script with a structured
//! error rather than `unimplemented!()`.

use ripemd::Ripemd160;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::script::checker::SignatureChecker;
use crate::script::error::ScriptError;

use super::Interpreter;

impl<C: SignatureChecker> Interpreter<'_, C> {
    /// Pop the top, hash it with `H`, push the digest.
    ///
    /// The digest goes through `push_checked(&[u8], …)` which copies into an
    /// inline `StackElement` slot — no `Vec` allocation. SHA-256 / SHA-1 /
    /// RIPEMD-160 outputs (32, 20, 20 bytes) all fit in the default 32-byte
    /// inline cap.
    fn hash_top<H: Digest>(&mut self) -> Result<(), ScriptError> {
        let top = self.stack.pop()?;
        let digest = H::digest(top.as_slice());
        self.stack.push_checked(digest.as_slice(), self.altstack.len())
    }

    pub(super) fn op_ripemd160(&mut self) -> Result<(), ScriptError> {
        self.hash_top::<Ripemd160>()
    }

    pub(super) fn op_sha1(&mut self) -> Result<(), ScriptError> {
        self.hash_top::<Sha1>()
    }

    pub(super) fn op_sha256(&mut self) -> Result<(), ScriptError> {
        self.hash_top::<Sha256>()
    }

    /// `OP_HASH160` = `RIPEMD160(SHA256(top))`.
    pub(super) fn op_hash160(&mut self) -> Result<(), ScriptError> {
        let top = self.stack.pop()?;
        let inner = Sha256::digest(top.as_slice());
        let outer = Ripemd160::digest(inner);
        self.stack.push_checked(outer.as_slice(), self.altstack.len())
    }

    /// `OP_HASH256` = `SHA256(SHA256(top))`. Reuses `mantle::hash::sha256d`.
    pub(super) fn op_hash256(&mut self) -> Result<(), ScriptError> {
        let top = self.stack.pop()?;
        let digest = mantle::hash::sha256d(top.as_slice());
        self.stack.push_checked(&digest, self.altstack.len())
    }

    // ── M1.2 stubs ──────────────────────────────────────────────────────────
    //
    // These take `&mut self` to match the dispatch shape; M1.2 fills in real
    // bodies that consume stack and read `self.checker` / `self.last_codeseparator`.

    /// TODO(M1.2): legacy sighash + ECDSA verification through `consensus_crypto`.
    #[allow(clippy::unused_self)] // body lands in M1.2
    pub(super) fn op_check_sig_unsupported(&mut self) -> Result<(), ScriptError> {
        Err(ScriptError::SigDer)
    }

    /// TODO(M1.2): n-of-m multisig including the `OP_0` dummy bug.
    #[allow(clippy::unused_self)] // body lands in M1.2
    pub(super) fn op_check_multi_sig_unsupported(&mut self) -> Result<(), ScriptError> {
        Err(ScriptError::SigDer)
    }

    /// TODO(M1.2): mark current position as the new `last_codeseparator`.
    #[allow(clippy::unused_self)] // body lands in M1.2
    pub(super) fn op_code_separator_unsupported(&mut self) -> Result<(), ScriptError> {
        Err(ScriptError::OpCodeSeparator)
    }

    /// TODO(M4): `OP_CHECKLOCKTIMEVERIFY`.
    #[allow(clippy::unused_self)] // body lands in M4
    pub(super) fn op_check_lock_time_verify_unsupported(&mut self) -> Result<(), ScriptError> {
        Err(ScriptError::UnsatisfiedLocktime)
    }

    /// TODO(M4): `OP_CHECKSEQUENCEVERIFY`.
    #[allow(clippy::unused_self)] // body lands in M4
    pub(super) fn op_check_sequence_verify_unsupported(&mut self) -> Result<(), ScriptError> {
        Err(ScriptError::UnsatisfiedLocktime)
    }
}
