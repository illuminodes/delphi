use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature as EcdsaSig, SigningKey};
use mantle::codec::Encode;
use mantle::primitives::{
    Block, BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, Transaction, TxHash,
    TxIn, TxOut, Witness,
};
use mantle_consensus::chain_view::{ChainView, Coin, MemoryChainView};
use mantle_consensus::merkle::{compute_merkle_root_reusing, merkle_root_owned};
use mantle_consensus::script::checker::TxSigChecker;
use mantle_consensus::script::consensus_crypto::Schnorr;
use mantle_consensus::script::sighash::{
    Bip143Cache, Bip341Cache, SigHashType, TapHasher, TaprootSigHashType, TxSighash,
};
use mantle_consensus::script::verify::{ScriptVerifier, verify_script};
use mantle_consensus::script::{
    InterpStackElement, Interpreter, NopSigChecker, ScriptFlags, SigVersion,
};
use mantle_consensus::validation::{validate_block_structure_reusing, validate_block_wire};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn encode<T: Encode>(val: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    val.encode(&mut buf).expect("Vec<u8> write cannot fail");
    buf
}

fn coinbase_tx(height_script: Vec<u8>, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256::ZERO,
                vout: u32::MAX,
            },
            script_sig: height_script.into(),
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

fn regular_tx(prev: Hash256, vout: u32, value: u64) -> Transaction {
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

fn build_chain(len: u32) -> MemoryChainView {
    let mut view = MemoryChainView::new();
    let mut prev = Hash256::ZERO;
    for i in 0..len {
        let h = BlockHeader {
            version: 4,
            prev_block: prev,
            merkle_root: Hash256::ZERO,
            time: 1_296_688_602 + i * 600,
            bits: CompactBits::new(0x207f_ffff),
            nonce: 0,
        };
        prev = h.hash();
        view.push_header(h);
    }
    view
}

fn mine_block(mut block: Block) -> Block {
    let target = block
        .header
        .bits
        .to_target()
        .expect("mine_block called with invalid nBits");
    for nonce in 0..=u32::MAX {
        block.header.nonce = nonce;
        let raw = *Hash256::hash(&block.header.to_bytes()).as_bytes();
        let mut be = raw;
        be.reverse();
        if be <= target {
            return block;
        }
    }
    panic!("no nonce found");
}

fn make_valid_block(txs: Vec<Transaction>) -> Block {
    let txids: Vec<Hash256> = txs
        .iter()
        .map(Transaction::txid)
        .collect::<Result<Vec<_>, _>>()
        .expect("txid encoding cannot fail");
    let merkle_root = merkle_root_owned(txids).expect("non-empty tx list always has a merkle root");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 1_231_006_505,
            bits: CompactBits::new(0x207f_ffff),
            nonce: 0,
        },
        transactions: txs,
    };
    mine_block(block)
}

