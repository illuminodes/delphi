use mantle::codec::Encode;
use mantle::hash::sha256d;
use mantle::primitives::{Block, BlockHeader, Hash256, OutPoint, Transaction, TxIn, TxOut};
use mantle_consensus::chain_view::{ChainView, Coin, MemoryChainView};
use mantle_consensus::merkle::merkle_root_owned;
use mantle_consensus::validation::{txid, validate_block_structure_reusing};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn encode<T: Encode>(val: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    val.encode(&mut buf).unwrap();
    buf
}

fn coinbase_tx(height_script: Vec<u8>, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: height_script,
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value, script_pubkey: vec![0x51] }],
        locktime: 0,
    }
}

fn regular_tx(prev: Hash256, vout: u32, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: prev, vout },
            script_sig: vec![0x51; 20],
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value, script_pubkey: vec![0x51] }],
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
            bits: 0x207f_ffff,
            nonce: 0,
        };
        prev = Hash256(sha256d(&h.to_bytes()));
        view.push_header(h);
    }
    view
}

fn nbits_to_target(bits: u32) -> Option<[u8; 32]> {
    let exponent = (bits >> 24) as usize;
    let mantissa = bits & 0x007f_ffff;
    if bits & 0x0080_0000 != 0 || mantissa == 0 || exponent > 34 { return None; }
    let mut target = [0u8; 32];
    if exponent >= 3 {
        let start = 32 - exponent;
        target[start] = ((mantissa >> 16) & 0xff) as u8;
        target[start + 1] = ((mantissa >> 8) & 0xff) as u8;
        target[start + 2] = (mantissa & 0xff) as u8;
    }
    Some(target)
}

fn mine_block(mut block: Block) -> Block {
    let target = nbits_to_target(block.header.bits).unwrap();
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
    let txids: Vec<Hash256> = txs.iter().map(txid).collect();
    let merkle_root = merkle_root_owned(txids).unwrap();
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 1_231_006_505,
            bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: txs,
    };
    mine_block(block)
}

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
        let mut seen_inputs = HashSet::new();
        let mut seen_txids = HashSet::new();
        for _ in 0..500 {
            validate_block_structure_reusing(
                &block,
                &mut serial_scratch,
                &mut txids,
                &mut seen_inputs,
                &mut seen_txids,
            ).unwrap();
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
        for _ in 0..200 {
            let bytes = encode(&block);
            let _ = Block::decode(&mut Cursor::new(bytes.as_slice())).unwrap();
        }
    }

    // --- scenario 3: chain view build + tip lookups (1000 iters) ---
    {
        let view = build_chain(200);
        for i in 0..1000u32 {
            let _ = view.get_header_at(i % 200).unwrap();
            let tip = view.tip();
            let _ = view.get_header(&tip.hash).unwrap();
        }
    }

    // --- scenario 4: merkle root over 1000 txids (100 iters) ---
    {
        let template: Vec<Hash256> = (0..1000u32).map(|i| Hash256([(i & 0xff) as u8; 32])).collect();
        for _ in 0..100 {
            let _ = merkle_root_owned(template.clone()).unwrap();
        }
    }

    // --- scenario 5: MemoryChainView coin insert/lookup/spend (5000 iters) ---
    {
        let mut view = MemoryChainView::new();
        for i in 0..5000u32 {
            let op = OutPoint { txid: Hash256(sha256d(&i.to_le_bytes())), vout: 0 };
            view.insert_coin(op, Coin { value: 1000, script_pubkey: vec![0x51], height: i, coinbase: false });
            let _ = view.get_coin(&op).unwrap();
            view.spend_coin(&op);
        }
    }
}
