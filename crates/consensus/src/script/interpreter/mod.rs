//! Bitcoin script dispatch loop.
//!
//! The `Interpreter` owns a main stack, alt stack, condition stack
//! (`OP_IF`/`OP_NOTIF` nesting), opcode counter, last-`OP_CODESEPARATOR`
//! offset, the script flags, the sighash regime, and a borrowed
//! `SignatureChecker`. Each opcode handler is a method on `Interpreter`
//! living in one of the `op_*.rs` files in this directory.
//!
//! The `eval` entry point runs the pre-scan (`pre_scan.rs`), then iterates
//! through `ScriptReader::next_step` (`reader.rs`) calling `step` for each
//! decoded opcode. The match in `step` is the dispatch; the methods are
//! the logic.

pub mod op_arith;
pub mod op_bitwise;
pub mod op_control;
pub mod op_crypto;
pub mod op_stack;
pub mod pre_scan;
pub mod reader;

use crate::script::checker::SignatureChecker;
use crate::script::condition_stack::ConditionStack;
use crate::script::error::ScriptError;
use crate::script::flags::ScriptFlags;
use crate::script::opcodes::Opcode;
use crate::script::sig_version::SigVersion;
use crate::script::stack::Stack;

use reader::{ScriptReader, Step};

/// Maximum non-push opcodes per script (`MAX_OPS_PER_SCRIPT`). `OP_0`,
/// `OP_PUSHBYTES_*`, `OP_PUSHDATA*`, `OP_1NEGATE`, and `OP_1`..`OP_16` are
/// not counted.
pub const MAX_OPS_PER_SCRIPT: u32 = 201;

/// Dispatch loop and execution state for a single script invocation.
pub struct Interpreter<'a, C: SignatureChecker> {
    pub(super) stack: Stack,
    pub(super) altstack: Stack,
    pub(super) conditions: ConditionStack,
    pub(super) op_count: u32,
    /// Byte offset within the current script of the last `OP_CODESEPARATOR`,
    /// or 0 if none has been seen. Used by sighash to slice scriptCode in M1.2.
    #[allow(dead_code)] // wired in M1.2
    pub(super) last_codeseparator: usize,
    pub(super) flags: ScriptFlags,
    pub(super) sig_version: SigVersion,
    /// Read by the signature opcodes in M1.2.
    #[allow(dead_code)] // wired in M1.2
    pub(super) checker: &'a C,
}

impl<'a, C: SignatureChecker> Interpreter<'a, C> {
    /// Construct an interpreter ready to evaluate a script under the given
    /// flags and sighash regime.
    #[must_use]
    pub fn new(checker: &'a C, flags: ScriptFlags, sig_version: SigVersion) -> Self {
        Self {
            stack: Stack::new(),
            altstack: Stack::new(),
            conditions: ConditionStack::new(),
            op_count: 0,
            last_codeseparator: 0,
            flags,
            sig_version,
            checker,
        }
    }

    /// Borrow the main stack — for tests and for the upcoming
    /// `verify_script` final-stack-truthy check.
    #[must_use]
    pub fn stack(&self) -> &Stack {
        &self.stack
    }

    /// Mutable access to the main stack — for tests that need to seed
    /// initial state without going through a script.
    pub fn stack_mut(&mut self) -> &mut Stack {
        &mut self.stack
    }

    /// Evaluate `script` to completion.
    ///
    /// On success, the final stack is left in `self.stack` for the caller
    /// to inspect (e.g. for the `verify_script` truthy-top check).
    ///
    /// # Errors
    /// Any `ScriptError` produced by the pre-scan or by an opcode handler.
    pub fn eval(&mut self, script: &[u8]) -> Result<(), ScriptError> {
        pre_scan::pre_scan(script, self.sig_version)?;

        let mut reader = ScriptReader::new(script);
        while let Some(step) = reader.next_step()? {
            self.step(step)?;
        }

        if !self.conditions.is_empty() {
            return Err(ScriptError::UnbalancedConditional);
        }

        Ok(())
    }

