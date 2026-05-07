/// Layer 4: BIP test vectors and static regression fixtures.
///
/// These are static, hand-encoded constants derived from known-good Bitcoin
/// data. No runtime file I/O; everything compiles into the test binary.
use mantle::codec::{Decode, Encode};
use mantle::hash::sha256d;
use mantle::primitives::{Block, BlockHeader, BlockHeight, Hash256, OutPoint, Transaction, TxHash, TxIn, TxOut, Witness};
use std::io::Cursor;

use crate::chain_view::{ChainView, Coin, MemoryChainView};
use crate::error::ValidationError;
use crate::genesis::{GENESIS_BLOCK_HASH, GENESIS_BLOCK_HEX};
use crate::merkle::compute_merkle_root;
use crate::test_helpers::{decode_hex, hash_to_hex};
use crate::validation::{
    validate_block_context, validate_block_structure, BIP34_HEIGHT,
    MAX_COINBASE_SCRIPTSIG_LEN, MAX_MONEY, MIN_COINBASE_SCRIPTSIG_LEN,
};

fn hash_to_hex_display(hash: Hash256) -> String {
    hash_to_hex(hash.0)
}

/// Build the minimal null-outpoint input (coinbase marker).
fn coinbase_input(script_sig: Vec<u8>) -> TxIn {
    TxIn {
        previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
        script_sig: script_sig.into(),
        sequence: u32::MAX,
        witness: Witness::empty(),
    }
}


fn make_output(value: u64) -> TxOut {
    TxOut { value: mantle::primitives::Satoshi::new(value), script_pubkey: vec![0x51].into() } // OP_1 (trivial P2PK stand-in)
}

/// Build a minimal valid block whose Stage 1 checks pass.
///
/// The caller supplies the coinbase scriptSig. nBits is set to the easiest
/// possible target (0x207fffff) so proof-of-work is trivially satisfied.
fn minimal_block(coinbase_script_sig: Vec<u8>, transactions: Vec<Transaction>) -> Block {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input(coinbase_script_sig)],
        outputs: vec![make_output(50_0000_0000)],
        locktime: 0,
    };

    let mut txs = vec![coinbase];
    txs.extend(transactions);

    let txids: Vec<Hash256> = txs.iter()
        .map(Transaction::txid)
        .collect::<Result<Vec<_>, _>>()
        .expect("txid encoding cannot fail");
    let merkle_root = compute_merkle_root(&txids).expect("non-empty tx list must have merkle root");

    Block {
        header: BlockHeader {
            version: 4,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 1_231_006_505,
            bits: mantle::primitives::CompactBits(0x207f_ffff), // regtest easiest target
            nonce: 0,
        },
        transactions: txs,
    }
}

// ── Build a MemoryChainView with N sequential headers ─────────────────────────

fn build_chain(n: u32, bits: mantle::primitives::CompactBits, base_time: u32) -> MemoryChainView {
    let mut view = MemoryChainView::new();
    let mut prev = Hash256::ZERO;
    for i in 0..n {
        let h = BlockHeader {
            version: 4,
            prev_block: prev,
            merkle_root: Hash256::ZERO,
            time: base_time + i * 600,
            bits,
            nonce: 0,
        };
        prev = h.hash();
        view.push_header(h);
    }
    view
}

/// Build a chain whose tip is at `tip_height` by only storing the last 12
/// headers. Validation only needs the parent + 11 ancestors for MTP.
fn build_chain_at(tip_height: BlockHeight, bits: mantle::primitives::CompactBits, base_time: u32) -> MemoryChainView {
    let window = 12u32;
    let start = tip_height.0.saturating_sub(window - 1);
    let mut view = MemoryChainView::with_height_base(BlockHeight(start));
    let mut prev = Hash256::ZERO;
    for i in 0..window {
        let h = BlockHeader {
            version: 4,
            prev_block: prev,
            merkle_root: Hash256::ZERO,
            time: base_time + (start + i) * 600,
            bits,
            nonce: 0,
        };
        prev = h.hash();
        view.push_header(h);
    }
    view
}

// ── Genesis block BIP vectors ─────────────────────────────────────────────────

#[test]
fn genesis_decode_and_stage1_pass() {
    let bytes = decode_hex(GENESIS_BLOCK_HEX);
    let block = Block::decode(&mut Cursor::new(&bytes)).expect("genesis block must decode");

    // The genesis block predates BIP34, so validate_block_structure should pass.
    // The nBits is 0x1d00ffff; genesis hash is known to satisfy that target.
    validate_block_structure(&block).expect("genesis block must pass Stage 1");
}

#[test]
fn genesis_hash_matches_known_constant() {
    let bytes = decode_hex(GENESIS_BLOCK_HEX);
    let raw = sha256d(&bytes[..BlockHeader::ENCODED_LEN]);
    assert_eq!(hash_to_hex_display(Hash256(raw)), GENESIS_BLOCK_HASH);
}

