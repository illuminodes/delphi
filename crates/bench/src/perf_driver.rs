/// Tight-loop perf driver — no allocator instrumentation, runs for ~5s
/// so `perf record` accumulates a statistically meaningful sample set.
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature as EcdsaSig, SigningKey};
use mantle::codec::Encode;
use mantle::hash::sha256d;
use mantle::primitives::{
    Block, BlockHeader, Hash256, OutPoint, Satoshi, Transaction, TxHash, TxIn, TxOut, Witness,
};
use mantle_consensus::merkle::compute_merkle_root;
use mantle_consensus::script::checker::TxSigChecker;
use mantle_consensus::script::consensus_crypto::Schnorr;
use mantle_consensus::script::sighash::{
    Bip143Cache, Bip341Cache, SigHashType, TapHasher, TaprootSigHashType, TxSighash,
};
use mantle_consensus::script::verify::{ScriptVerifier, verify_script};
use mantle_consensus::script::{
    InterpStackElement, Interpreter, NopSigChecker, ScriptFlags, SigVersion,
};
use mantle_consensus::validation::{validate_block_structure, validate_block_wire};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn encode<T: Encode>(v: &T) -> Vec<u8> {
    let mut b = Vec::new();
    v.encode(&mut b).expect("Vec<u8> write cannot fail");
    b
}

fn coinbase(script: Vec<u8>, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256::ZERO,
                vout: u32::MAX,
            },
            script_sig: script.into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: 0,
    }
}

fn regular(prev: Hash256, vout: u32, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: prev, vout },
            script_sig: vec![0x51; 20].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: 0,
    }
}

fn mine(mut b: Block) -> Block {
    let target = b
        .header
        .bits
        .to_target()
        .expect("mine called with invalid nBits");
    for n in 0..=u32::MAX {
        b.header.nonce = n;
        let mut h = sha256d(&b.header.to_bytes());
        h.reverse();
        if h <= target {
            return b;
        }
    }
    panic!("no nonce");
}

fn valid_block(txs: Vec<Transaction>) -> Block {
    let ids: Vec<Hash256> = txs
        .iter()
        .map(Transaction::txid)
        .collect::<Result<Vec<_>, _>>()
        .expect("txid encoding cannot fail");
    let root = compute_merkle_root(&ids).expect("non-empty tx list must have merkle root");
    let b = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: root,
            time: 1_231_006_505,
            bits: mantle::primitives::CompactBits(0x207f_ffff),
            nonce: 0,
        },
        transactions: txs,
    };
    mine(b)
}

fn run_for(dur: Duration, mut f: impl FnMut()) -> u64 {
    let deadline = Instant::now() + dur;
    let mut n = 0u64;
    while Instant::now() < deadline {
        f();
        n += 1;
    }
    n
}

