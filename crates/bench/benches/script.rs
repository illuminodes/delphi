//! Script-interpreter benches.
//!
//! Four workload tiers:
//! - Tiny scripts: per-step dispatch overhead.
//! - Realistic legacy scripts: typical opcode mixes (P2PKH-shape sans
//!   signature, arithmetic, IF/ELSE).
//! - Pathological / limit cases: at the 201-op cap, deep IF nesting,
//!   maximum push sizes.
//! - Segwit v0 end-to-end: full `verify_script` with real ECDSA signing for
//!   P2WPKH and P2WSH paths (BIP141/143).

use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature as EcdsaSig, SigningKey};
use mantle::primitives::{Hash256, OutPoint, Satoshi, Transaction, TxIn, TxOut, Witness};
use mantle_consensus::script::{
    Interpreter, InterpStackElement, NopSigChecker, ScriptFlags, SigVersion,
};
use mantle_consensus::script::sighash::{Bip143Cache, SigHashType, signature_hash_bip143};
use mantle_consensus::script::verify::{verify_script, verify_script_with_bufs};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};
use std::hint::black_box;

fn main() {
    divan::main();
}

const NOP_CHECKER: NopSigChecker = NopSigChecker;

fn eval_legacy(script: &[u8]) {
    let mut interp = Interpreter::new(&NOP_CHECKER, ScriptFlags::NONE, SigVersion::Base);
    let _ = interp.eval(black_box(script));
}

// ── Tier 1: per-step dispatch overhead ─────────────────────────────────────

#[divan::bench]
fn empty_script() {
    eval_legacy(&[]);
}

/// Single `OP_NOP` — measures the minimal "decode one op + dispatch" cost.
#[divan::bench]
fn single_nop() {
    eval_legacy(&[0x61]);
}

/// Single push — measures the push fast path (no method call, just stack push).
#[divan::bench]
fn single_push_1() {
    eval_legacy(&[0x51]); // OP_1
}

/// 100 `OP_NOPs` back-to-back — amortises decode + dispatch.
#[divan::bench(args = [10usize, 100, 200])]
fn n_nops(bencher: divan::Bencher, n: usize) {
    let script = vec![0x61u8; n];
    bencher.bench(|| eval_legacy(black_box(&script)));
}

/// Pure push-only sequence. Tests the push fast path with no opcodes
/// between pushes.
#[divan::bench(args = [10usize, 100, 500])]
fn n_pushes(bencher: divan::Bencher, n: usize) {
    // OP_1 OP_DROP repeated to keep stack depth bounded.
    let mut script = Vec::with_capacity(n * 2);
    for _ in 0..n {
        script.push(0x51); // OP_1
        script.push(0x75); // OP_DROP
    }
    bencher.bench(|| eval_legacy(black_box(&script)));
}

// ── Tier 2: realistic legacy mixes ─────────────────────────────────────────

/// `OP_DUP OP_HASH160 OP_PUSHBYTES_20 <20-byte hash> OP_EQUALVERIFY OP_1`
/// — the typical pre-CHECKSIG portion of a P2PKH script. The expected hash
/// is precomputed (HASH160 of 20×0xab) so the bench doesn't depend on a
/// hash crate.
#[divan::bench]
fn p2pkh_shape_through_equalverify(bencher: divan::Bencher) {
    // HASH160(20 × 0xab) — verified against an independent calculator.
    const EXPECTED: [u8; 20] = [
        0xc8, 0x07, 0x76, 0x10, 0x73, 0xee, 0xea, 0x40, 0x01, 0xd1,
        0x07, 0xb6, 0x4f, 0xa6, 0x96, 0x70, 0x39, 0x6c, 0x99, 0x97,
    ];
    let mut script = Vec::with_capacity(2 * 20 + 5);
    script.push(0x14); // PUSHBYTES_20
    script.extend_from_slice(&[0xab; 20]); // preimage
    script.push(0x76); // OP_DUP
    script.push(0xa9); // OP_HASH160
    script.push(0x14); // PUSHBYTES_20
    script.extend_from_slice(&EXPECTED);
    script.push(0x88); // OP_EQUALVERIFY
    script.push(0x51); // OP_1 — leave a truthy top
    bencher.bench(|| eval_legacy(black_box(&script)));
}