#[test]
fn genesis_txid_matches_merkle_root() {
    let bytes = decode_hex(GENESIS_BLOCK_HEX);
    let block = Block::decode(&mut Cursor::new(&bytes)).expect("genesis block must decode");

    let mut tx_bytes = Vec::new();
    block.transactions[0].encode(&mut tx_bytes).expect("Vec<u8> write cannot fail");
    let computed_txid = Hash256(sha256d(&tx_bytes));

    // Genesis has one tx — merkle root equals the txid (no pair hashing).
    assert_eq!(block.header.merkle_root, computed_txid);
}

// ── Stage 1: structural validation vectors ────────────────────────────────────

#[test]
fn stage1_empty_block_rejected() {
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 0,
            bits: mantle::primitives::CompactBits(0x207f_ffff),
            nonce: 0,
        },
        transactions: vec![],
    };
    assert_eq!(validate_block_structure(&block), Err(ValidationError::EmptyBlock));
}

#[test]
fn stage1_first_tx_not_coinbase_rejected() {
    let non_cb = Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256([0xab; 32]), vout: 0 },
            script_sig: vec![].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![make_output(1000)],
        locktime: 0,
    };
    let txids = vec![non_cb.txid().expect("txid encoding cannot fail")];
    let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 0,
            bits: mantle::primitives::CompactBits(0x207f_ffff),
            nonce: 0,
        },
        transactions: vec![non_cb],
    };
    assert_eq!(validate_block_structure(&block), Err(ValidationError::FirstTxNotCoinbase));
}

#[test]
fn stage1_non_coinbase_with_null_outpoint_rejected() {
    // Second tx uses the coinbase null outpoint — must be rejected.
    let cb_script = {
        let mut s = Transaction::bip34_height_script(BlockHeight::ZERO);
        s.extend_from_slice(&[0x00]); // pad to meet min length
        s
    };
    let fake_cb = Transaction {
        version: 1,
        inputs: vec![coinbase_input(vec![0x51, 0x52])], // valid coinbase
        outputs: vec![make_output(50_0000_0000)],
        locktime: 0,
    };
    let imposter = Transaction {
        version: 1,
        inputs: vec![coinbase_input(vec![0x00, 0x00])], // null outpoint in tx[1]
        outputs: vec![make_output(1000)],
        locktime: 0,
    };
    let _ = cb_script;
    let txids = vec![
        fake_cb.txid().expect("txid encoding cannot fail"),
        imposter.txid().expect("txid encoding cannot fail"),
    ];
    let merkle_root = compute_merkle_root(&txids).expect("two-tx merkle root must exist");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 0,
            bits: mantle::primitives::CompactBits(0x207f_ffff),
            nonce: 0,
        },
        transactions: vec![fake_cb, imposter],
    };
    assert_eq!(
        validate_block_structure(&block),
        Err(ValidationError::UnexpectedCoinbaseInput { tx_index: 1 }),
    );
}

#[test]
fn stage1_coinbase_script_too_short() {
    let block = minimal_block(vec![0x00], vec![]); // 1 byte — below minimum of 2
    assert_eq!(
        validate_block_structure(&block),
        Err(ValidationError::CoinbaseScriptSigLength { len: 1 }),
    );
}

#[test]
fn stage1_coinbase_script_too_long() {
    let long_script = vec![0x00; MAX_COINBASE_SCRIPTSIG_LEN + 1];
    let block = minimal_block(long_script, vec![]);
    assert_eq!(
        validate_block_structure(&block),
        Err(ValidationError::CoinbaseScriptSigLength { len: MAX_COINBASE_SCRIPTSIG_LEN + 1 }),
    );
}

#[test]
fn stage1_coinbase_script_boundary_values_accepted() {
    // We only care that the coinbase script-length check doesn't reject these.
    // PoW with nonce=0 may or may not pass (hash is ~uniform), so we accept
    // any error that is NOT CoinbaseScriptSigLength.
    for &len in &[MIN_COINBASE_SCRIPTSIG_LEN, MAX_COINBASE_SCRIPTSIG_LEN] {
        let script = vec![0x51; len];
        match validate_block_structure(&minimal_block(script, vec![])) {
            Ok(()) => {}
            Err(ValidationError::CoinbaseScriptSigLength { len: actual }) => {
                panic!("coinbase script of {len} bytes was rejected as bad length (got {actual})");
            }
            Err(_other) => {} // PoW or other structural error — script length was accepted
        }
    }
}

#[test]
fn stage1_output_value_over_max_money_rejected() {
    let too_much = mantle::primitives::Satoshi::new(MAX_MONEY.to_u64() + 1);
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input(vec![0x51, 0x52])],
        outputs: vec![TxOut { value: too_much, script_pubkey: vec![0x51].into() }],
        locktime: 0,
    };
    let txids = vec![coinbase.txid().expect("txid encoding cannot fail")];
    let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 0,
            bits: mantle::primitives::CompactBits(0x207f_ffff),
            nonce: 0,
        },
        transactions: vec![coinbase],
    };
    assert_eq!(
        validate_block_structure(&block),
        Err(ValidationError::OutputValueOutOfRange { tx_index: 0, output_index: 0 }),
    );
}

