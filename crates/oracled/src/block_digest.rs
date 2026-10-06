use mantle::amount::Satoshi;
use mantle::codec::VarInt;
use mantle::difficulty::{BlockHeight, CompactBits};
use mantle::primitives::{Block, BlockHeader};
use mantle::tx::Transaction;

use crate::JsonObject;

/// What one block says about itself, read from the block alone.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDigest {
    pub height: BlockHeight,
    pub hash: String,
    pub previous: String,
    pub time: u32,
    pub version: i32,
    pub difficulty: f64,
    pub transactions: usize,
    pub outputs: usize,
    pub size: usize,
    pub weight: u64,
    pub output_value: Satoshi,
    pub coinbase_value: Satoshi,
    pub subsidy: Satoshi,
}

impl BlockDigest {
    const HALVING_INTERVAL: u32 = 210_000;
    const INITIAL_SUBSIDY: u64 = 5_000_000_000;
    const DIFFICULTY_ONE_EXPONENT: i32 = 0x1d;
    const DIFFICULTY_ONE_MANTISSA: f64 = 65_535.0;

    #[must_use]
    pub fn of(height: BlockHeight, block: &Block) -> Self {
        let header = &block.header;
        let coinbase_value = block.transactions.first().map_or(Satoshi::new(0), Self::paid);
        let output_value = block
            .transactions
            .iter()
            .skip(1)
            .map(|transaction| Self::paid(transaction).to_u64())
            .sum();
        Self {
            height,
            hash: header.hash().to_display_hex(),
            previous: header.prev_block().to_display_hex(),
            time: header.time().to_u32(),
            version: header.version().to_i32(),
            difficulty: Self::difficulty_of(header.bits()),
            transactions: block.transactions.len(),
            outputs: block.transactions.iter().map(|transaction| transaction.outputs.len()).sum(),
            size: BlockHeader::ENCODED_LEN
                + VarInt::from(block.transactions.len()).encoded_len()
                + block.transactions.iter().map(Transaction::encoded_len).sum::<usize>(),
            weight: block.weight(),
            output_value: Satoshi::new(output_value),
            coinbase_value,
            subsidy: Self::subsidy_at(height),
        }
    }

    /// The fees the miner collected: what the coinbase paid beyond the subsidy.
    #[must_use]
    pub fn fees(&self) -> Satoshi {
        Satoshi::new(self.coinbase_value.to_u64().saturating_sub(self.subsidy.to_u64()))
    }

    #[must_use]
    pub fn subsidy_at(height: BlockHeight) -> Satoshi {
        let halvings = height.to_u32() / Self::HALVING_INTERVAL;
        Satoshi::new(Self::INITIAL_SUBSIDY.checked_shr(halvings).unwrap_or(0))
    }

    /// The target of `bits` as a multiple of the easiest target, the way nodes report difficulty.
    #[must_use]
    pub fn difficulty_of(bits: CompactBits) -> f64 {
        let raw = bits.to_u32();
        let mantissa = f64::from(raw & 0x007f_ffff);
        if mantissa == 0.0 {
            return f64::NAN;
        }
        let exponent = i32::try_from(raw >> 24).unwrap_or(0);
        Self::DIFFICULTY_ONE_MANTISSA / mantissa * 256_f64.powi(Self::DIFFICULTY_ONE_EXPONENT - exponent)
    }

    fn paid(transaction: &Transaction) -> Satoshi {
        Satoshi::new(transaction.outputs.iter().map(|output| output.value.to_u64()).sum())
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        let mut object = JsonObject::new();
        object.count("height", self.height.to_u32() as usize);
        object.text("hash", &self.hash);
        object.text("previous", &self.previous);
        object.count("time", self.time as usize);
        object.raw("version", &self.version.to_string());
        object.float("difficulty", self.difficulty);
        object.count("transactions", self.transactions);
        object.count("outputs", self.outputs);
        object.count("size", self.size);
        object.raw("weight", &self.weight.to_string());
        object.raw("output_value", &self.output_value.to_u64().to_string());
        object.raw("coinbase_value", &self.coinbase_value.to_u64().to_string());
        object.raw("subsidy", &self.subsidy.to_u64().to_string());
        object.raw("fees", &self.fees().to_u64().to_string());
        object.finish()
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime, BlockVersion, CompactBits, Locktime, Nonce, Sequence, TxVersion};
    use mantle::hash::Hash256;
    use mantle::primitives::{Block, BlockHeader};
    use mantle::script::Witness;
    use mantle::tx::{OutPoint, Transaction, TxIn, TxOut};

