//! Script execution failure modes.
//!
//! Variants are exhaustive — every consensus-distinct script failure has its
//! own variant. The differential fuzzer compares accept/reject only, but
//! internal logging gets the structured kind for free.

/// Reason a script execution failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    // ── Parsing / control ───────────────────────────────────────────────────
    /// Opcode byte does not correspond to any defined opcode.
    BadOpcode,
    /// Opcode is consensus-disabled (e.g. `OP_CAT`, `OP_SUBSTR`, …).
    DisabledOpcode,
    /// Push opcode declared more bytes than remain in the script.
    BadPushLength,
    /// Push exceeds the 520-byte per-element limit.
    PushSize,
    /// `OP_RETURN` was executed.
    OpReturn,
    /// `OP_VERIF` / `OP_VERNOTIF` reached during execution.
    BadOpcodeReserved,
    /// `OP_IF`/`OP_NOTIF`/`OP_ELSE`/`OP_ENDIF` nesting is malformed.
    UnbalancedConditional,

    // ── Stack ───────────────────────────────────────────────────────────────
    /// Stack does not have enough items for the opcode.
    InvalidStackOperation,
    /// Combined stack + altstack exceeded the 1000-item cap.
    StackSize,
    /// `OP_VERIFY`/`OP_EQUALVERIFY`/etc. found a falsey top-of-stack.
    Verify,
    /// Final stack does not contain exactly one truthy element (under `CLEANSTACK`).
    CleanStack,
    /// Final top-of-stack was falsey.
    EvalFalse,

    // ── Numbers ─────────────────────────────────────────────────────────────
    /// `ScriptNum` decoded from more bytes than the opcode allows
    /// (4 for arithmetic, 5 for `OP_CHECKLOCKTIMEVERIFY`).
    NumOverflow,
    /// `ScriptNum` was not minimally encoded under `MINIMALDATA`.
    NumNotMinimal,
    /// Push operation was not minimally encoded under `MINIMALDATA`.
    MinimalData,
    /// `OP_IF`/`OP_NOTIF` operand was not exactly 0 or 1 under `MINIMALIF`.
    MinimalIf,

    // ── Locktime ────────────────────────────────────────────────────────────
    /// `OP_CHECKLOCKTIMEVERIFY` / `OP_CHECKSEQUENCEVERIFY` saw a negative value.
    NegativeLocktime,
    /// Locktime not satisfied.
    UnsatisfiedLocktime,

    // ── Signatures / pubkeys ────────────────────────────────────────────────
    /// Signature is not strict-DER under `DERSIG` / `STRICTENC`.
    SigDer,
    /// Signature hashtype byte is not one of the known values under `STRICTENC`.
    SigHashtype,
    /// Signature has high-S under `LOW_S`.
    SigHighS,
    /// `scriptSig` contained non-push opcodes under `SIGPUSHONLY`.
    SigPushOnly,
    /// Pubkey is not a valid encoding under `STRICTENC`.
    PubKeyType,
    /// Pubkey count for `OP_CHECKMULTISIG` outside [0, 20].
    PubKeyCount,
    /// Signature count for `OP_CHECKMULTISIG` outside [0, `pubkey_count`].
    SigCount,
    /// Dummy element preceding `OP_CHECKMULTISIG` was not the empty push under `NULLDUMMY`.
    SigNullDummy,
    /// Signature was non-empty after a failed `OP_CHECKSIG`/`OP_CHECKMULTISIG`
    /// under `NULLFAIL`.
    SigNullFail,
    /// `OP_CODESEPARATOR` or `FindAndDelete` interaction with a signature was
    /// disallowed under `CONST_SCRIPTCODE`.
    OpCodeSeparator,

    // ── Witness / taproot (M2+ — placeholders so the enum doesn't churn) ────
    /// Witness program version or length is invalid.
    WitnessProgramMismatch,
    /// Witness was provided where none was expected.
    WitnessUnexpected,
    /// scriptSig was non-empty for a native witness input.
    WitnessMalleated,
    /// scriptSig contained anything other than the expected push for P2SH-wrapped witness.
    WitnessMalleatedP2sh,
    /// Witness program of unknown version under `DISCOURAGE_UPGRADABLE_WITNESS_PROGRAM`.
    DiscourageUpgradableWitnessProgram,
    /// `OP_NOPx` execution under `DISCOURAGE_UPGRADABLE_NOPS`.
    DiscourageUpgradableNops,

    // ── Limits ──────────────────────────────────────────────────────────────
    /// Script length exceeds the 10,000-byte limit.
    ScriptSize,
    /// Op count (non-push opcodes) exceeded the 201-per-script limit.
    OpCount,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ScriptError {}