#[test]
fn stage1_duplicate_inputs_rejected() {
    let outpoint = OutPoint { txid: Hash256([0xab; 32]), vout: 1 };
    let tx = Transaction {
        version: 1,
        inputs: vec![
            TxIn { previous_output: outpoint, script_sig: vec![].into(), sequence: u32::MAX, witness: Witness::empty() },
            TxIn { previous_output: outpoint, script_sig: vec![].into(), sequence: u32::MAX, witness: Witness::empty() }, // dup
        ],
        outputs: vec![make_output(1000)],
        locktime: 0,
    };
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input(vec![0x51, 0x52])],
        outputs: vec![make_output(50_0000_0000)],
        locktime: 0,
    };
    let txids = vec![
        coinbase.txid().expect("txid encoding cannot fail"),
        tx.txid().expect("txid encoding cannot fail"),
    ];
    let merkle_root = compute_merkle_root(&txids).expect("two-tx merkle root must exist");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 0,
            bits: mantle::primitives::CompactBits(0x207f_ffff),
            nonce: 0,
        },
        transactions: vec![coinbase, tx],
    };
    assert_eq!(
        validate_block_structure(&block),
        Err(ValidationError::DuplicateInput { tx_index: 1 }),
    );
}

#[test]
fn stage1_merkle_mismatch_rejected() {
    let mut block = minimal_block(vec![0x51, 0x52], vec![]);
    // Corrupt the merkle root.
    block.header.merkle_root = Hash256([0xff; 32]);
    assert_eq!(validate_block_structure(&block), Err(ValidationError::MerkleRootMismatch));
}

#[test]
fn stage1_invalid_bits_rejected() {
    let mut block = minimal_block(vec![0x51, 0x52], vec![]);
    block.header.bits = mantle::primitives::CompactBits(0); // zero nBits is always invalid
    // Also recompute merkle (header bits don't affect merkle root — this is fine).
    assert_eq!(validate_block_structure(&block), Err(ValidationError::InvalidBits));
}

#[test]
fn stage1_insufficient_pow_rejected() {
    // Use a very hard target (0x03000001 ≈ 0x000001 × 2^(3-3)*8 — basically
    // requires the hash to be almost zero). The block header nonce is 0 so
    // the hash is effectively random and almost certainly fails this target.
    let mut block = minimal_block(vec![0x51, 0x52], vec![]);
    block.header.bits = mantle::primitives::CompactBits(0x1900_0001); // very hard target
    // Recompute the hash check — the actual hash won't satisfy target 0x0001…
    // Note: we don't update merkle root for the bits field (it only appears in
    // the header, not in tx data). So we need to rebuild:
    let txids: Vec<Hash256> = block.transactions.iter()
        .map(Transaction::txid)
        .collect::<Result<Vec<_>, _>>()
        .expect("txid encoding cannot fail");
    let merkle_root = compute_merkle_root(&txids).expect("non-empty tx list must have merkle root");
    block.header.merkle_root = merkle_root;
    assert_eq!(
        validate_block_structure(&block),
        Err(ValidationError::InsufficientProofOfWork),
    );
}

// ── Stage 2: contextual validation vectors ────────────────────────────────────

/// Build a structurally valid block whose coinbase scriptSig starts with the
/// BIP34 height encoding for `this_height`.
fn stage2_block_for_height(
    view: &MemoryChainView,
    this_height: BlockHeight,
    version: i32,
) -> Block {
    let tip = view.tip();
    let parent_bits = view.get_header_at(tip.height)
        .expect("storage error").expect("tip header must exist").bits;
    let script_sig = {
        let mut s = Transaction::bip34_height_script(this_height);
        // Ensure minimum length of 2 even if bip34_script produced 1 byte.
        if s.len() < MIN_COINBASE_SCRIPTSIG_LEN {
            s.push(0x00);
        }
        s
    };
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input(script_sig)],
        outputs: vec![make_output(50_0000_0000)],
        locktime: 0,
    };
    let txids = vec![coinbase.txid().expect("txid encoding cannot fail")];
    let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");

    // MTP = median of up to 11 preceding blocks. We need time > MTP.
    // We pick a timestamp 600 seconds after the tip's timestamp.
    let parent_time = view.get_header_at(tip.height)
        .expect("storage error").expect("tip header must exist").time;
    let time = parent_time + 600;

    Block {
        header: BlockHeader {
            version,
            prev_block: tip.hash,
            merkle_root,
            time,
            bits: parent_bits, // same difficulty (non-retarget height)
            nonce: 0,
        },
        transactions: vec![coinbase],
    }
}

