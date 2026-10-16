use serde::Deserialize;

use crate::BlockFilter;

/// One block as the daemon read it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChainBlock {
    pub height: u32,
    pub hash: String,
    #[serde(default)]
    pub previous: String,
    pub time: i64,
    #[serde(default)]
    pub version: i32,
    pub difficulty: Option<f64>,
    pub transactions: usize,
    #[serde(default)]
    pub outputs: usize,
    #[serde(default)]
    pub size: usize,
    #[serde(default)]
    pub weight: u64,
    #[serde(default)]
    pub output_value: u64,
    #[serde(default)]
    pub coinbase_value: u64,
    #[serde(default)]
    pub subsidy: u64,
    #[serde(default)]
    pub fees: u64,
    #[serde(default)]
    pub filter: BlockFilter,
}

impl ChainBlock {
    pub const MAX_WEIGHT: u64 = 4_000_000;
    const SATS_PER_BTC: f64 = 100_000_000.0;

    /// How full the block is, as a share of the consensus weight limit.
    #[must_use]
    pub fn fullness(&self) -> f64 {
        self.weight as f64 / Self::MAX_WEIGHT as f64
    }

    /// Average fee rate in sat/vB, leaving out the coinbase transaction.
    #[must_use]
    pub fn fee_rate(&self) -> Option<f64> {
        let virtual_size = self.weight.div_ceil(4);
        (virtual_size > 0 && self.transactions > 1).then(|| self.fees as f64 / virtual_size as f64)
    }

    #[must_use]
    pub fn btc(sats: u64) -> f64 {
        sats as f64 / Self::SATS_PER_BTC
    }

    /// The hash with its leading zeros counted and the rest elided, e.g. `19×0 1a2b3c4d…c4d5e6f7`.
    #[must_use]
    pub fn short_hash(&self) -> String {
        let zeros = self.hash.chars().take_while(|character| *character == '0').count();
        let rest = &self.hash[zeros..];
        let tail = rest.len().saturating_sub(8);
        if rest.len() <= 16 {
            return self.hash.clone();
        }
        format!("{zeros}×0 {}…{}", &rest[..8], &rest[tail..])
    }
}

#[cfg(test)]
mod tests {
    use super::ChainBlock;

    struct BlockFixture;

    impl BlockFixture {
        fn block() -> ChainBlock {
            serde_json::from_str(
                r#"{"height":970000,"hash":"00000000000000000001a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7","previous":"00","time":1780000000,"version":536870912,"difficulty":1.2e14,"transactions":3001,"outputs":9000,"size":1600000,"weight":3996000,"output_value":500000000000,"coinbase_value":332500000,"subsidy":312500000,"fees":20000000}"#,
            )
            .expect("block")
        }
    }

    #[test]
    fn fullness_and_fee_rate_come_from_weight_and_fees() {
        let block = BlockFixture::block();

        assert!((block.fullness() - 0.999).abs() < 1e-9);
        assert!((block.fee_rate().expect("rate") - 20_000_000.0 / 999_000.0).abs() < 1e-9);
        assert!((ChainBlock::btc(block.fees) - 0.2).abs() < 1e-12);
    }

    #[test]
    fn a_block_with_only_a_coinbase_has_no_fee_rate() {
        let mut block = BlockFixture::block();
        block.transactions = 1;

        assert!(block.fee_rate().is_none());
    }

    #[test]
    fn the_short_hash_counts_the_leading_zeros() {
        assert_eq!(BlockFixture::block().short_hash(), "19×0 1a2b3c4d…c4d5e6f7");
    }

    #[test]
    fn fields_the_daemon_did_not_send_default_to_zero() {
        let block: ChainBlock =
            serde_json::from_str(r#"{"height":1,"hash":"ab","time":2,"difficulty":null,"transactions":1}"#).expect("block");

        assert_eq!(block.weight, 0);
        assert!(block.difficulty.is_none());
        assert_eq!(block.short_hash(), "ab");
    }
}
