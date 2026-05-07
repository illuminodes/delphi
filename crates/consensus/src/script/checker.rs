//! `SignatureChecker` — the abstraction the interpreter uses to ask the
//! transaction context "does this signature verify?".
//!
//! Mirrors Bitcoin Core's `BaseSignatureChecker` virtual class, but as a Rust
//! trait — the interpreter is generic over `C: SignatureChecker` so dispatch
//! is monomorphised, no vtable.
//!
//! Production wires a `TxSigChecker { tx, n_in, amount }` (M1.2). Tests wire
//! `NopSigChecker` to exercise non-crypto opcodes without a transaction.

use super::sig_version::SigVersion;

/// Asks "does this signature, pubkey, and scriptCode verify?" for the script
/// being evaluated. The interpreter calls one of these methods for every
/// signature-checking opcode (`OP_CHECKSIG`, `OP_CHECKMULTISIG`,
/// `OP_CHECKLOCKTIMEVERIFY`, `OP_CHECKSEQUENCEVERIFY`).
pub trait SignatureChecker {
    /// Verify an ECDSA signature against a pubkey and the sighash computed
    /// from `script_code` (the scriptCode of the current `OP_CHECKSIG` call,
    /// already with `OP_CODESEPARATOR` handling applied) under `sig_version`.
    ///
    /// Returns `true` on success, `false` on any failure mode that consensus
    /// treats as "signature did not verify" — including malformed encodings,
    /// curve failures, and `NULLFAIL` violations. The interpreter does not
    /// distinguish *why* a signature failed; it only acts on the boolean.
    fn check_ecdsa_sig(
        &self,
        sig: &[u8],
        pubkey: &[u8],
        script_code: &[u8],
        sig_version: SigVersion,
    ) -> bool;

    /// `OP_CHECKLOCKTIMEVERIFY` — does this transaction's `nLockTime` permit
    /// spending under `locktime`?
    fn check_lock_time(&self, locktime: i64) -> bool;

    /// `OP_CHECKSEQUENCEVERIFY` — does this input's `nSequence` permit
    /// spending under `sequence`?
    fn check_sequence(&self, sequence: i64) -> bool;
}

/// A `SignatureChecker` that fails every check. Useful in tests that exercise
/// the non-crypto opcode set: any path that reaches a signature operation
/// will short-circuit to `false`, which the interpreter treats as a failed
/// `CHECKSIG`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NopSigChecker;

impl SignatureChecker for NopSigChecker {
    fn check_ecdsa_sig(
        &self,
        _sig: &[u8],
        _pubkey: &[u8],
        _script_code: &[u8],
        _sig_version: SigVersion,
    ) -> bool {
        false
    }

    fn check_lock_time(&self, _locktime: i64) -> bool {
        false
    }

    fn check_sequence(&self, _sequence: i64) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nop_checker_rejects_everything() {
        let c = NopSigChecker;
        assert!(!c.check_ecdsa_sig(&[], &[], &[], SigVersion::Base));
        assert!(!c.check_lock_time(0));
        assert!(!c.check_sequence(0));
    }
}