#[test]
fn stage2_unknown_parent_rejected() {
    let view = build_chain(10, mantle::primitives::CompactBits(0x207f_ffff), 1_000_000);
    let mut block = stage2_block_for_height(&view, BlockHeight(10), 4);
    // Point to an unknown parent.
    block.header.prev_block = Hash256([0xde; 32]);
    assert_eq!(
        validate_block_context(&block, &view),
        Err(ValidationError::UnknownParent),
    );
}

#[test]
fn stage2_unexpected_difficulty_rejected() {
    let view = build_chain(10, mantle::primitives::CompactBits(0x207f_ffff), 1_000_000);
    let mut block = stage2_block_for_height(&view, BlockHeight(10), 4);
    // Change the nBits to something different from the parent (non-retarget block).
    block.header.bits = mantle::primitives::CompactBits(0x1d00_ffff);
    assert_eq!(
        validate_block_context(&block, &view),
        Err(ValidationError::UnexpectedDifficulty),
    );
}

#[test]
fn stage2_timestamp_not_greater_than_mtp_rejected() {
    let view = build_chain(12, mantle::primitives::CompactBits(0x207f_ffff), 1_000_000);
    let tip = view.tip();
    let parent_bits = view.get_header_at(tip.height)
        .expect("storage error").expect("tip header must exist").bits;
    let this_height = tip.height + 1;
    let script_sig = {
        let mut s = Transaction::bip34_height_script(this_height);
        if s.len() < MIN_COINBASE_SCRIPTSIG_LEN {
            s.push(0x00);
        }
        s
    };
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input(script_sig)],
        outputs: vec![make_output(50_0000_0000)],
        locktime: 0,
    };
    let txids = vec![coinbase.txid().expect("txid encoding cannot fail")];
    let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");

    // MTP for 12 blocks at times [1_000_000, 1_000_600, …, 1_006_600].
    // The 11 preceding blocks span heights 1..=11 (times 1_000_600..1_006_600).
    // Median of 11 = the 6th = 1_003_600. We set time = 1_003_600 (not strictly greater).
    let mtp = 1_003_600_u32; // median of heights 1..=11
    let _ = Block {
        header: BlockHeader {
            version: 4,
            prev_block: tip.hash,
            merkle_root,
            time: mtp, // equal to MTP — must be *greater*
            bits: parent_bits,
            nonce: 0,
        },
        transactions: vec![coinbase],
    };

    // Build a block with time == MTP (not strictly greater).
    let bad_block = {
        let script_sig2 = {
            let mut s = Transaction::bip34_height_script(this_height);
            if s.len() < MIN_COINBASE_SCRIPTSIG_LEN {
                s.push(0x00);
            }
            s
        };
        let coinbase2 = Transaction {
            version: 1,
            inputs: vec![coinbase_input(script_sig2)],
            outputs: vec![make_output(50_0000_0000)],
            locktime: 0,
        };
        let txids2 = vec![coinbase2.txid().expect("txid encoding cannot fail")];
        let mr2 = compute_merkle_root(&txids2).expect("single-tx merkle root must exist");
        Block {
            header: BlockHeader {
                version: 4,
                prev_block: tip.hash,
                merkle_root: mr2,
                time: mtp,
                bits: parent_bits,
                nonce: 0,
            },
            transactions: vec![coinbase2],
        }
    };
    assert_eq!(
        validate_block_context(&bad_block, &view),
        Err(ValidationError::TimestampTooEarly),
    );
}

#[test]
fn stage2_bip34_height_missing_rejected() {
    let view = build_chain_at(BIP34_HEIGHT - 1u32, mantle::primitives::CompactBits(0x207f_ffff), 1_000_000);
    let tip = view.tip();
    assert_eq!(tip.height, BIP34_HEIGHT - 1u32);

    let parent_bits = view.get_header_at(tip.height)
        .expect("storage error").expect("tip header must exist").bits;
    let parent_time = view.get_header_at(tip.height)
        .expect("storage error").expect("tip header must exist").time;

    // coinbase with NO height encoding — just padding bytes.
    let bad_script = vec![0x51, 0x52]; // does not start with encoded height
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input(bad_script)],
        outputs: vec![make_output(50_0000_0000)],
        locktime: 0,
    };
    let txids = vec![coinbase.txid().expect("txid encoding cannot fail")];
    let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");
    let block = Block {
        header: BlockHeader {
            version: 4,
            prev_block: tip.hash,
            merkle_root,
            time: parent_time + 600,
            bits: parent_bits,
            nonce: 0,
        },
        transactions: vec![coinbase],
    };
    assert_eq!(
        validate_block_context(&block, &view),
        Err(ValidationError::Bip34HeightMissing),
    );
}

#[test]
fn stage2_bip34_correct_encoding_accepted() {
    let view = build_chain_at(BIP34_HEIGHT - 1u32, mantle::primitives::CompactBits(0x207f_ffff), 1_000_000);
    let block = stage2_block_for_height(&view, BIP34_HEIGHT, 4);
    // Stage 1 uses easiest PoW so it passes; Stage 2 should also pass.
    validate_block_context(&block, &view)
        .expect("block with correct BIP34 height encoding must pass Stage 2");
}

