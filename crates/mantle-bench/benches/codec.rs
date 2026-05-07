use mantle_core::codec::{Decode, DecodeRef, Encode, VarInt};
use std::io::Cursor;

fn main() {
    divan::main();
}

// ── VarInt ────────────────────────────────────────────────────────────────────

#[divan::bench(args = [0xfcu64, 0xfdu64, 0x1_0000u64, u64::MAX])]
fn varint_encode(n: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    VarInt(n).encode(&mut buf).unwrap();
    buf
}

#[divan::bench(args = [0xfcu64, 0xfdu64, 0x1_0000u64, u64::MAX])]
fn varint_decode(bencher: divan::Bencher, n: u64) {
    let mut buf = Vec::new();
    VarInt(n).encode(&mut buf).unwrap();
    bencher.bench(|| VarInt::decode(&mut Cursor::new(buf.as_slice())).unwrap());
}

#[divan::bench(args = [0xfcu64, 0xfdu64, 0x1_0000u64, u64::MAX])]
fn varint_decode_ref(bencher: divan::Bencher, n: u64) {
    let mut buf = Vec::new();
    VarInt(n).encode(&mut buf).unwrap();
    bencher.bench(|| VarInt::decode_ref(buf.as_slice()).unwrap());
}

// ── Primitives ────────────────────────────────────────────────────────────────

#[divan::bench]
fn u32_decode_ref() -> (u32, &'static [u8]) {
    u32::decode_ref(&[0x01, 0x00, 0x00, 0x00]).unwrap()
}

#[divan::bench]
fn u64_decode_ref() -> (u64, &'static [u8]) {
    u64::decode_ref(&[0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe]).unwrap()
}

// ── VarStr ────────────────────────────────────────────────────────────────────

mod varstr {
    use super::*;

    fn encode_str(s: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        s.encode(&mut buf).unwrap();
        buf
    }

    #[divan::bench]
    fn encode_short() -> Vec<u8> {
        encode_str("ping")
    }

    #[divan::bench]
    fn encode_long_256(bencher: divan::Bencher) {
        let s = "a".repeat(256);
        bencher.bench(|| encode_str(&s));
    }

    #[divan::bench]
    fn decode_short(bencher: divan::Bencher) {
        let buf = encode_str("ping");
        bencher.bench(|| String::decode(&mut Cursor::new(buf.as_slice())).unwrap());
    }

    #[divan::bench]
    fn decode_long_256(bencher: divan::Bencher) {
        let buf = encode_str(&"a".repeat(256));
        bencher.bench(|| String::decode(&mut Cursor::new(buf.as_slice())).unwrap());
    }

    #[divan::bench]
    fn decode_ref_short(bencher: divan::Bencher) {
        let buf = encode_str("ping");
        bencher.bench(|| <&str>::decode_ref(buf.as_slice()).unwrap());
    }

    #[divan::bench]
    fn decode_ref_long_256(bencher: divan::Bencher) {
        let buf = encode_str(&"a".repeat(256));
        bencher.bench(|| <&str>::decode_ref(buf.as_slice()).unwrap());
    }
}