    /// Dispatch one decoded step. Public to the parent module so individual
    /// opcodes can be tested in isolation by feeding hand-built `Step`s.
    pub(super) fn step(&mut self, step: Step<'_>) -> Result<(), ScriptError> {
        let executing = self.conditions.executing();

        // Op count is incremented for every executed non-push opcode that
        // is `> OP_16` (0x60). Push opcodes don't count.
        if !step.opcode.is_push() && step.opcode.to_byte() > Opcode::OpN(16).to_byte() {
            self.op_count += 1;
            if self.op_count > MAX_OPS_PER_SCRIPT {
                return Err(ScriptError::OpCount);
            }
        }

        // Conditional opcodes are *always* dispatched — they manipulate the
        // condition stack regardless of the executing flag. Everything else
        // is skipped when not executing.
        let is_conditional = matches!(
            step.opcode,
            Opcode::If | Opcode::NotIf | Opcode::Else | Opcode::EndIf
        );
        if !executing && !is_conditional {
            return Ok(());
        }

        self.dispatch(step)
    }

    /// The big match. Each arm calls the corresponding `op_*` method.
    #[allow(
        clippy::too_many_lines,    // dispatch table; methods do the work
        clippy::match_same_arms,   // arms grouped by category for readability
    )]
    fn dispatch(&mut self, step: Step<'_>) -> Result<(), ScriptError> {
        match step.opcode {
            // ── Push opcodes ────────────────────────────────────────────────
            Opcode::Op0 => self.stack.push_checked(&[], self.altstack.len()),
            Opcode::PushBytes(_)
            | Opcode::PushData1
            | Opcode::PushData2
            | Opcode::PushData4 => self
                .stack
                .push_checked(step.data, self.altstack.len()),
            Opcode::Op1Negate => self.stack.push_num(-1, self.altstack.len()),
            Opcode::OpN(n) => self.stack.push_num(i64::from(n), self.altstack.len()),

            // ── Control flow ────────────────────────────────────────────────
            Opcode::If => self.op_if(false),
            Opcode::NotIf => self.op_if(true),
            Opcode::Else => self.op_else(),
            Opcode::EndIf => self.op_endif(),
            Opcode::Verify => self.op_verify(),
            Opcode::Return => self.op_return(),
            Opcode::Nop => self.op_nop(),
            Opcode::Nop1
            | Opcode::Nop4
            | Opcode::Nop5
            | Opcode::Nop6
            | Opcode::Nop7
            | Opcode::Nop8
            | Opcode::Nop9
            | Opcode::Nop10 => self.op_nop_upgradable(),

            // ── Stack manipulation ──────────────────────────────────────────
            Opcode::ToAltStack => self.op_to_alt_stack(),
            Opcode::FromAltStack => self.op_from_alt_stack(),
            Opcode::Drop => self.op_drop(),
            Opcode::TwoDrop => self.op_2drop(),
            Opcode::Dup => self.op_dup(),
            Opcode::TwoDup => self.op_2dup(),
            Opcode::ThreeDup => self.op_3dup(),
            Opcode::Over => self.op_over(),
            Opcode::TwoOver => self.op_2over(),
            Opcode::Swap => self.op_swap(),
            Opcode::TwoSwap => self.op_2swap(),
            Opcode::Rot => self.op_rot(),
            Opcode::TwoRot => self.op_2rot(),
            Opcode::Nip => self.op_nip(),
            Opcode::Tuck => self.op_tuck(),
            Opcode::Pick => self.op_pick(),
            Opcode::Roll => self.op_roll(),
            Opcode::Depth => self.op_depth(),
            Opcode::IfDup => self.op_if_dup(),
            Opcode::Size => self.op_size(),

            // ── Bitwise ─────────────────────────────────────────────────────
            Opcode::Equal => self.op_equal(),
            Opcode::EqualVerify => self.op_equal_verify(),

            // ── Arithmetic ──────────────────────────────────────────────────
            Opcode::OneAdd => self.op_1add(),
            Opcode::OneSub => self.op_1sub(),
            Opcode::Negate => self.op_negate(),
            Opcode::Abs => self.op_abs(),
            Opcode::Not => self.op_not(),
            Opcode::ZeroNotEqual => self.op_0not_equal(),
            Opcode::Add => self.op_add(),
            Opcode::Sub => self.op_sub(),
            Opcode::BoolAnd => self.op_bool_and(),
            Opcode::BoolOr => self.op_bool_or(),
            Opcode::NumEqual => self.op_num_equal(),
            Opcode::NumEqualVerify => self.op_num_equal_verify(),
            Opcode::NumNotEqual => self.op_num_not_equal(),
            Opcode::LessThan => self.op_less_than(),
            Opcode::GreaterThan => self.op_greater_than(),
            Opcode::LessThanOrEqual => self.op_less_than_or_equal(),
            Opcode::GreaterThanOrEqual => self.op_greater_than_or_equal(),
            Opcode::Min => self.op_min(),
            Opcode::Max => self.op_max(),
            Opcode::Within => self.op_within(),

            // ── Crypto ──────────────────────────────────────────────────────
            Opcode::Ripemd160 => self.op_ripemd160(),
            Opcode::Sha1 => self.op_sha1(),
            Opcode::Sha256 => self.op_sha256(),
            Opcode::Hash160 => self.op_hash160(),
            Opcode::Hash256 => self.op_hash256(),

            // ── M1.2 stubs ──────────────────────────────────────────────────
            Opcode::CodeSeparator => self.op_code_separator_unsupported(),
            Opcode::CheckSig | Opcode::CheckSigVerify => self.op_check_sig_unsupported(),
            Opcode::CheckMultiSig | Opcode::CheckMultiSigVerify => {
                self.op_check_multi_sig_unsupported()
            }
            Opcode::CheckLockTimeVerify => self.op_check_lock_time_verify_unsupported(),
            Opcode::CheckSequenceVerify => self.op_check_sequence_verify_unsupported(),

            // ── Always-fail-when-executed ──────────────────────────────────
            // OP_RESERVED, OP_VER, OP_RESERVED1/2 fail when reached during
            // execution. (Note OP_VERIF/OP_VERNOTIF are pre-scanned out.)
            Opcode::OpReserved
            | Opcode::Ver
            | Opcode::Reserved1
            | Opcode::Reserved2 => Err(ScriptError::BadOpcodeReserved),

            // ── Disabled even-when-unexecuted in legacy ────────────────────
            // pre_scan rejects these in SigVersion::Base before we get here;
            // under tapscript they fail when executed.
            Opcode::Cat
            | Opcode::SubStr
            | Opcode::Left
            | Opcode::Right
            | Opcode::Invert
            | Opcode::And
            | Opcode::Or
            | Opcode::Xor
            | Opcode::TwoMul
            | Opcode::TwoDiv
            | Opcode::Mul
            | Opcode::Div
            | Opcode::Mod
            | Opcode::LShift
            | Opcode::RShift => Err(ScriptError::DisabledOpcode),

            // ── M3 stub ─────────────────────────────────────────────────────
            Opcode::CheckSigAdd => Err(ScriptError::BadOpcode),

            // ── Pre-scanned out (defensive) ─────────────────────────────────
            Opcode::VerIf | Opcode::VerNotIf => Err(ScriptError::BadOpcodeReserved),

            // ── Anything else ──────────────────────────────────────────────
            Opcode::Invalid(_) => Err(ScriptError::BadOpcode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::checker::NopSigChecker;

    fn run(script: &[u8]) -> Result<Stack, ScriptError> {
        let checker = NopSigChecker;
        let mut interp = Interpreter::new(&checker, ScriptFlags::NONE, SigVersion::Base);
        interp.eval(script)?;
        Ok(interp.stack)
    }

    fn run_flags(script: &[u8], flags: ScriptFlags) -> Result<Stack, ScriptError> {
        let checker = NopSigChecker;
        let mut interp = Interpreter::new(&checker, flags, SigVersion::Base);
        interp.eval(script)?;
        Ok(interp.stack)
    }

    // ── Push opcodes ────────────────────────────────────────────────────────

    #[test]
    fn op_0_pushes_empty() {
        let s = run(&[0x00]).expect("ok");
        assert_eq!(s.len(), 1);
        assert!(s.peek(0).expect("non-empty").is_empty());
    }

    #[test]
    fn pushbytes_pushes_data() {
        // OP_PUSHBYTES_3 0xaa 0xbb 0xcc
        let s = run(&[0x03, 0xaa, 0xbb, 0xcc]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn op_1negate_pushes_minus_one() {
        // OP_1NEGATE OP_1ADD  →  0
        let s = run(&[0x4f, 0x8b]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[]); // 0 encodes as empty
    }

    #[test]
    fn op_n_pushes_constant() {
        // OP_5 → top is [0x05]
        let s = run(&[0x55]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x05]);
    }

    // ── Stack ops ───────────────────────────────────────────────────────────

    #[test]
    fn op_dup() {
        // OP_3 OP_DUP  →  [3, 3]
        let s = run(&[0x53, 0x76]).expect("ok");
        assert_eq!(s.len(), 2);
        assert_eq!(s.peek(0).expect("ok"), &[0x03]);
        assert_eq!(s.peek(1).expect("ok"), &[0x03]);
    }

    #[test]
    fn op_drop() {
        // OP_1 OP_2 OP_DROP  →  [1]
        let s = run(&[0x51, 0x52, 0x75]).expect("ok");
        assert_eq!(s.len(), 1);
        assert_eq!(s.peek(0).expect("ok"), &[0x01]);
    }

    #[test]
    fn op_swap() {
        // OP_1 OP_2 OP_SWAP  →  bottom 2, top 1
        let s = run(&[0x51, 0x52, 0x7c]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x01]);
        assert_eq!(s.peek(1).expect("ok"), &[0x02]);
    }

    #[test]
    fn alt_stack_roundtrip() {
        // OP_5 OP_TOALTSTACK OP_FROMALTSTACK  →  [5]
        let s = run(&[0x55, 0x6b, 0x6c]).expect("ok");
        assert_eq!(s.len(), 1);
        assert_eq!(s.peek(0).expect("ok"), &[0x05]);
    }

    // ── Arithmetic ──────────────────────────────────────────────────────────

    #[test]
    fn op_add() {
        // OP_3 OP_4 OP_ADD  →  [7]
        let s = run(&[0x53, 0x54, 0x93]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x07]);
    }

    #[test]
    fn op_sub_negative_result() {
        // OP_3 OP_5 OP_SUB  →  -2
        let s = run(&[0x53, 0x55, 0x94]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x82]); // -2 encoded
    }

    #[test]
    fn op_within_in_range() {
        // 5 0 10 OP_WITHIN  →  true ([0x01])
        let s = run(&[0x55, 0x00, 0x5a, 0xa5]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x01]);
    }

    // ── Comparison ──────────────────────────────────────────────────────────

    #[test]
    fn op_equal_true() {
        // OP_5 OP_5 OP_EQUAL  →  [0x01]
        let s = run(&[0x55, 0x55, 0x87]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x01]);
    }

    #[test]
    fn op_equal_verify_passes_consumes_both() {
        // OP_5 OP_5 OP_EQUALVERIFY  →  empty stack, ok
        let s = run(&[0x55, 0x55, 0x88]).expect("ok");
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn op_equal_verify_fails_on_mismatch() {
        // OP_5 OP_6 OP_EQUALVERIFY  →  Verify
        let r = run(&[0x55, 0x56, 0x88]);
        assert!(matches!(r, Err(ScriptError::Verify)));
    }

    // ── Hashes ──────────────────────────────────────────────────────────────

    #[test]
    fn op_sha256_empty() {
        // OP_0 OP_SHA256  →  SHA256("") = e3b0c442…
        let s = run(&[0x00, 0xa8]).expect("ok");
        let top = s.peek(0).expect("ok");
        assert_eq!(top.len(), 32);
        assert_eq!(&top[..4], &[0xe3, 0xb0, 0xc4, 0x42]);
    }

    #[test]
    fn op_hash160_known_vector() {
        // OP_0 OP_HASH160  →  RIPEMD160(SHA256(""))
        // SHA256("") = e3b0c442…
        // RIPEMD160(SHA256("")) = b472a266d0bd89c13706a4132ccfb16f7c3b9fcb
        let s = run(&[0x00, 0xa9]).expect("ok");
        let top = s.peek(0).expect("ok");
        assert_eq!(top.len(), 20);
        assert_eq!(
            top,
            &[
                0xb4, 0x72, 0xa2, 0x66, 0xd0, 0xbd, 0x89, 0xc1, 0x37, 0x06,
                0xa4, 0x13, 0x2c, 0xcf, 0xb1, 0x6f, 0x7c, 0x3b, 0x9f, 0xcb,
            ]
        );
    }

    #[test]
    fn op_hash256_empty_known_vector() {
        // SHA256d("") = 5df6e0e2…
        let s = run(&[0x00, 0xaa]).expect("ok");
        let top = s.peek(0).expect("ok");
        assert_eq!(top.len(), 32);
        assert_eq!(&top[..4], &[0x5d, 0xf6, 0xe0, 0xe2]);
    }

    #[test]
    fn op_ripemd160_known_vector() {
        // RIPEMD160("") = 9c1185a5c5e9fc54612808977ee8f548b2258d31
        let s = run(&[0x00, 0xa6]).expect("ok");
        let top = s.peek(0).expect("ok");
        assert_eq!(top.len(), 20);
        assert_eq!(&top[..4], &[0x9c, 0x11, 0x85, 0xa5]);
    }

    #[test]
    fn op_sha1_known_vector() {
        // SHA1("") = da39a3ee5e6b4b0d3255bfef95601890afd80709
        let s = run(&[0x00, 0xa7]).expect("ok");
        let top = s.peek(0).expect("ok");
        assert_eq!(top.len(), 20);
        assert_eq!(&top[..4], &[0xda, 0x39, 0xa3, 0xee]);
    }

    // ── Control flow ────────────────────────────────────────────────────────

    #[test]
    fn op_if_taken_branch_executes() {
        // OP_1 OP_IF OP_5 OP_ELSE OP_6 OP_ENDIF  →  [5]
        let s = run(&[0x51, 0x63, 0x55, 0x67, 0x56, 0x68]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x05]);
    }

    #[test]
    fn op_if_skipped_branch_runs_else() {
        // OP_0 OP_IF OP_5 OP_ELSE OP_6 OP_ENDIF  →  [6]
        let s = run(&[0x00, 0x63, 0x55, 0x67, 0x56, 0x68]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x06]);
    }

    #[test]
    fn op_notif_inverts_condition() {
        // OP_0 OP_NOTIF OP_5 OP_ENDIF  →  [5]
        let s = run(&[0x00, 0x64, 0x55, 0x68]).expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x05]);
    }

    #[test]
    fn unbalanced_if_fails() {
        // OP_1 OP_IF OP_5 (no ENDIF)
        let r = run(&[0x51, 0x63, 0x55]);
        assert!(matches!(r, Err(ScriptError::UnbalancedConditional)));
    }

    #[test]
    fn unbalanced_endif_fails() {
        // OP_ENDIF with no preceding OP_IF
        let r = run(&[0x68]);
        assert!(matches!(r, Err(ScriptError::UnbalancedConditional)));
    }

    #[test]
    fn nested_if_executing_correctly() {
        // OP_1 OP_IF
        //   OP_1 OP_IF OP_7 OP_ELSE OP_8 OP_ENDIF
        // OP_ELSE
        //   OP_9
        // OP_ENDIF
        // → [7]
        let s = run(&[
            0x51, 0x63, 0x51, 0x63, 0x57, 0x67, 0x58, 0x68, 0x67, 0x59, 0x68,
        ])
        .expect("ok");
        assert_eq!(s.peek(0).expect("ok"), &[0x07]);
    }

    #[test]
    fn nested_if_outer_false_skips_everything() {
        // OP_0 OP_IF
        //   OP_1 OP_IF OP_7 OP_ELSE OP_8 OP_ENDIF
        // OP_ENDIF
        // → empty stack (outer false; nothing runs except IF/ELSE/ENDIF dispatch)
        let s = run(&[0x00, 0x63, 0x51, 0x63, 0x57, 0x67, 0x58, 0x68, 0x68])
            .expect("ok");
        assert_eq!(s.len(), 0);
    }

    // ── Always-fail / disabled ──────────────────────────────────────────────

    #[test]
    fn op_return_fails() {
        let r = run(&[0x6a]);
        assert!(matches!(r, Err(ScriptError::OpReturn)));
    }

    #[test]
    fn disabled_opcode_pre_scan_fails() {
        // OP_CAT (0x7e) — disabled in legacy.
        let r = run(&[0x7e]);
        assert!(matches!(r, Err(ScriptError::DisabledOpcode)));
    }

    #[test]
    fn op_verif_pre_scanned() {
        let r = run(&[0x65]);
        assert!(matches!(r, Err(ScriptError::BadOpcodeReserved)));
    }

    #[test]
    fn op_reserved_fails_when_executed() {
        // OP_RESERVED (0x50)
        let r = run(&[0x50]);
        assert!(matches!(r, Err(ScriptError::BadOpcodeReserved)));
    }

    #[test]
    fn op_reserved_skipped_when_unexecuted() {
        // OP_0 OP_IF OP_RESERVED OP_ENDIF  →  empty stack, no error
        let s = run(&[0x00, 0x63, 0x50, 0x68]).expect("ok");
        assert_eq!(s.len(), 0);
    }

    // ── Op count limit ──────────────────────────────────────────────────────

    #[test]
    fn op_count_limit() {
        // 202 OP_NOPs > MAX_OPS_PER_SCRIPT (201).
        let script = vec![0x61u8; 202];
        let r = run(&script);
        assert!(matches!(r, Err(ScriptError::OpCount)));

        // 201 OP_NOPs is exactly the cap and passes.
        let script = vec![0x61u8; 201];
        run(&script).expect("at-cap ok");
    }

    #[test]
    fn pushes_dont_count_against_op_limit() {
        // 500 OP_1 pushes — all pushes, none count against the 201 op limit.
        // This would have exceeded the limit if pushes counted (500 > 201).
        // The 1000-element stack cap holds because we stay at 500 < 1000.
        let script = vec![0x51u8; 500];
        run(&script).expect("ok");
    }

    // ── M1.2 stubs ──────────────────────────────────────────────────────────

    #[test]
    fn op_check_sig_is_stubbed_to_sig_der() {
        // OP_0 OP_0 OP_CHECKSIG  →  SigDer (M1.2 stub)
        let r = run(&[0x00, 0x00, 0xac]);
        assert!(matches!(r, Err(ScriptError::SigDer)));
    }

    // ── Upgradable NOPs ─────────────────────────────────────────────────────

    #[test]
    fn upgradable_nop_passes_by_default() {
        // OP_NOP4 (0xb3)
        run(&[0xb3]).expect("ok");
    }

    #[test]
    fn upgradable_nop_fails_under_discourage_flag() {
        let r = run_flags(&[0xb3], ScriptFlags::DISCOURAGE_UPGRADABLE_NOPS);
        assert!(matches!(r, Err(ScriptError::DiscourageUpgradableNops)));
    }
}