#[test]
fn stage2_block_version_too_low_rejected() {
    let view = build_chain_at(BlockHeight(388_381 - 1), mantle::primitives::CompactBits(0x207f_ffff), 1_000_000);
    let block = stage2_block_for_height(&view, BlockHeight(388_381), 3); // version 3 < required 4
    assert_eq!(
        validate_block_context(&block, &view),
        Err(ValidationError::BlockVersionTooLow { version: 3, required: 4 }),
    );
}

#[test]
fn stage2_valid_block_accepted() {
    // Small chain, pre-BIP34: no height encoding required, version 1 is fine.
    let view = build_chain(5, mantle::primitives::CompactBits(0x207f_ffff), 1_000_000);
    let tip = view.tip();
    let parent_bits = view.get_header_at(tip.height)
        .expect("storage error").expect("tip header must exist").bits;
    let parent_time = view.get_header_at(tip.height)
        .expect("storage error").expect("tip header must exist").time;
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input(vec![0x51, 0x52])],
        outputs: vec![make_output(50_0000_0000)],
        locktime: 0,
    };
    let txids = vec![coinbase.txid().expect("txid encoding cannot fail")];
    let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: tip.hash,
            merkle_root,
            time: parent_time + 600,
            bits: parent_bits,
            nonce: 0,
        },
        transactions: vec![coinbase],
    };
    validate_block_context(&block, &view).expect("minimal valid block must pass Stage 2");
}

// ── Known transaction fixtures ────────────────────────────────────────────────
//
// These are hand-encoded from Bitcoin Core's tx_valid / tx_invalid test data.
// Only a curated subset is included to keep the file self-contained and fast.

/// The genesis coinbase transaction (raw hex), confirmed in block 0.
const GENESIS_COINBASE_RAW: &str = concat!(
    "01000000", // version = 1
    "01",       // input count = 1
    // prevout hash (null, 32 bytes)
    "0000000000000000000000000000000000000000000000000000000000000000",
    "ffffffff", // prevout index = 0xffffffff (coinbase)
    // scriptSig length + bytes: "The Times 03/Jan/2009 Chancellor on brink of second bailout for banks"
    "4d04ffff001d0104455468652054696d65732030332f4a616e2f32303039204368616e63656c6c6f72206f6e206272696e6b206f66207365636f6e64206261696c6f757420666f722062616e6b73",
    "ffffffff", // sequence
    "01",       // output count = 1
    "00f2052a01000000", // value = 5_000_000_000 (50 BTC)
    // scriptPubKey (65-byte uncompressed pubkey + OP_CHECKSIG)
    "434104678afdb0fe5548271967f1a67130b7105cd6a828e03909a67962e0ea1f61deb649f6bc3f4cef38c4f35504e51ec112de5c384df7ba0b8d578a4c702b6bf11d5fac",
    "00000000"  // locktime = 0
);

/// Known txid of the genesis coinbase (display / reversed byte order).
const GENESIS_COINBASE_TXID_DISPLAY: &str =
    "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";

#[test]
fn genesis_coinbase_decode_roundtrip() {
    let bytes = decode_hex(GENESIS_COINBASE_RAW);
    let tx = Transaction::decode(&mut Cursor::new(&bytes)).expect("genesis coinbase must decode");

    // Basic structure checks.
    assert_eq!(tx.version, 1);
    assert_eq!(tx.inputs.len(), 1);
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].value, mantle::primitives::Satoshi::new(50_0000_0000));
    assert_eq!(tx.locktime, 0);

    // Re-encode must produce identical bytes.
    let mut reencoded = Vec::new();
    tx.encode(&mut reencoded).expect("Vec<u8> write cannot fail");
    assert_eq!(reencoded, bytes, "encode(decode(x)) != x for genesis coinbase");
}

#[test]
fn genesis_coinbase_txid_matches() {
    let bytes = decode_hex(GENESIS_COINBASE_RAW);
    let tx = Transaction::decode(&mut Cursor::new(&bytes)).expect("genesis coinbase must decode");
    let computed = tx.txid().expect("txid encoding cannot fail");
    assert_eq!(
        hash_to_hex_display(computed),
        GENESIS_COINBASE_TXID_DISPLAY,
        "genesis coinbase txid mismatch",
    );
}

#[test]
fn genesis_coinbase_is_valid_null_outpoint() {
    let bytes = decode_hex(GENESIS_COINBASE_RAW);
    let tx = Transaction::decode(&mut Cursor::new(&bytes)).expect("genesis coinbase must decode");
    // Input must reference the null outpoint (coinbase marker).
    assert_eq!(tx.inputs[0].previous_output.txid, Hash256::ZERO);
    assert_eq!(tx.inputs[0].previous_output.vout, u32::MAX);
}

// ── BIP68 / sequence numbers (fixture regression) ────────────────────────────
//
// Sequence number 0xffffffff means "final" (no relative lock-time). We verify
// the codec preserves arbitrary sequence values.

