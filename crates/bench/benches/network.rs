use mantle::codec::{Decode, Encode, VarInt};
use mantle::hash::checksum;
use mantle::network::{
    Command, CommandName, GetHeadersMsg, HeadersMsg, InvEntry, InvMsg, InvType, Magic,
    MessageHeader, NetworkAddr, PingMsg, VersionMsg, encode_message,
};
use mantle::primitives::{BlockHeader, Hash256};
use std::io::Cursor;
use std::hint::black_box;

fn main() {
    divan::main();
}

fn encode_bytes<T: Encode>(val: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    val.encode(&mut buf).expect("Vec<u8> write cannot fail");
    buf
}

fn make_addr() -> NetworkAddr {
    NetworkAddr { services: 1, ip: [0u8; 16], port: 8333 }
}

fn make_version_msg() -> VersionMsg {
    let addr = make_addr();
    VersionMsg {
        version: 70015,
        services: 1,
        timestamp: 1_700_000_000,
        receiver: addr,
        sender: addr,
        nonce: 0xdead_beef,
        user_agent: "/mantle:0.1.0/".to_string(),
        start_height: 800_000,
        relay: true,
    }
}

// ── MessageHeader ─────────────────────────────────────────────────────────────

#[divan::bench]
fn message_header_decode(bencher: divan::Bencher) {
    let payload = b"nonce_8_bytes___";
    let bytes = encode_bytes(&MessageHeader {
        magic: Magic::Mainnet,
        command: CommandName::new("ping"),
        length: u32::try_from(payload.len()).expect("payload fits u32"),
        checksum: checksum(payload),
    });
    bencher.bench(|| MessageHeader::decode(&mut Cursor::new(bytes.as_slice())).expect("MessageHeader decode cannot fail on valid bytes"));
}

#[divan::bench]
fn encode_message_ping() -> Vec<u8> {
    let nonce = 0xdead_beef_cafe_babe_u64.to_le_bytes();
    let mut buf = Vec::with_capacity(32);
    encode_message(Magic::Mainnet, Command::Ping, &nonce, &mut buf).expect("encode_message cannot fail on Vec<u8>");
    buf
}

// ── VersionMsg ────────────────────────────────────────────────────────────────

#[divan::bench]
fn version_msg_encode(bencher: divan::Bencher) {
    let msg = make_version_msg();
    bencher.bench(|| encode_bytes(&msg));
}

#[divan::bench]
fn version_msg_decode(bencher: divan::Bencher) {
    let bytes = encode_bytes(&make_version_msg());
    bencher.bench(|| VersionMsg::decode(&mut Cursor::new(bytes.as_slice())).expect("VersionMsg decode cannot fail on valid bytes"));
}

// ── PingMsg ───────────────────────────────────────────────────────────────────

#[divan::bench]
fn ping_encode() -> Vec<u8> {
    encode_bytes(&PingMsg { nonce: 0xdead_beef })
}

#[divan::bench]
fn ping_decode(bencher: divan::Bencher) {
    let bytes = encode_bytes(&PingMsg { nonce: 0xdead_beef });
    bencher.bench(|| PingMsg::decode(&mut Cursor::new(bytes.as_slice())).expect("PingMsg decode cannot fail on valid bytes"));
}

// ── InvMsg ────────────────────────────────────────────────────────────────────

#[divan::bench(args = [1usize, 16, 256, 2000])]
fn inv_msg_encode(bencher: divan::Bencher, n: usize) {
    let msg = InvMsg((0..n).map(|i| InvEntry { kind: InvType::Block, hash: Hash256([u8::try_from(i & 0xff).unwrap(); 32]) }).collect());
    bencher.bench(|| encode_bytes(&msg));
}

#[divan::bench(args = [1usize, 16, 256, 2000])]
fn inv_msg_decode(bencher: divan::Bencher, n: usize) {
    let msg = InvMsg((0..n).map(|i| InvEntry { kind: InvType::Block, hash: Hash256([u8::try_from(i & 0xff).unwrap(); 32]) }).collect());
    let bytes = encode_bytes(&msg);
    bencher.bench(|| InvMsg::decode(&mut Cursor::new(bytes.as_slice())).expect("InvMsg decode cannot fail on valid bytes"));
}