    use super::BlockDigest;

    struct DigestFixture;

    impl DigestFixture {
        fn transaction(values: &[u64]) -> Transaction {
            Transaction {
                version: TxVersion::new(2),
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::ZERO,
                        vout: 0,
                    },
                    script_sig: vec![0x01].into(),
                    sequence: Sequence::MAX,
                    witness: Witness::empty(),
                }],
                outputs: values
                    .iter()
                    .map(|value| TxOut {
                        value: Satoshi::new(*value),
                        script_pubkey: vec![0x51].into(),
                    })
                    .collect(),
                locktime: Locktime::new(0),
            }
        }

        fn block() -> Block {
            Block {
                header: BlockHeader::new(
                    BlockVersion::new(0x2000_0000),
                    Hash256::hash(b"previous"),
                    Hash256::ZERO,
                    BlockTime::new(1_780_000_000),
                    CompactBits::new(0x1d00_ffff),
                    Nonce::new(7),
                ),
                transactions: vec![
                    Self::transaction(&[312_500_000 + 20_000]),
                    Self::transaction(&[1_000, 2_000]),
                    Self::transaction(&[5_000]),
                ],
            }
        }
    }

    #[test]
    fn a_block_is_counted_and_its_fees_come_from_the_coinbase() {
        let digest = BlockDigest::of(BlockHeight::new(840_000), &DigestFixture::block());

        assert_eq!(digest.transactions, 3);
        assert_eq!(digest.outputs, 4);
        assert_eq!(digest.output_value, Satoshi::new(8_000));
        assert_eq!(digest.subsidy, Satoshi::new(312_500_000));
        assert_eq!(digest.fees(), Satoshi::new(20_000));
        assert!(digest.size > 80 && digest.weight == 4 * digest.size as u64);
        assert_eq!(digest.hash, DigestFixture::block().header.hash().to_display_hex());
    }

    #[test]
    fn the_subsidy_halves_every_210_000_blocks_and_ends() {
        assert_eq!(BlockDigest::subsidy_at(BlockHeight::new(0)), Satoshi::new(5_000_000_000));
        assert_eq!(BlockDigest::subsidy_at(BlockHeight::new(629_999)), Satoshi::new(1_250_000_000));
        assert_eq!(BlockDigest::subsidy_at(BlockHeight::new(840_000)), Satoshi::new(312_500_000));
        assert_eq!(BlockDigest::subsidy_at(BlockHeight::new(64 * 210_000)), Satoshi::new(0));
    }

    #[test]
    fn difficulty_is_one_at_the_easiest_target_and_grows_as_it_tightens() {
        assert!((BlockDigest::difficulty_of(CompactBits::new(0x1d00_ffff)) - 1.0).abs() < 1e-12);
        let mainnet = BlockDigest::difficulty_of(CompactBits::new(0x1702_8c61));
        assert!(mainnet > 1.0e14 && mainnet < 1.0e15, "{mainnet}");
        assert!(BlockDigest::difficulty_of(CompactBits::new(0)).is_nan());
    }

    #[test]
    fn the_json_carries_every_field() {
        let digest = BlockDigest::of(BlockHeight::new(840_000), &DigestFixture::block());
        let json: serde_json::Value = serde_json::from_str(&digest.to_json()).expect("json");

        assert_eq!(json["height"], 840_000);
        assert_eq!(json["transactions"], 3);
        assert_eq!(json["fees"], 20_000);
        assert_eq!(json["hash"].as_str().map(str::len), Some(64));
        assert!((json["difficulty"].as_f64().expect("difficulty") - 1.0).abs() < 1e-9);
    }
}