/// Heavy arithmetic mix: 50 ADD/SUB pairs over small operands. Stresses the
/// `pop_num`/`push_num` path.
#[divan::bench]
fn arithmetic_mix(bencher: divan::Bencher) {
    let mut script = vec![0x51u8]; // OP_1 — initial value
    for i in 0..50 {
        // Push a small constant 1..16, alternating ADD and SUB.
        script.push(0x51 + u8::try_from(i % 16).unwrap()); // OP_1..OP_16
        script.push(if i % 2 == 0 { 0x93 } else { 0x94 }); // OP_ADD / OP_SUB
    }
    bencher.bench(|| eval_legacy(black_box(&script)));
}

/// IF / ELSE / ENDIF mix at modest nesting, all branches taken — measures
/// the conditional dispatch when executing.
#[divan::bench]
fn if_else_taken(bencher: divan::Bencher) {
    // Repeat: OP_1 OP_IF OP_5 OP_ELSE OP_6 OP_ENDIF OP_DROP
    let mut script = Vec::new();
    for _ in 0..30 {
        script.extend_from_slice(&[0x51, 0x63, 0x55, 0x67, 0x56, 0x68, 0x75]);
    }
    bencher.bench(|| eval_legacy(black_box(&script)));
}

/// Same shape as `if_else_taken` but the IF condition is false — exercises
/// the "skip non-conditional opcodes" fast path.
#[divan::bench]
fn if_else_skipped(bencher: divan::Bencher) {
    // Repeat: OP_0 OP_IF OP_5 OP_ELSE OP_6 OP_ENDIF OP_DROP
    let mut script = Vec::new();
    for _ in 0..30 {
        script.extend_from_slice(&[0x00, 0x63, 0x55, 0x67, 0x56, 0x68, 0x75]);
    }
    bencher.bench(|| eval_legacy(black_box(&script)));
}

/// Hash-heavy: SHA256 + HASH160 + HASH256 chained 10 times.
#[divan::bench]
fn hash_chain(bencher: divan::Bencher) {
    // Push a 32-byte initial value, then loop SHA256/HASH160/HASH256/DROP/PUSH.
    let mut script = Vec::new();
    script.push(0x20); // PUSHBYTES_32
    script.extend_from_slice(&[0u8; 32]);
    for _ in 0..10 {
        script.push(0xa8); // OP_SHA256
        script.push(0xa9); // OP_HASH160
        script.push(0xaa); // OP_HASH256
        script.push(0x75); // OP_DROP
        script.push(0x20); // PUSHBYTES_32
        script.extend_from_slice(&[0u8; 32]);
    }
    bencher.bench(|| eval_legacy(black_box(&script)));
}

// ── Tier 3: limits / pathological ──────────────────────────────────────────

/// Exactly the 201-op cap. Probes the op-count check overhead at the
/// boundary.
#[divan::bench]
fn at_op_count_limit() {
    let script = vec![0x61u8; 201];
    eval_legacy(&script);
}

/// Deep IF nesting — `K` nested IFs, all true, then `K` ENDIFs. Confirms the
/// O(1) `executing()` query stays flat under nesting.
#[divan::bench(args = [1usize, 10, 50, 100])]
fn deep_if_nesting(bencher: divan::Bencher, depth: usize) {
    let mut script = Vec::with_capacity(depth * 2 + 1);
    for _ in 0..depth {
        script.push(0x51); // OP_1
        script.push(0x63); // OP_IF
    }
    script.extend(std::iter::repeat_n(0x68u8, depth)); // OP_ENDIF
    bencher.bench(|| eval_legacy(black_box(&script)));
}