#[allow(clippy::too_many_lines)]
fn main() {
    let secs = Duration::from_secs(2);

    // ── workload A: validate_block_structure, 100-tx block ───────────────────
    {
        let mut txs = vec![coinbase(vec![0x03, 0x64, 0x00, 0x00], 5_000_000_000)];
        for i in 1..100u8 {
            txs.push(regular(Hash256([i; 32]), 0, 1000));
        }
        let block = valid_block(txs);
        let n = run_for(secs, || {
            validate_block_structure(black_box(&block))
                .expect("valid block must pass structure check");
            black_box(());
        });
        eprintln!("validate_structure(100tx): {n} iters");
    }

    // ── workload B: txid over a 16-input transaction ─────────────────────────
    {
        let tx = Transaction {
            version: 1,
            inputs: (0..16u8)
                .map(|i| TxIn {
                    previous_output: OutPoint {
                        txid: Hash256([i; 32]),
                        vout: u32::from(i),
                    },
                    script_sig: vec![0x51; 20].into(),
                    sequence: u32::MAX,
                    witness: Witness::empty(),
                })
                .collect(),
            outputs: vec![TxOut {
                value: Satoshi::new(1000),
                script_pubkey: vec![0x51].into(),
            }],
            locktime: 0,
        };
        let n = run_for(secs, || {
            black_box(black_box(&tx).txid().expect("txid encoding cannot fail"));
        });
        eprintln!("txid(16-input tx): {n} iters");
    }

    // ── workload C: compute_merkle_root over 1000 txids ──────────────────────
    {
        let ids: Vec<Hash256> = (0..1000u32)
            .map(|i| Hash256(sha256d(&i.to_le_bytes())))
            .collect();
        let n = run_for(secs, || {
            black_box(compute_merkle_root(black_box(&ids)));
        });
        eprintln!("merkle_root(1000): {n} iters");
    }

    // ── workload D: block encode + decode roundtrip, 50 txs ──────────────────
    {
        use mantle::codec::Decode;
        use std::io::Cursor;
        let mut txs = vec![coinbase(vec![0x03, 0x32, 0x00, 0x00], 5_000_000_000)];
        for i in 1..50u8 {
            txs.push(regular(Hash256([i; 32]), 0, 1000));
        }
        let block = valid_block(txs);
        let bytes = encode(&block);
        let n = run_for(secs, || {
            black_box(
                Block::decode(&mut Cursor::new(black_box(bytes.as_slice())))
                    .expect("valid block bytes must decode"),
            );
        });
        eprintln!("block_decode(50tx): {n} iters");
    }

    // ── workload E: validate_block_wire, 100-tx block (zero-copy path) ───────
    {
        use std::collections::HashSet;
        let mut txs = vec![coinbase(vec![0x03, 0x64, 0x00, 0x00], 5_000_000_000)];
        for i in 1..100u8 {
            txs.push(regular(Hash256([i; 32]), 0, 1000));
        }
        let wire = encode(&valid_block(txs));
        let mut t: Vec<Hash256> = Vec::new();
        let mut mb: Vec<Hash256> = Vec::new();
        let mut si: HashSet<OutPoint> = HashSet::new();
        let mut st: HashSet<Hash256> = HashSet::new();
        let n = run_for(secs, || {
            validate_block_wire(black_box(&wire), &mut t, &mut mb, &mut si, &mut st)
                .expect("valid block wire must pass structure check");
            black_box(());
        });
        eprintln!("validate_wire(100tx):   {n} iters");
    }

    // ── workload G: verify_script P2WPKH end-to-end ──────────────────────────
    // Full segwit v0 path: Bip143Cache construction, BIP143 sighash, ECDSA
    // verify, synthetic P2PKH scriptCode dispatch. One TxSigChecker (and one
    // Bip143Cache) built per call, matching real per-input validation cost.
    {
        let sk = SigningKey::from_bytes(&[11u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let pkh: [u8; 20] = {
            let mid = Sha256::digest(&pk);
            Ripemd160::digest(mid).into()
        };
        let mut spk = vec![0x00u8, 0x14];
        spk.extend_from_slice(&pkh);

        let mut script_code = vec![0x76u8, 0xa9, 0x14];
        script_code.extend_from_slice(&pkh);
        script_code.extend_from_slice(&[0x88, 0xac]);

        let amount = Satoshi::new(100_000);
        let flags = ScriptFlags::P2SH
            | ScriptFlags::DERSIG
            | ScriptFlags::LOW_S
            | ScriptFlags::STRICTENC
            | ScriptFlags::NULLDUMMY
            | ScriptFlags::WITNESS;

        let tmp_tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };
        let cache = Bip143Cache::new(&tmp_tx);
        let digest = tmp_tx.bip143_sighash(0, &script_code, amount, SigHashType::ALL, &cache);
        let raw_sig: EcdsaSig = sk.sign_prehash(&digest.0).expect("signing cannot fail");
        let raw_sig = raw_sig.normalize_s().unwrap_or(raw_sig);
        let mut sig_bytes = raw_sig.to_der().as_bytes().to_vec();
        sig_bytes.push(u8::try_from(SigHashType::ALL.0 & 0xff).expect("hashtype fits u8"));

        let tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::from_slice(&[sig_bytes.as_slice(), pk.as_slice()]),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };

        let n = run_for(secs, || {
            verify_script(
                black_box(&[]),
                black_box(&spk),
                black_box(&tx),
                0,
                amount,
                flags,
            )
            .expect("p2wpkh must verify");
            black_box(());
        });
        eprintln!("verify_script(p2wpkh):         {n} iters");
    }

    // ── workload G-buf: verify_script_with_bufs P2WPKH ───────────────────────
    // Same as G but with buffer reuse — stack Vecs are allocated once and
    // threaded through every call. Measures the zero-alloc ceiling.
    {
        let sk = SigningKey::from_bytes(&[11u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let pkh: [u8; 20] = {
            let mid = Sha256::digest(&pk);
            Ripemd160::digest(mid).into()
        };
        let mut spk = vec![0x00u8, 0x14];
        spk.extend_from_slice(&pkh);

        let mut script_code = vec![0x76u8, 0xa9, 0x14];
        script_code.extend_from_slice(&pkh);
        script_code.extend_from_slice(&[0x88, 0xac]);

        let amount = Satoshi::new(100_000);
        let flags = ScriptFlags::P2SH
            | ScriptFlags::DERSIG
            | ScriptFlags::LOW_S
            | ScriptFlags::STRICTENC
            | ScriptFlags::NULLDUMMY
            | ScriptFlags::WITNESS;

        let tmp_tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };
        let cache = Bip143Cache::new(&tmp_tx);
        let digest = tmp_tx.bip143_sighash(0, &script_code, amount, SigHashType::ALL, &cache);
        let raw_sig: EcdsaSig = sk.sign_prehash(&digest.0).expect("signing cannot fail");
        let raw_sig = raw_sig.normalize_s().unwrap_or(raw_sig);
        let mut sig_bytes = raw_sig.to_der().as_bytes().to_vec();
        sig_bytes.push(u8::try_from(SigHashType::ALL.0 & 0xff).expect("hashtype fits u8"));

        let tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::from_slice(&[sig_bytes.as_slice(), pk.as_slice()]),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };

        let mut bufs: (Vec<InterpStackElement>, Vec<InterpStackElement>) = (Vec::new(), Vec::new());
        // warm up the bufs outside the timed loop
        let verifier = ScriptVerifier::new(&tx, 0, amount, flags);
        let (r, b) = verifier.verify_with_bufs(&[], &spk, bufs);
        r.expect("p2wpkh must verify");
        bufs = b;

        let mut n = 0u64;
        let deadline = std::time::Instant::now() + secs;
        while std::time::Instant::now() < deadline {
            let (r, b) = verifier.verify_with_bufs(black_box(&[]), black_box(&spk), bufs);
            r.expect("p2wpkh must verify");
            bufs = b;
            black_box(());
            n += 1;
        }
        eprintln!("verify_script_bufs(p2wpkh):    {n} iters");
    }

    // ── workload H: verify_script P2WSH end-to-end ───────────────────────────
    // P2WSH path: SHA256 commitment check, witness script eval (P2PKH inside
    // P2WSH), clean-stack enforcement, BIP143 sighash.
    {
        let sk = SigningKey::from_bytes(&[13u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let pkh: [u8; 20] = {
            let mid = Sha256::digest(&pk);
            Ripemd160::digest(mid).into()
        };
        let mut witness_script = vec![0x76u8, 0xa9, 0x14];
        witness_script.extend_from_slice(&pkh);
        witness_script.extend_from_slice(&[0x88, 0xac]);

        let script_hash: [u8; 32] = Sha256::digest(&witness_script).into();
        let mut spk = vec![0x00u8, 0x20];
        spk.extend_from_slice(&script_hash);

        let amount = Satoshi::new(200_000);
        let flags = ScriptFlags::P2SH
            | ScriptFlags::DERSIG
            | ScriptFlags::LOW_S
            | ScriptFlags::STRICTENC
            | ScriptFlags::NULLDUMMY
            | ScriptFlags::WITNESS;

        let tmp_tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };
        let cache = Bip143Cache::new(&tmp_tx);
        let digest = tmp_tx.bip143_sighash(0, &witness_script, amount, SigHashType::ALL, &cache);
        let raw_sig: EcdsaSig = sk.sign_prehash(&digest.0).expect("signing cannot fail");
        let raw_sig = raw_sig.normalize_s().unwrap_or(raw_sig);
        let mut sig_bytes = raw_sig.to_der().as_bytes().to_vec();
        sig_bytes.push(u8::try_from(SigHashType::ALL.0 & 0xff).expect("hashtype fits u8"));

        let tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::from_slice(&[
                    sig_bytes.as_slice(),
                    pk.as_slice(),
                    witness_script.as_slice(),
                ]),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };

        let n = run_for(secs, || {
            verify_script(
                black_box(&[]),
                black_box(&spk),
                black_box(&tx),
                0,
                amount,
                flags,
            )
            .expect("p2wsh must verify");
            black_box(());
        });
        eprintln!("verify_script(p2wsh):          {n} iters");
    }

    // ── workload H-buf: verify_script_with_bufs P2WSH ────────────────────────
    // Same as H but with buffer reuse.
    {
        let sk = SigningKey::from_bytes(&[13u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let pkh: [u8; 20] = {
            let mid = Sha256::digest(&pk);
            Ripemd160::digest(mid).into()
        };
        let mut witness_script = vec![0x76u8, 0xa9, 0x14];
        witness_script.extend_from_slice(&pkh);
        witness_script.extend_from_slice(&[0x88, 0xac]);

        let script_hash: [u8; 32] = Sha256::digest(&witness_script).into();
        let mut spk = vec![0x00u8, 0x20];
        spk.extend_from_slice(&script_hash);

        let amount = Satoshi::new(200_000);
        let flags = ScriptFlags::P2SH
            | ScriptFlags::DERSIG
            | ScriptFlags::LOW_S
            | ScriptFlags::STRICTENC
            | ScriptFlags::NULLDUMMY
            | ScriptFlags::WITNESS;

        let tmp_tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };
        let cache = Bip143Cache::new(&tmp_tx);
        let digest = tmp_tx.bip143_sighash(0, &witness_script, amount, SigHashType::ALL, &cache);
        let raw_sig: EcdsaSig = sk.sign_prehash(&digest.0).expect("signing cannot fail");
        let raw_sig = raw_sig.normalize_s().unwrap_or(raw_sig);
        let mut sig_bytes = raw_sig.to_der().as_bytes().to_vec();
        sig_bytes.push(u8::try_from(SigHashType::ALL.0 & 0xff).expect("hashtype fits u8"));

        let tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_ffff,
                witness: Witness::from_slice(&[
                    sig_bytes.as_slice(),
                    pk.as_slice(),
                    witness_script.as_slice(),
                ]),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };

        let mut bufs: (Vec<InterpStackElement>, Vec<InterpStackElement>) = (Vec::new(), Vec::new());
        let verifier = ScriptVerifier::new(&tx, 0, amount, flags);
        let (r, b) = verifier.verify_with_bufs(&[], &spk, bufs);
        r.expect("p2wsh must verify");
        bufs = b;

        let mut n = 0u64;
        let deadline = std::time::Instant::now() + secs;
        while std::time::Instant::now() < deadline {
            let (r, b) = verifier.verify_with_bufs(black_box(&[]), black_box(&spk), bufs);
            r.expect("p2wsh must verify");
            bufs = b;
            black_box(());
            n += 1;
        }
        eprintln!("verify_script_bufs(p2wsh):     {n} iters");
    }

    // ── workload I: verify_script taproot key-path (M3) ─────────────────────
    // BIP341 sighash + Bip341Cache construction + Schnorr verify. The costliest
    // single-input path in post-taproot validation.
    {
        use k256::schnorr::SigningKey as SchnorrSK;
        use k256::schnorr::signature::hazmat::PrehashSigner;

        let internal_secret = [0x55u8; 32];
        let internal_key: [u8; 32] = SchnorrSK::from_bytes(&internal_secret)
            .expect("nonzero")
            .verifying_key()
            .to_bytes()
            .into();
        let output_key = TapHasher::tweak_output_key(&internal_key, &[]).expect("valid key");

        let tweaked_secret: [u8; 32] = {
            use k256::elliptic_curve::ops::Reduce;
            use k256::{NonZeroScalar, Scalar, U256};
            let sk = SchnorrSK::from_bytes(&internal_secret).expect("nonzero");
            let tweak_bytes = Schnorr::tagged_hash(b"TapTweak", &internal_key);
            let t = <Scalar as Reduce<U256>>::reduce_bytes((&tweak_bytes).into());
            let d: Scalar = *sk.as_nonzero_scalar().as_ref();
            NonZeroScalar::new(d + t).expect("nonzero").to_bytes().into()
        };

        let amount = Satoshi::new(100_000);
        let mut spk = vec![0x51u8, 0x20];
        spk.extend_from_slice(&output_key);

        let placeholder = Transaction {
            version: 2,
            inputs: vec![TxIn {
                previous_output: OutPoint { txid: Hash256([0x55; 32]), vout: 0 },
                script_sig: vec![].into(),
                sequence: 0xffff_fffd,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut { value: amount, script_pubkey: spk.clone().into() }],
            locktime: 0,
        };
        let cache = Bip341Cache::new(&placeholder, &[amount], &[spk.as_slice()]);
        let msg = placeholder.bip341_sighash(
            0, amount, &spk, TaprootSigHashType::DEFAULT, &cache, None, None,
        );
        let sig64: [u8; 64] = {
            let sk = SchnorrSK::from_bytes(&tweaked_secret).expect("nonzero");
            let sig: k256::schnorr::Signature = sk.sign_prehash(&msg).expect("sign");
            sig.to_bytes()
        };

        let tx = Transaction {
            version: 2,
            inputs: vec![TxIn {
                previous_output: OutPoint { txid: Hash256([0x55; 32]), vout: 0 },
                script_sig: vec![].into(),
                sequence: 0xffff_fffd,
                witness: Witness::from_slice(&[sig64.as_slice()]),
            }],
            outputs: vec![TxOut { value: amount, script_pubkey: spk.clone().into() }],
            locktime: 0,
        };

        let flags = ScriptFlags::P2SH | ScriptFlags::WITNESS | ScriptFlags::TAPROOT;
        let n = run_for(secs, || {
            verify_script(black_box(&[]), black_box(&spk), black_box(&tx), 0, amount, flags)
                .expect("taproot key-path must verify");
            black_box(());
        });
        eprintln!("verify_script(taproot-keypath): {n} iters");
    }

    // ── workload J: CLTV + CSV opcode throughput (M4) ────────────────────────
    // Each iteration creates a fresh Interpreter (one per input in real
    // validation) and runs the opcode. Isolates interpreter startup vs opcode
    // dispatch cost relative to workload F.
    {
        // CLTV: scriptPubKey = OP_PUSHBYTES_1 0x64 OP_CHECKLOCKTIMEVERIFY OP_DROP OP_1
        let cltv_spk: &[u8] = &[0x01, 0x64, 0xb1, 0x75, 0x51];
        let cltv_flags = ScriptFlags::P2SH | ScriptFlags::CHECKLOCKTIMEVERIFY;
        let cltv_tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint { txid: Hash256([0x33; 32]), vout: 0 },
                script_sig: vec![].into(),
                sequence: 0xffff_fffe,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: Satoshi::new(1000),
                script_pubkey: vec![0x51].into(),
            }],
            locktime: 200,
        };
        let cltv_checker = TxSigChecker::new(&cltv_tx, 0, Satoshi::new(1000), cltv_flags);
        let n = run_for(secs, || {
            let mut interp =
                Interpreter::new(&cltv_checker, black_box(cltv_flags), SigVersion::Base);
            interp.eval(black_box(cltv_spk)).expect("CLTV must pass");
            black_box(());
        });
        eprintln!("cltv_throughput:               {n} iters");

        // CSV: scriptPubKey = OP_PUSHBYTES_1 0x05 OP_CHECKSEQUENCEVERIFY OP_DROP OP_1
        let csv_spk: &[u8] = &[0x01, 0x05, 0xb2, 0x75, 0x51];
        let csv_flags = ScriptFlags::P2SH | ScriptFlags::CHECKSEQUENCEVERIFY;
        let csv_tx = Transaction {
            version: 2,
            inputs: vec![TxIn {
                previous_output: OutPoint { txid: Hash256([0x44; 32]), vout: 0 },
                script_sig: vec![].into(),
                sequence: 10,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: Satoshi::new(1000),
                script_pubkey: vec![0x51].into(),
            }],
            locktime: 0,
        };
        let csv_checker = TxSigChecker::new(&csv_tx, 0, Satoshi::new(1000), csv_flags);
        let n = run_for(secs, || {
            let mut interp =
                Interpreter::new(&csv_checker, black_box(csv_flags), SigVersion::Base);
            interp.eval(black_box(csv_spk)).expect("CSV must pass");
            black_box(());
        });
        eprintln!("csv_throughput:                {n} iters");
    }

    // ── workload F: script Interpreter::eval, P2PKH-shape mix ────────────────
    // P2PKH-ish script (OP_0 OP_DUP OP_HASH160 PUSHBYTES_20 <hash> OP_EQUALVERIFY)
    // followed by an arithmetic chain. Exercises every non-crypto opcode group:
    // push, dup, hash, equality, arithmetic. The Interpreter is recreated per
    // iteration to match real validation (one interpreter per input).
    {
        // HASH160(empty) — same vector as op_hash160_known_vector unit test.
        const HASH160_EMPTY: [u8; 20] = [
            0xb4, 0x72, 0xa2, 0x66, 0xd0, 0xbd, 0x89, 0xc1, 0x37, 0x06, 0xa4, 0x13, 0x2c, 0xcf,
            0xb1, 0x6f, 0x7c, 0x3b, 0x9f, 0xcb,
        ];
        let checker = NopSigChecker;
        let mut script = Vec::with_capacity(64);
        script.push(0x00); // OP_0
        script.push(0x76); // OP_DUP
        script.push(0xa9); // OP_HASH160
        script.push(0x14);
        script.extend_from_slice(&HASH160_EMPTY); // PUSHBYTES_20 expected
        script.push(0x88); // OP_EQUALVERIFY
        for k in 0u8..5 {
            script.push(0x51 + k);
            if k > 0 {
                script.push(0x93);
            } // OP_ADD
        }
        let n = run_for(secs, || {
            let mut interp = Interpreter::new(&checker, ScriptFlags::NONE, SigVersion::Base);
            interp
                .eval(black_box(&script))
                .expect("script must succeed");
            black_box(());
        });
        eprintln!("script_eval(p2pkh-shape): {n} iters");
    }
}
