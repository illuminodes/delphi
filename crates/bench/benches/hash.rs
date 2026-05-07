use mantle::hash::{checksum, sha256d};

fn main() {
    divan::main();
}

#[divan::bench(args = [0usize, 11, 256, 8192])]
fn sha256d_bench(bencher: divan::Bencher, len: usize) {
    let data = vec![0xabu8; len];
    bencher.bench(|| sha256d(&data));
}

#[divan::bench]
fn checksum_32b() -> [u8; 4] {
    checksum(b"a typical ping payload with nonce")
}