#[divan::bench(args = [1usize, 16, 256, 2000])]
fn inv_msg_iter(bencher: divan::Bencher, n: usize) {
    let msg = InvMsg((0..n).map(|i| InvEntry { kind: InvType::Block, hash: Hash256([u8::try_from(i & 0xff).unwrap(); 32]) }).collect());
    let bytes = encode_bytes(&msg);
    bencher.bench(|| {
        let mut count = 0usize;
        for entry in InvMsg::iter_entries::<50_000>(&bytes).expect("iter_entries cannot fail on valid bytes") {
            black_box(entry.expect("inv entry decode cannot fail on valid bytes"));
            count += 1;
        }
        count
    });
}

#[divan::bench(args = [1usize, 16, 256, 2000])]
fn inv_msg_decode_into(bencher: divan::Bencher, n: usize) {
    let msg = InvMsg((0..n).map(|i| InvEntry { kind: InvType::Block, hash: Hash256([u8::try_from(i & 0xff).unwrap(); 32]) }).collect());
    let bytes = encode_bytes(&msg);
    let mut buf = Vec::new();
    bencher.bench_local(|| {
        InvMsg::decode_into_limited::<50_000>(&mut Cursor::new(bytes.as_slice()), &mut buf).expect("decode_into_limited cannot fail on valid bytes");
        black_box(buf.len())
    });
}

// ── HeadersMsg ────────────────────────────────────────────────────────────────

fn make_header(i: usize) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256([u8::try_from(i & 0xff).unwrap(); 32]),
        merkle_root: Hash256::ZERO,
        time: u32::try_from(i).expect("bench index fits u32"),
        bits: mantle::primitives::CompactBits(0x1d00_ffff),
        nonce: u32::try_from(i).expect("bench index fits u32"),
    }
}

#[divan::bench(args = [1usize, 100, 2000])]
fn headers_msg_encode(bencher: divan::Bencher, n: usize) {
    let msg = HeadersMsg((0..n).map(make_header).collect());
    bencher.bench(|| encode_bytes(&msg));
}

#[divan::bench(args = [1usize, 100, 2000])]
fn headers_msg_decode(bencher: divan::Bencher, n: usize) {
    let msg = HeadersMsg((0..n).map(make_header).collect());
    let bytes = encode_bytes(&msg);
    bencher.bench(|| HeadersMsg::decode(&mut Cursor::new(bytes.as_slice())).expect("HeadersMsg decode cannot fail on valid bytes"));
}

#[divan::bench(args = [1usize, 100, 2000])]
fn headers_msg_iter(bencher: divan::Bencher, n: usize) {
    let msg = HeadersMsg((0..n).map(make_header).collect());
    let bytes = encode_bytes(&msg);
    bencher.bench(|| {
        let mut count = 0usize;
        for header in HeadersMsg::iter_entries::<2_000>(&bytes).expect("iter_entries cannot fail on valid bytes") {
            black_box(header.expect("header decode cannot fail on valid bytes"));
            count += 1;
        }
        count
    });
}

#[divan::bench(args = [1usize, 100, 2000])]
fn headers_msg_decode_into(bencher: divan::Bencher, n: usize) {
    let msg = HeadersMsg((0..n).map(make_header).collect());
    let bytes = encode_bytes(&msg);
    let mut buf = Vec::new();
    bencher.bench_local(|| {
        HeadersMsg::decode_into_limited::<2_000>(&mut Cursor::new(bytes.as_slice()), &mut buf).expect("decode_into_limited cannot fail on valid bytes");
        black_box(buf.len())
    });
}

// ── GetHeadersMsg ─────────────────────────────────────────────────────────────

#[divan::bench]
fn getheaders_encode(bencher: divan::Bencher) {
    let msg = GetHeadersMsg {
        version: 70015,
        locator_hashes: (0..10u8).map(|i| Hash256([i; 32])).collect(),
        stop_hash: Hash256::ZERO,
    };
    bencher.bench(|| encode_bytes(&msg));
}

#[divan::bench]
fn getheaders_decode(bencher: divan::Bencher) {
    let msg = GetHeadersMsg {
        version: 70015,
        locator_hashes: (0..10u8).map(|i| Hash256([i; 32])).collect(),
        stop_hash: Hash256::ZERO,
    };
    let bytes = encode_bytes(&msg);
    bencher.bench(|| GetHeadersMsg::decode(&mut Cursor::new(bytes.as_slice())).expect("GetHeadersMsg decode cannot fail on valid bytes"));
}

// ── VarInt list ───────────────────────────────────────────────────────────────

#[divan::bench]
fn varint_encode_2000() -> Vec<u8> {
    let mut buf = Vec::with_capacity(2000 * 3);
    for i in 0u64..2000 {
        VarInt(i).encode(&mut buf).expect("Vec<u8> write cannot fail");
    }
    buf
}