#[test]
fn sequence_field_roundtrip() {
    for &seq in &[0u32, 1, 0xffff_fffe, 0xffff_ffff] {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint { txid: Hash256([0xab; 32]), vout: 0 },
                script_sig: vec![].into(),
                sequence: seq,
                witness: Witness::empty(),
            }],
            outputs: vec![make_output(1000)],
            locktime: 0,
        };
        let mut buf = Vec::new();
        tx.encode(&mut buf).expect("Vec<u8> write cannot fail");
        let decoded = Transaction::decode(&mut Cursor::new(&buf))
            .expect("transaction roundtrip must decode");
        assert_eq!(decoded.inputs[0].sequence, seq, "sequence {seq:#010x} not preserved");
    }
}

// ── ChainView UTXO fixtures ───────────────────────────────────────────────────

#[test]
fn memory_chain_view_utxo_insert_lookup_spend() {
    let mut view = MemoryChainView::new();
    let op = OutPoint { txid: Hash256([0x12; 32]), vout: 3 };
    let coin = Coin {
        value: mantle::primitives::Satoshi::new(100_000),
        script_pubkey: vec![0x76, 0xa9, 0x14].into(), // OP_DUP OP_HASH160 OP_DATA14
        height: BlockHeight(500_000),
        coinbase: false,
    };

    assert!(view.get_coin(&op).expect("storage error").is_none(), "UTXO must not exist before insert");
    view.insert_coin(op, coin.clone());
    assert_eq!(view.get_coin(&op).expect("storage error"), Some(coin), "UTXO must be retrievable after insert");

    view.spend_coin(&op);
    assert!(view.get_coin(&op).expect("storage error").is_none(), "UTXO must be gone after spend");
}

#[test]
fn memory_chain_view_coinbase_flag_preserved() {
    let mut view = MemoryChainView::new();
    let op = OutPoint { txid: Hash256([0x99; 32]), vout: 0 };
    let coin = Coin {
        value: mantle::primitives::Satoshi::new(50_0000_0000),
        script_pubkey: vec![0x51].into(),
        height: BlockHeight::ZERO,
        coinbase: true,
    };
    view.insert_coin(op, coin.clone());
    let retrieved = view.get_coin(&op).expect("storage error").expect("coin must exist after insert");
    assert!(retrieved.coinbase, "coinbase flag must survive round-trip through MemoryChainView");
    assert_eq!(retrieved.height, BlockHeight::ZERO);
    assert_eq!(retrieved.value, mantle::primitives::Satoshi::new(50_0000_0000));
}

// ── validate_block_wire ───────────────────────────────────────────────────────

use std::collections::HashSet;
use crate::validation::validate_block_wire;

fn wire_scratch() -> (Vec<mantle::primitives::Hash256>, Vec<mantle::primitives::Hash256>, HashSet<OutPoint>, HashSet<mantle::primitives::Hash256>) {
    (Vec::new(), Vec::new(), HashSet::new(), HashSet::new())
}

#[test]
fn wire_genesis_passes_stage1() {
    let bytes = decode_hex(GENESIS_BLOCK_HEX);
    let (mut t, mut mb, mut si, mut st) = wire_scratch();
    validate_block_wire(&bytes, &mut t, &mut mb, &mut si, &mut st)
        .expect("genesis wire must pass Stage 1");
}

#[test]
fn wire_agrees_with_owned_on_valid_block() {
    let block = minimal_block(vec![0x03, 0x01, 0x00, 0x00], vec![]);
    let mut bytes = Vec::new();
    block.encode(&mut bytes).expect("Vec<u8> write cannot fail");

    let owned_result = validate_block_structure(&block);
    let (mut t, mut mb, mut si, mut st) = wire_scratch();
    let wire_result = validate_block_wire(&bytes, &mut t, &mut mb, &mut si, &mut st);
    assert_eq!(owned_result.is_ok(), wire_result.is_ok());
}

#[test]
fn wire_empty_block_rejected() {
    // Header + VarInt(0) — no transactions.
    let block = Block {
        header: BlockHeader {
            version: 1, prev_block: Hash256::ZERO, merkle_root: Hash256::ZERO,
            time: 0, bits: mantle::primitives::CompactBits(0x207f_ffff), nonce: 0,
        },
        transactions: vec![],
    };
    let mut bytes = Vec::new();
    block.encode(&mut bytes).expect("Vec<u8> write cannot fail");
    let (mut t, mut mb, mut si, mut st) = wire_scratch();
    assert_eq!(
        validate_block_wire(&bytes, &mut t, &mut mb, &mut si, &mut st),
        Err(ValidationError::EmptyBlock),
    );
}