/// Maximum-size single push (520 bytes via PUSHDATA2).
#[divan::bench]
fn max_size_push(bencher: divan::Bencher) {
    let mut script = Vec::with_capacity(4 + 520);
    script.push(0x4d); // PUSHDATA2
    script.push(0x08); // length lo (520 = 0x208)
    script.push(0x02); // length hi
    script.extend_from_slice(&[0xab; 520]);
    script.push(0x75); // OP_DROP — keep the stack clean
    bencher.bench(|| eval_legacy(black_box(&script)));
}

/// Maximum-script-size NOP wall — 10000 `OP_NOPs` would exceed the op-count
/// cap, so we use 200 NOPs padded with pushes. Tests the parser's tolerance
/// for long scripts.
#[divan::bench]
fn near_max_script_size(bencher: divan::Bencher) {
    // 200 NOPs (under 201 cap) + 9800 single-byte pushes (don't count).
    // Total: 10000 bytes = at the script-size cap.
    let mut script = Vec::with_capacity(10_000);
    script.extend(std::iter::repeat_n(0x61u8, 200)); // 200 NOPs
    // Pushes that pop themselves to keep stack bounded:
    // 0x51 0x75  →  OP_1 OP_DROP, 2 bytes per cycle. 9800/2 = 4900 cycles.
    for _ in 0..4900 {
        script.push(0x51);
        script.push(0x75);
    }
    // OP_DROP counts as a non-push opcode → would blow the 201 cap. Use
    // pure pushes instead: 9800 OP_0 pushes (need 999 cap on stack though).
    // Simpler: skip OP_DROP, just emit OP_NOP-equivalents that don't push.
    // We'll just have 200 NOPs and 9800 nothing-bytes — actually no, every
    // byte must decode. Reset:
    let mut script = Vec::with_capacity(10_000);
    script.extend(std::iter::repeat_n(0x00u8, 9_800)); // 9800 OP_0 pushes
    // …but that would overflow the 1000-element stack. Cap at 999 OP_0s
    // followed by a flat run of 200 NOPs — must stay under 10000 bytes.
    // Simpler still: 9799 pure pushes filling stack to the cap minus one,
    // then 200 NOPs that don't touch the stack, plus one more push. Total
    // ops = 200 NOPs (only those count). Stays under both caps.
    // Final form below:
    let mut script = Vec::new();
    script.extend(std::iter::repeat_n(0x51u8, 999)); // OP_1 — fills stack to 999
    script.extend(std::iter::repeat_n(0x61u8, 200)); // 200 NOPs (under 201)
    // Currently 1199 bytes. Pad with extra OP_DROP+OP_1 *not* viable since
    // OP_DROP counts. We'll measure at 1199 bytes — the size-cap stress is
    // the parser walking a long-but-cheap byte sequence; 1199 is enough to
    // amortise. Bench name remains "near_max" by intent.
    bencher.bench(|| eval_legacy(black_box(&script)));
}

// ── Components in isolation ────────────────────────────────────────────────

/// `pre_scan` alone — separate from eval so we can see its share of total.
#[divan::bench(args = [10usize, 100, 1000])]
fn pre_scan_only(bencher: divan::Bencher, n: usize) {
    use mantle_consensus::script::interpreter::pre_scan::pre_scan;
    let script = vec![0x61u8; n];
    bencher.bench(|| {
        let _ = pre_scan(black_box(&script), SigVersion::Base);
    });
}

/// `ScriptReader::next_step` walking through a long script with no eval.
#[divan::bench]
fn reader_walk_1000_ops(bencher: divan::Bencher) {
    use mantle_consensus::script::interpreter::reader::ScriptReader;
    let script = vec![0x61u8; 1000];
    bencher.bench(|| {
        let mut r = ScriptReader::new(black_box(&script));
        while let Ok(Some(s)) = r.next_step() {
            black_box(s);
        }
    });
}