#[allow(clippy::too_many_lines)] // sequence of scenario blocks; structure is the point
fn main() {
    let _profiler = dhat::Profiler::new_heap();

    // --- scenario 1: validate_block_structure on a 100-tx block (500 iters) ---
    {
        use std::collections::HashSet;
        let cb = coinbase_tx(vec![0x03, 0x64, 0x00, 0x00], 5_000_000_000);
        let mut txs = vec![cb];
        for i in 1..100u8 {
            txs.push(regular_tx(Hash256::from_bytes([i; 32]), 0, 1000));
        }
        let block = make_valid_block(txs);
        let mut serial_scratch = Vec::new();
        let mut txids = Vec::new();
        let mut merkle_buf = Vec::new();
        let mut seen_inputs = HashSet::new();
        let mut seen_txids = HashSet::new();
        for _ in 0..500 {
            validate_block_structure_reusing(
                &block,
                &mut serial_scratch,
                &mut txids,
                &mut merkle_buf,
                &mut seen_inputs,
                &mut seen_txids,
            )
            .expect("valid block must pass structure check");
        }
    }

    // --- scenario 2: encode + decode a 50-tx block (200 iters) ---
    {
        use mantle::codec::Decode;
        use std::io::Cursor;

        let cb = coinbase_tx(vec![0x03, 0x32, 0x00, 0x00], 5_000_000_000);
        let mut txs = vec![cb];
        for i in 1..50u8 {
            txs.push(regular_tx(Hash256::from_bytes([i; 32]), 0, 1000));
        }
        let block = make_valid_block(txs);
        let mut encode_buf: Vec<u8> = Vec::new();
        for _ in 0..200 {
            encode_buf.clear();
            block
                .encode(&mut encode_buf)
                .expect("Vec<u8> write cannot fail");
            let _ = Block::decode(&mut Cursor::new(encode_buf.as_slice()))
                .expect("valid block bytes must decode");
        }
    }

    // --- scenario 3: chain view build + tip lookups (1000 iters) ---
    {
        let view = build_chain(200);
        for i in 0..1000u32 {
            let _ = view
                .get_header_at(BlockHeight(i % 200))
                .expect("storage error");
            let tip = view.tip();
            let _ = view.get_header(&tip.hash).expect("storage error");
        }
    }

    // --- scenario 4: merkle root over 1000 txids (100 iters) ---
    {
        let template: Vec<Hash256> = (0..1000u32)
            .map(|i| Hash256::from_bytes([(i & 0xff) as u8; 32]))
            .collect();
        let mut buf: Vec<Hash256> = Vec::with_capacity(template.len());
        for _ in 0..100 {
            let _ = compute_merkle_root_reusing(&template, &mut buf)
                .expect("non-empty tx list must have merkle root");
        }
    }

    // --- scenario 5: MemoryChainView coin insert/lookup/spend (5000 iters) ---
    {
        let mut view = MemoryChainView::new();
        for i in 0..5000u32 {
            let op = OutPoint {
                txid: Hash256::hash(&i.to_le_bytes()),
                vout: 0,
            };
            view.insert_coin(
                op,
                Coin {
                    value: Satoshi::new(1000),
                    script_pubkey: vec![0x51].into(),
                    height: BlockHeight(i),
                    coinbase: false,
                },
            );
            let _ = view.get_coin(&op).expect("storage error");
            view.spend_coin(&op);
        }
    }

    // --- scenario 6: validate_block_wire on 100-tx block (500 iters, zero-copy path) ---
    {
        use std::collections::HashSet;
        let cb = coinbase_tx(vec![0x03, 0x64, 0x00, 0x00], 5_000_000_000);
        let mut txs = vec![cb];
        for i in 1..100u8 {
            txs.push(regular_tx(Hash256::from_bytes([i; 32]), 0, 1000));
        }
        let wire = encode(&make_valid_block(txs));
        let mut txids: Vec<Hash256> = Vec::new();
        let mut merkle_buf: Vec<Hash256> = Vec::new();
        let mut seen_inputs: HashSet<OutPoint> = HashSet::new();
        let mut seen_txids: HashSet<Hash256> = HashSet::new();
        for _ in 0..500 {
            validate_block_wire(
                &wire,
                &mut txids,
                &mut merkle_buf,
                &mut seen_inputs,
                &mut seen_txids,
            )
            .expect("valid block wire must pass structure check");
        }
    }

    // --- scenario 7: script Interpreter::eval on a NOP-heavy script (10000 iters) ---
    // Exercises the per-step dispatch hot path. The Interpreter is recreated
    // every iteration (matches real validation, one interpreter per input).
    // We expect to see one Stack::with_capacity(MAX_STACK_SIZE) allocation
    // per Interpreter — that's the ~6KB pre-allocation. dhat will tell us
    // whether anything else is sneaking through.
    {
        let checker = NopSigChecker;
        let script = vec![0x61u8; 200]; // 200 NOPs (under the 201 cap)
        for _ in 0..10_000 {
            let mut interp = Interpreter::new(&checker, ScriptFlags::NONE, SigVersion::Base);
            interp.eval(&script).expect("NOP-only script must succeed");
        }
    }

    // --- scenario 8: verify_script P2WPKH end-to-end (2000 iters) ---
    // Exercises the full segwit v0 path: Bip143Cache construction, BIP143
    // sighash, ECDSA verify, synthetic P2PKH scriptCode dispatch.
    // One TxSigChecker (and one Bip143Cache) is built per verify_script call,
    // matching real per-input validation.
    {
        let sk = SigningKey::from_bytes(&[11u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk.verifying_key().to_sec1_point(true).as_bytes().to_vec();
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
        let sig: EcdsaSig = sk.sign_prehash(digest.as_bytes()).expect("signing cannot fail");
        let sig = sig.normalize_s();
        let mut sig_bytes = sig.to_der().as_bytes().to_vec();
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

        for _ in 0..2_000 {
            verify_script(&[], &spk, &tx, 0, amount, flags).expect("p2wpkh must verify");
        }
    }

    // --- scenario 8b: verify_script_with_bufs P2WPKH (2000 iters) ---
    // Same workload as scenario 8 but with buffer reuse. After the first
    // iteration the two stack Vecs are already warm — subsequent calls should
    // incur zero heap allocation. dhat should show a flat allocation curve
    // here versus a linear one in scenario 8.
    {
        let sk = SigningKey::from_bytes(&[11u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk.verifying_key().to_sec1_point(true).as_bytes().to_vec();
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
        let sig: EcdsaSig = sk.sign_prehash(digest.as_bytes()).expect("signing cannot fail");
        let sig = sig.normalize_s();
        let mut sig_bytes = sig.to_der().as_bytes().to_vec();
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
        let verifier = ScriptVerifier::new(&tx, 0, amount, flags);
        for _ in 0..2_000 {
            let (result, returned) = verifier.verify_with_bufs(&[], &spk, bufs);
            result.expect("p2wpkh must verify");
            bufs = returned;
        }
    }

    // --- scenario 9: verify_script P2WSH end-to-end (1000 iters) ---
    // Exercises the P2WSH path: SHA256 commitment check, witness script eval,
    // clean-stack enforcement. The witness script is a P2PKH-inside-P2WSH.
    {
        let sk = SigningKey::from_bytes(&[13u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk.verifying_key().to_sec1_point(true).as_bytes().to_vec();
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
        let sig: EcdsaSig = sk.sign_prehash(digest.as_bytes()).expect("signing cannot fail");
        let sig = sig.normalize_s();
        let mut sig_bytes = sig.to_der().as_bytes().to_vec();
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

        for _ in 0..1_000 {
            verify_script(&[], &spk, &tx, 0, amount, flags).expect("p2wsh must verify");
        }
    }

    // --- scenario 9b: verify_script_with_bufs P2WSH (1000 iters) ---
    // Buffer-reuse counterpart to scenario 9.
    {
        let sk = SigningKey::from_bytes(&[13u8; 32].into()).expect("nonzero secret");
        let pk: Vec<u8> = sk.verifying_key().to_sec1_point(true).as_bytes().to_vec();
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
        let sig: EcdsaSig = sk.sign_prehash(digest.as_bytes()).expect("signing cannot fail");
        let sig = sig.normalize_s();
        let mut sig_bytes = sig.to_der().as_bytes().to_vec();
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
        for _ in 0..1_000 {
            let (result, returned) = verifier.verify_with_bufs(&[], &spk, bufs);
            result.expect("p2wsh must verify");
            bufs = returned;
        }
    }

    // --- scenario 11: verify_script taproot key-path (1000 iters, M3) ---
    // Exercises BIP341 sighash preimage, Bip341Cache construction, and Schnorr
    // verify via k256. One full `verify_script` call per iter — no buffer reuse.
    // dhat should show one Bip341Cache allocation per call (the sighash amounts
    // and script_pubkeys vecs) plus the Schnorr verifier internals.
    {
        use k256::schnorr::SigningKey as SchnorrSK;
        use k256::schnorr::signature::hazmat::PrehashSigner;

        let internal_secret = [0x55u8; 32];
        let internal_key: [u8; 32] = SchnorrSK::from_bytes((&internal_secret).into())
            .expect("nonzero")
            .verifying_key()
            .to_bytes()
            .into();
        let output_key = TapHasher::tweak_output_key(&internal_key, &[]).expect("valid key");

        // tweaked signing key
        let tweaked_secret: [u8; 32] = {
            use k256::elliptic_curve::bigint::Encoding;
            use k256::elliptic_curve::ops::Reduce;
            use k256::{NonZeroScalar, Scalar, U256};
            let sk = SchnorrSK::from_bytes((&internal_secret).into()).expect("nonzero");
            let mut tweak_input = [0u8; 32];
            tweak_input.copy_from_slice(&internal_key);
            let tweak_bytes = Schnorr::tagged_hash(b"TapTweak", &tweak_input);
            let t = <Scalar as Reduce<U256>>::reduce(&U256::from_be_bytes(tweak_bytes.into()));
            let d: Scalar = *sk.as_nonzero_scalar().as_ref();
            NonZeroScalar::new(d + t)
                .expect("nonzero")
                .to_bytes()
                .into()
        };

        let amount = Satoshi::new(100_000);
        let mut spk = vec![0x51u8, 0x20];
        spk.extend_from_slice(&output_key);

        // build a placeholder tx to compute the sighash, sign, then put the
        // real sig in the witness.
        let placeholder = Transaction {
            version: 2,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes([0x55; 32]),
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_fffd,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };
        let cache = Bip341Cache::new(&placeholder, &[amount], &[spk.as_slice()]);
        let msg = placeholder.bip341_sighash(
            0,
            amount,
            &spk,
            TaprootSigHashType::DEFAULT,
            &cache,
            None,
            None,
        );
        let sig64: [u8; 64] = {
            let sk = SchnorrSK::from_bytes((&tweaked_secret).into()).expect("nonzero");
            let sig: k256::schnorr::Signature = sk.sign_prehash(&msg).expect("sign");
            sig.to_bytes()
        };

        let tx = Transaction {
            version: 2,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes([0x55; 32]),
                    vout: 0,
                },
                script_sig: vec![].into(),
                sequence: 0xffff_fffd,
                witness: Witness::from_slice(&[sig64.as_slice()]),
            }],
            outputs: vec![TxOut {
                value: amount,
                script_pubkey: spk.clone().into(),
            }],
            locktime: 0,
        };

        let flags = ScriptFlags::P2SH | ScriptFlags::WITNESS | ScriptFlags::TAPROOT;
        for _ in 0..1_000 {
            verify_script(&[], &spk, &tx, 0, amount, flags).expect("taproot key-path must verify");
        }
    }

    // --- scenario 12: CLTV + CSV opcode throughput (10000 iters each, M4) ---
    // Measures allocation overhead of CLTV/CSV dispatch. These opcodes are pure
    // stack-peek + trait calls; any allocations here are interpreter overhead.
    {
        // CLTV: scriptPubKey = OP_PUSHBYTES_1 0x64 OP_CHECKLOCKTIMEVERIFY OP_DROP OP_1
        let cltv_spk: &[u8] = &[0x01, 0x64, 0xb1, 0x75, 0x51];
        let cltv_flags = ScriptFlags::P2SH | ScriptFlags::CHECKLOCKTIMEVERIFY;
        let cltv_tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes([0x33; 32]),
                    vout: 0,
                },
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
        for _ in 0..10_000 {
            let checker = TxSigChecker::new(&cltv_tx, 0, Satoshi::new(1000), cltv_flags);
            let mut interp = Interpreter::new(&checker, cltv_flags, SigVersion::Base);
            interp.eval(cltv_spk).expect("CLTV must pass");
        }

        // CSV: scriptPubKey = OP_PUSHBYTES_1 0x05 OP_CHECKSEQUENCEVERIFY OP_DROP OP_1
        let csv_spk: &[u8] = &[0x01, 0x05, 0xb2, 0x75, 0x51];
        let csv_flags = ScriptFlags::P2SH | ScriptFlags::CHECKSEQUENCEVERIFY;
        let csv_tx = Transaction {
            version: 2,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes([0x44; 32]),
                    vout: 0,
                },
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
        for _ in 0..10_000 {
            let checker = TxSigChecker::new(&csv_tx, 0, Satoshi::new(1000), csv_flags);
            let mut interp = Interpreter::new(&checker, csv_flags, SigVersion::Base);
            interp.eval(csv_spk).expect("CSV must pass");
        }
    }

    // --- scenario 10: script Interpreter::eval on a hash + arithmetic mix (5000 iters) ---
    // Real-shape script: P2PKH-ish (DUP HASH160 PUSHBYTES_20 <hash> EQUALVERIFY OP_1)
    // + a small arithmetic chain. Closest to typical mainnet pre-CHECKSIG work.
    // Allocation pressure here comes from each push (Vec::new() per stack
    // element) and each hash op (digest.to_vec() into a fresh allocation).
    {
        // HASH160(empty) = b472a266d0bd89c13706a4132ccfb16f7c3b9fcb.
        // Same vector covered by the script::interpreter::op_hash160_known_vector test,
        // so we can pin it as a const here without a hash-crate dep.
        const HASH160_EMPTY: [u8; 20] = [
            0xb4, 0x72, 0xa2, 0x66, 0xd0, 0xbd, 0x89, 0xc1, 0x37, 0x06, 0xa4, 0x13, 0x2c, 0xcf,
            0xb1, 0x6f, 0x7c, 0x3b, 0x9f, 0xcb,
        ];
        let checker = NopSigChecker;
        let mut script = Vec::with_capacity(64);
        script.push(0x00); // OP_0 (push empty)
        script.push(0x76); // OP_DUP
        script.push(0xa9); // OP_HASH160
        script.push(0x14);
        script.extend_from_slice(&HASH160_EMPTY); // PUSHBYTES_20 expected
        script.push(0x88); // OP_EQUALVERIFY
        // Arithmetic: 1 + 2 + 3 + 4 + 5 = 15
        for k in 0u8..5 {
            script.push(0x51 + k); // OP_(k+1)
            if k > 0 {
                script.push(0x93);
            } // OP_ADD
        }
        for _ in 0..5_000 {
            let mut interp = Interpreter::new(&checker, ScriptFlags::NONE, SigVersion::Base);
            interp.eval(&script).expect("script must succeed");
        }
    }
}