#[test]
fn wire_merkle_mismatch_rejected() {
    let mut block = minimal_block(vec![0x03, 0x01, 0x00, 0x00], vec![]);
    block.header.merkle_root = Hash256([0xff; 32]); // corrupt the root
    let mut bytes = Vec::new();
    block.encode(&mut bytes).expect("Vec<u8> write cannot fail");
    let (mut t, mut mb, mut si, mut st) = wire_scratch();
    assert_eq!(
        validate_block_wire(&bytes, &mut t, &mut mb, &mut si, &mut st),
        Err(ValidationError::MerkleRootMismatch),
    );
}

#[test]
fn wire_truncated_returns_malformed() {
    let block = minimal_block(vec![0x03, 0x01, 0x00, 0x00], vec![]);
    let mut bytes = Vec::new();
    block.encode(&mut bytes).expect("Vec<u8> write cannot fail");
    let truncated = &bytes[..bytes.len() / 2];
    let (mut t, mut mb, mut si, mut st) = wire_scratch();
    let err = validate_block_wire(truncated, &mut t, &mut mb, &mut si, &mut st)
        .expect_err("truncated block must fail validation");
    assert!(
        matches!(err, ValidationError::MalformedTransaction { .. }),
        "expected MalformedTransaction, got {err:?}",
    );
}

#[test]
fn wire_agrees_with_owned_on_all_stage1_errors() {
    // Run a matrix of blocks that owned validation rejects and verify
    // wire validation returns the same error variant.
    use mantle::primitives::TxIn;

    let cases: &[(&str, Block)] = &[
        ("first_tx_not_coinbase", {
            let non_cb = Transaction {
                version: 1,
                inputs: vec![TxIn {
                    previous_output: OutPoint { txid: Hash256([0xab; 32]), vout: 0 },
                    script_sig: vec![0x51].into(),
                    sequence: u32::MAX,
                    witness: Witness::empty(),
                }],
                outputs: vec![make_output(1000)],
                locktime: 0,
            };
            let txids = vec![non_cb.txid().expect("txid encoding cannot fail")];
            let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");
            Block {
                header: BlockHeader {
                    version: 4, prev_block: Hash256::ZERO, merkle_root,
                    time: 1_231_006_505, bits: mantle::primitives::CompactBits(0x207f_ffff), nonce: 0,
                },
                transactions: vec![non_cb],
            }
        }),
        ("coinbase_script_too_short",
            minimal_block(vec![0x51], vec![])),
        ("output_value_overflow", {
            let coinbase = Transaction {
                version: 1,
                inputs: vec![coinbase_input(vec![0x03, 0x01, 0x00, 0x00])],
                outputs: vec![TxOut { value: mantle::primitives::Satoshi::new(u64::MAX), script_pubkey: vec![0x51].into() }],
                locktime: 0,
            };
            let txids = vec![coinbase.txid().expect("txid encoding cannot fail")];
            let merkle_root = compute_merkle_root(&txids).expect("single-tx merkle root must exist");
            Block {
                header: BlockHeader {
                    version: 4, prev_block: Hash256::ZERO, merkle_root,
                    time: 1_231_006_505, bits: mantle::primitives::CompactBits(0x207f_ffff), nonce: 0,
                },
                transactions: vec![coinbase],
            }
        }),
    ];

    for (name, block) in cases {
        let mut bytes = Vec::new();
        block.encode(&mut bytes).expect("Vec<u8> write cannot fail");
        let owned_err = validate_block_structure(block).expect_err("case must fail owned validation");
        let (mut t, mut mb, mut si, mut st) = wire_scratch();
        let wire_err = validate_block_wire(&bytes, &mut t, &mut mb, &mut si, &mut st)
            .expect_err("case must fail wire validation");
        assert_eq!(
            std::mem::discriminant(&owned_err),
            std::mem::discriminant(&wire_err),
            "case {name}: owned={owned_err:?} wire={wire_err:?}",
        );
    }
}

// ── BIP66: strict-DER signature encoding ─────────────────────────────────────
//
// BIP66 narrowed the legacy ECDSA signature encoding to canonical DER. The
// vectors below cover both sides of that line:
//
// - **Accept** vectors are canonical DER: outer `SEQUENCE`, two unsigned
//   `INTEGER` bodies, no superfluous leading zeros, no negative values.
// - **Reject** vectors are the forms that *did* parse on pre-BIP66 nodes
//   (libsecp256k1's lenient parser) but are now consensus-invalid.
//
// We exercise `consensus_crypto::is_valid_signature_encoding_der` — the
// behavioural mirror of Core's `IsValidSignatureEncoding`. Both arms include
// a brief description so failures point at the rule under test.
//
// Note: these are signature *bodies*, not `<DER>||<sighash>` blobs — the
// trailing sighash byte is split off by `TxSigChecker` before this validator
// runs. Vector lengths are 8..=72 (the post-split body range).

mod bip66 {
    use crate::script::consensus_crypto::is_valid_signature_encoding_der;

    /// Smallest possible canonical DER signature: `r=1, s=1`.
    /// `30 06 02 01 01 02 01 01`.
    fn minimal_canonical() -> Vec<u8> {
        vec![0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01]
    }