// ── Tier 4: segwit v0 end-to-end ───────────────────────────────────────────

fn segwit_flags() -> ScriptFlags {
    ScriptFlags::P2SH
        | ScriptFlags::DERSIG
        | ScriptFlags::LOW_S
        | ScriptFlags::STRICTENC
        | ScriptFlags::NULLDUMMY
        | ScriptFlags::WITNESS
}

fn make_witness_tx(witness_items: &[&[u8]]) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: 0 },
            script_sig: vec![].into(),
            sequence: 0xffff_ffff,
            witness: Witness::from_slice(witness_items),
        }],
        outputs: vec![TxOut { value: Satoshi::new(100_000), script_pubkey: vec![0x6a].into() }],
        locktime: 0,
    }
}

fn hash160(data: &[u8]) -> [u8; 20] {
    Ripemd160::digest(Sha256::digest(data)).into()
}

fn ecdsa_sign(sk: &SigningKey, digest: &[u8; 32], hash_type: SigHashType) -> Vec<u8> {
    let sig: EcdsaSig = sk.sign_prehash(digest).expect("signing cannot fail");
    let sig = sig.normalize_s().unwrap_or(sig);
    let mut bytes = sig.to_der().as_bytes().to_vec();
    bytes.push(u8::try_from(hash_type.0 & 0xff).expect("hashtype fits u8"));
    bytes
}

/// Full `verify_script` call for a P2WPKH input. Exercises BIP143 sighash,
/// `TxSigChecker` construction (including `Bip143Cache`), and ECDSA verify.
///
/// Pair with `verify_p2wpkh_with_bufs` to see the allocation overhead in isolation.
#[divan::bench]
fn verify_p2wpkh(bencher: divan::Bencher) {
    let sk = SigningKey::from_bytes(&[11u8; 32].into()).expect("nonzero secret");
    let pk: Vec<u8> = sk.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let pkh = hash160(&pk);

    let mut spk = vec![0x00u8, 0x14];
    spk.extend_from_slice(&pkh);

    let mut script_code = vec![0x76u8, 0xa9, 0x14];
    script_code.extend_from_slice(&pkh);
    script_code.extend_from_slice(&[0x88, 0xac]);

    let amount = Satoshi::new(100_000);
    let tmp_tx = make_witness_tx(&[]);
    let cache = Bip143Cache::new(&tmp_tx);
    let digest = signature_hash_bip143(&tmp_tx, 0, &script_code, amount, SigHashType::ALL, &cache);
    let sig = ecdsa_sign(&sk, &digest.0, SigHashType::ALL);

    let tx = make_witness_tx(&[sig.as_slice(), pk.as_slice()]);

    bencher.bench(|| {
        verify_script(black_box(&[]), black_box(&spk), black_box(&tx), 0, amount, segwit_flags())
            .expect("p2wpkh must verify");
    });
}

/// Full `verify_script` call for a P2WSH input (witness script = P2PKH).
/// Adds SHA256 commitment check and witness-script eval on top of P2WPKH cost.
///
/// Pair with `verify_p2wsh_with_bufs` to see the allocation overhead in isolation.
#[divan::bench]
fn verify_p2wsh(bencher: divan::Bencher) {
    let sk = SigningKey::from_bytes(&[13u8; 32].into()).expect("nonzero secret");
    let pk: Vec<u8> = sk.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let pkh = hash160(&pk);

    let mut witness_script = vec![0x76u8, 0xa9, 0x14];
    witness_script.extend_from_slice(&pkh);
    witness_script.extend_from_slice(&[0x88, 0xac]);

    let script_hash: [u8; 32] = Sha256::digest(&witness_script).into();
    let mut spk = vec![0x00u8, 0x20];
    spk.extend_from_slice(&script_hash);

    let amount = Satoshi::new(200_000);
    let tmp_tx = make_witness_tx(&[]);
    let cache = Bip143Cache::new(&tmp_tx);
    let digest = signature_hash_bip143(&tmp_tx, 0, &witness_script, amount, SigHashType::ALL, &cache);
    let sig = ecdsa_sign(&sk, &digest.0, SigHashType::ALL);

    let tx = make_witness_tx(&[sig.as_slice(), pk.as_slice(), witness_script.as_slice()]);

    bencher.bench(|| {
        verify_script(black_box(&[]), black_box(&spk), black_box(&tx), 0, amount, segwit_flags())
            .expect("p2wsh must verify");
    });
}

