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

// ── Signature opcodes ───────────────────────────────────────────────────

#[test]
fn op_check_sig_with_nop_checker_pushes_false() {
    // OP_0 OP_0 OP_CHECKSIG. NopSigChecker fails every check; under no
    // flags the result is just `false` pushed onto the stack.
    let s = run(&[0x00, 0x00, 0xac]).expect("ok");
    assert_eq!(s.len(), 1);
    assert!(!s.peek(0).expect("ok").is_truthy());
}

#[test]
fn op_check_sig_verify_fails_under_nop_checker() {
    // OP_0 OP_0 OP_CHECKSIGVERIFY  →  Verify (false top fails the verify form).
    let r = run(&[0x00, 0x00, 0xad]);
    assert!(matches!(r, Err(ScriptError::Verify)));
}

#[test]
fn op_code_separator_updates_offset() {
    // OP_CODESEPARATOR OP_0 OP_0 OP_CHECKSIG  →  pushes false (sighash
    // is computed over the suffix script after CODESEPARATOR; the
    // NopSigChecker fails regardless, so we just check it doesn't error).
    let s = run(&[0xab, 0x00, 0x00, 0xac]).expect("ok");
    assert_eq!(s.len(), 1);
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