    /// `r` with high bit set, requiring a leading 0x00 sign byte.
    /// `30 07 02 02 00 80 02 01 01`.
    fn r_with_required_zero_pad() -> Vec<u8> {
        vec![0x30, 0x07, 0x02, 0x02, 0x00, 0x80, 0x02, 0x01, 0x01]
    }

    /// 32-byte `r` and `s` — typical real signature shape.
    /// `30 44 02 20 <32 bytes> 02 20 <32 bytes>`.
    fn full_size_canonical() -> Vec<u8> {
        let mut v = vec![0x30, 0x44, 0x02, 0x20];
        v.extend_from_slice(&[0x7f; 32]); // top bit clear → no padding needed
        v.push(0x02);
        v.push(0x20);
        v.extend_from_slice(&[0x7f; 32]);
        v
    }

    #[test]
    fn accept_minimal() {
        assert!(is_valid_signature_encoding_der(&minimal_canonical()));
    }

    #[test]
    fn accept_r_with_required_leading_zero() {
        assert!(is_valid_signature_encoding_der(&r_with_required_zero_pad()));
    }

    #[test]
    fn accept_full_size() {
        assert!(is_valid_signature_encoding_der(&full_size_canonical()));
    }

    #[test]
    fn accept_72_byte_max() {
        // 72-byte total: SEQUENCE (2) + r INTEGER (35: tag+len+33 bytes with
        // leading 0x00) + s INTEGER (35: same shape).
        let mut v = vec![0x30, 0x46, 0x02, 0x21, 0x00];
        v.extend_from_slice(&[0xff; 32]); // 33-byte r magnitude with leading 0
        v.push(0x02);
        v.push(0x21);
        v.push(0x00);
        v.extend_from_slice(&[0xff; 32]);
        assert_eq!(v.len(), 72);
        assert!(is_valid_signature_encoding_der(&v));
    }

    // ── Reject vectors ───────────────────────────────────────────────────────

    #[test]
    fn reject_too_short() {
        // 7 bytes — under the 8-byte minimum.
        assert!(!is_valid_signature_encoding_der(&[0x30, 0x05, 0x02, 0x01, 0x01, 0x02, 0x00]));
    }

    #[test]
    fn reject_too_long() {
        // 73 bytes — over the 72-byte maximum.
        let v = vec![0u8; 73];
        assert!(!is_valid_signature_encoding_der(&v));
    }

    #[test]
    fn reject_wrong_outer_tag() {
        // Outer tag must be 0x30 (SEQUENCE).
        let mut v = minimal_canonical();
        v[0] = 0x31;
        assert!(!is_valid_signature_encoding_der(&v));
    }

    #[test]
    fn reject_total_length_mismatch() {
        // Total-length byte claims 5 but body is 6.
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x05, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01,
        ]));
    }

    #[test]
    fn reject_r_not_integer_tag() {
        let mut v = minimal_canonical();
        v[2] = 0x03; // BIT STRING instead of INTEGER
        assert!(!is_valid_signature_encoding_der(&v));
    }

    #[test]
    fn reject_r_zero_length() {
        // r length 0 — empty INTEGER bodies aren't valid magnitude encodings.
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x05, 0x02, 0x00, 0x02, 0x01, 0x01,
        ]));
    }

    #[test]
    fn reject_r_negative() {
        // r high bit set without a leading 0x00 → reads as negative.
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x06, 0x02, 0x01, 0x80, 0x02, 0x01, 0x01,
        ]));
    }

    #[test]
    fn reject_r_redundant_leading_zero() {
        // r = 00 01 — leading zero is redundant since 0x01's high bit is clear.
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x07, 0x02, 0x02, 0x00, 0x01, 0x02, 0x01, 0x01,
        ]));
    }

    #[test]
    fn reject_s_not_integer_tag() {
        // r is fine; s tag is 0x03 instead of 0x02.
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x06, 0x02, 0x01, 0x01, 0x03, 0x01, 0x01,
        ]));
    }

    #[test]
    fn reject_s_zero_length() {
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x05, 0x02, 0x01, 0x01, 0x02, 0x00,
        ]));
    }

    #[test]
    fn reject_s_negative() {
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x80,
        ]));
    }

    #[test]
    fn reject_s_redundant_leading_zero() {
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x07, 0x02, 0x01, 0x01, 0x02, 0x02, 0x00, 0x01,
        ]));
    }

    #[test]
    fn reject_truncated_after_r_length_byte() {
        // r tag + r len byte but no body and no s.
        assert!(!is_valid_signature_encoding_der(&[
            0x30, 0x02, 0x02, 0x01,
        ]));
    }

    #[test]
    fn reject_trailing_garbage() {
        // Canonical signature with one extra byte appended — total length
        // byte no longer matches actual body length.
        let mut v = minimal_canonical();
        v.push(0xff);
        assert!(!is_valid_signature_encoding_der(&v));
    }
}