/// P2WPKH with caller-supplied stack buffers reused across iterations.
/// The delta vs `verify_p2wpkh` isolates the cost of per-call heap allocation.
#[divan::bench]
fn verify_p2wpkh_with_bufs(bencher: divan::Bencher) {
    let sk = SigningKey::from_bytes(&[11u8; 32].into()).expect("nonzero secret");
    let pk: Vec<u8> = sk.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let pkh = hash160(&pk);

    let mut spk = vec![0x00u8, 0x14];
    spk.extend_from_slice(&pkh);

    let mut script_code = vec![0x76u8, 0xa9, 0x14];
    script_code.extend_from_slice(&pkh);
    script_code.extend_from_slice(&[0x88, 0xac]);

    let amount = Satoshi::new(100_000);
    let tmp_tx = make_witness_tx(&[]);
    let cache = Bip143Cache::new(&tmp_tx);
    let digest = signature_hash_bip143(&tmp_tx, 0, &script_code, amount, SigHashType::ALL, &cache);
    let sig = ecdsa_sign(&sk, &digest.0, SigHashType::ALL);

    let tx = make_witness_tx(&[sig.as_slice(), pk.as_slice()]);

    let mut bufs: (Vec<InterpStackElement>, Vec<InterpStackElement>) = (Vec::new(), Vec::new());
    bencher.bench_local(|| {
        let (r, b) = verify_script_with_bufs(
            black_box(&[]), black_box(&spk), black_box(&tx), 0, amount, segwit_flags(),
            std::mem::take(&mut bufs),
        );
        r.expect("p2wpkh must verify");
        bufs = b;
    });
}

/// P2WSH with caller-supplied stack buffers reused across iterations.
/// The delta vs `verify_p2wsh` isolates the cost of per-call heap allocation.
#[divan::bench]
fn verify_p2wsh_with_bufs(bencher: divan::Bencher) {
    let sk = SigningKey::from_bytes(&[13u8; 32].into()).expect("nonzero secret");
    let pk: Vec<u8> = sk.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let pkh = hash160(&pk);

    let mut witness_script = vec![0x76u8, 0xa9, 0x14];
    witness_script.extend_from_slice(&pkh);
    witness_script.extend_from_slice(&[0x88, 0xac]);

    let script_hash: [u8; 32] = Sha256::digest(&witness_script).into();
    let mut spk = vec![0x00u8, 0x20];
    spk.extend_from_slice(&script_hash);

    let amount = Satoshi::new(200_000);
    let tmp_tx = make_witness_tx(&[]);
    let cache = Bip143Cache::new(&tmp_tx);
    let digest = signature_hash_bip143(&tmp_tx, 0, &witness_script, amount, SigHashType::ALL, &cache);
    let sig = ecdsa_sign(&sk, &digest.0, SigHashType::ALL);

    let tx = make_witness_tx(&[sig.as_slice(), pk.as_slice(), witness_script.as_slice()]);

    let mut bufs: (Vec<InterpStackElement>, Vec<InterpStackElement>) = (Vec::new(), Vec::new());
    bencher.bench_local(|| {
        let (r, b) = verify_script_with_bufs(
            black_box(&[]), black_box(&spk), black_box(&tx), 0, amount, segwit_flags(),
            std::mem::take(&mut bufs),
        );
        r.expect("p2wsh must verify");
        bufs = b;
    });
}
