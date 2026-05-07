use mantle::codec::Encode;
use mantle::hash::sha256d;
use mantle::primitives::{Block, BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, Transaction, TxHash, TxIn, TxOut};
use mantle_consensus::chain_view::{ChainView, Coin, MemoryChainView};
use mantle_consensus::merkle::{compute_merkle_root_reusing, merkle_root_owned};
use mantle_consensus::script::{Interpreter, NopSigChecker, ScriptFlags, SigVersion};
use mantle_consensus::validation::{validate_block_structure_reusing, validate_block_wire};

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
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: height_script.into(),
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value: Satoshi::new(value), script_pubkey: vec![0x51].into() }],
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
        }],
        outputs: vec![TxOut { value: Satoshi::new(value), script_pubkey: vec![0x51].into() }],
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
            bits: CompactBits(0x207f_ffff),
            nonce: 0,
        };
        prev = h.hash();
        view.push_header(h);
    }
    view
}

fn mine_block(mut block: Block) -> Block {
    let target = block.header.bits.to_target()
        .expect("mine_block called with invalid nBits");
    for nonce in 0..=u32::MAX {
        block.header.nonce = nonce;
        let raw = sha256d(&block.header.to_bytes());
        let mut be = raw;
        be.reverse();
        if be <= target { return block; }
    }
    panic!("no nonce found");
}

fn make_valid_block(txs: Vec<Transaction>) -> Block {
    let txids: Vec<Hash256> = txs.iter()
        .map(Transaction::txid)
        .collect::<Result<Vec<_>, _>>()
        .expect("txid encoding cannot fail");
    let merkle_root = merkle_root_owned(txids)
        .expect("non-empty tx list always has a merkle root");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 1_231_006_505,
            bits: CompactBits(0x207f_ffff),
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
            txs.push(regular_tx(Hash256([i; 32]), 0, 1000));
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
            ).expect("valid block must pass structure check");
        }
    }

    // --- scenario 2: encode + decode a 50-tx block (200 iters) ---
    {
        use mantle::codec::Decode;
        use std::io::Cursor;

        let cb = coinbase_tx(vec![0x03, 0x32, 0x00, 0x00], 5_000_000_000);
        let mut txs = vec![cb];
        for i in 1..50u8 {
            txs.push(regular_tx(Hash256([i; 32]), 0, 1000));
        }
        let block = make_valid_block(txs);
        let mut encode_buf: Vec<u8> = Vec::new();
        for _ in 0..200 {
            encode_buf.clear();
            block.encode(&mut encode_buf).expect("Vec<u8> write cannot fail");
            let _ = Block::decode(&mut Cursor::new(encode_buf.as_slice())).expect("valid block bytes must decode");
        }
    }

    // --- scenario 3: chain view build + tip lookups (1000 iters) ---
    {
        let view = build_chain(200);
        for i in 0..1000u32 {
            let _ = view.get_header_at(BlockHeight(i % 200)).expect("storage error");
            let tip = view.tip();
            let _ = view.get_header(&tip.hash).expect("storage error");
        }
    }

    // --- scenario 4: merkle root over 1000 txids (100 iters) ---
    {
        let template: Vec<Hash256> = (0..1000u32).map(|i| Hash256([(i & 0xff) as u8; 32])).collect();
        let mut buf: Vec<Hash256> = Vec::with_capacity(template.len());
        for _ in 0..100 {
            let _ = compute_merkle_root_reusing(&template, &mut buf).expect("non-empty tx list must have merkle root");
        }
    }

    // --- scenario 5: MemoryChainView coin insert/lookup/spend (5000 iters) ---
    {
        let mut view = MemoryChainView::new();
        for i in 0..5000u32 {
            let op = OutPoint { txid: Hash256(sha256d(&i.to_le_bytes())), vout: 0 };
            view.insert_coin(op, Coin { value: Satoshi::new(1000), script_pubkey: vec![0x51].into(), height: BlockHeight(i), coinbase: false });
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
            txs.push(regular_tx(Hash256([i; 32]), 0, 1000));
        }
        let wire = encode(&make_valid_block(txs));
        let mut txids: Vec<Hash256> = Vec::new();
        let mut merkle_buf: Vec<Hash256> = Vec::new();
        let mut seen_inputs: HashSet<OutPoint> = HashSet::new();
        let mut seen_txids: HashSet<Hash256> = HashSet::new();
        for _ in 0..500 {
            validate_block_wire(&wire, &mut txids, &mut merkle_buf, &mut seen_inputs, &mut seen_txids).expect("valid block wire must pass structure check");
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

    // --- scenario 8: script Interpreter::eval on a hash + arithmetic mix (5000 iters) ---
    // Real-shape script: P2PKH-ish (DUP HASH160 PUSHBYTES_20 <hash> EQUALVERIFY OP_1)
    // + a small arithmetic chain. Closest to typical mainnet pre-CHECKSIG work.
    // Allocation pressure here comes from each push (Vec::new() per stack
    // element) and each hash op (digest.to_vec() into a fresh allocation).
    {
        // HASH160(empty) = b472a266d0bd89c13706a4132ccfb16f7c3b9fcb.
        // Same vector covered by the script::interpreter::op_hash160_known_vector test,
        // so we can pin it as a const here without a hash-crate dep.
        const HASH160_EMPTY: [u8; 20] = [
            0xb4, 0x72, 0xa2, 0x66, 0xd0, 0xbd, 0x89, 0xc1, 0x37, 0x06,
            0xa4, 0x13, 0x2c, 0xcf, 0xb1, 0x6f, 0x7c, 0x3b, 0x9f, 0xcb,
        ];
        let checker = NopSigChecker;
        let mut script = Vec::with_capacity(64);
        script.push(0x00);                                             // OP_0 (push empty)
        script.push(0x76);                                             // OP_DUP
        script.push(0xa9);                                             // OP_HASH160
        script.push(0x14); script.extend_from_slice(&HASH160_EMPTY);   // PUSHBYTES_20 expected
        script.push(0x88);                                             // OP_EQUALVERIFY
        // Arithmetic: 1 + 2 + 3 + 4 + 5 = 15
        for k in 0u8..5 {
            script.push(0x51 + k); // OP_(k+1)
            if k > 0 { script.push(0x93); } // OP_ADD
        }
        for _ in 0..5_000 {
            let mut interp = Interpreter::new(&checker, ScriptFlags::NONE, SigVersion::Base);
            interp.eval(&script).expect("script must succeed");
        }
    }
}
