use std::collections::BTreeMap;

use serde::Deserialize;

/// How the oracle's filter treated one block: the verdict on every transaction, how many outputs
/// the accepted ones carried and kept, and, measured against the hourly price, how many of those
/// sat near a round dollar amount and how many agreed with the price.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BlockFilter {
    #[serde(default)]
    verdicts: BTreeMap<String, usize>,
    #[serde(default)]
    pub accepted_outputs: usize,
    #[serde(default)]
    pub admitted_outputs: usize,
    pub price: Option<f64>,
    pub near_round_usd: Option<usize>,
    pub inside_band: Option<usize>,
}

impl BlockFilter {
    pub const ACCEPTED: &'static str = "accepted";
    const REJECTIONS: [(&'static str, &'static str, &'static str); 7] = [
        ("coinbase", "coinbase", "the miner's reward transaction"),
        ("not_two_outputs", "not 2 outputs", "a payment plus change has exactly two outputs; batches, consolidations and sweeps do not"),
        ("too_many_inputs", "> 5 inputs", "more than five inputs looks like consolidation, not a purchase"),
        ("op_return", "OP_RETURN", "carries data (runes, inscriptions' commitments, protocol markers)"),
        ("oversized_witness", "big witness", "a witness item or input witness over 500 bytes, typical of inscriptions and complex scripts"),
        ("spends_unconfirmed_parent", "chained", "spends an output created earlier in the same block, so it is a relay or a change hop"),
        ("replacement", "double spend", "spends an outpoint another transaction in the block already spent"),
    ];

    #[must_use]
    pub fn accepted(&self) -> usize {
        self.count(Self::ACCEPTED)
    }

    #[must_use]
    pub fn total(&self) -> usize {
        self.verdicts.values().sum()
    }

    #[must_use]
    pub fn count(&self, key: &str) -> usize {
        self.verdicts.get(key).copied().unwrap_or(0)
    }

    /// Every rejection rule that turned at least one transaction away, largest first, as
    /// `(label, count, explanation)`.
    #[must_use]
    pub fn rejections(&self) -> Vec<(&'static str, usize, &'static str)> {
        let mut rows: Vec<_> = Self::REJECTIONS
            .iter()
            .map(|(key, label, hint)| (*label, self.count(key), *hint))
            .filter(|(_, count, _)| *count > 0)
            .collect();
        rows.sort_by_key(|(_, count, _)| std::cmp::Reverse(*count));
        rows
    }

    /// The label and explanation of one rejection rule, by its verdict key.
    #[must_use]
    pub fn rejection(key: &str) -> Option<(&'static str, &'static str)> {
        Self::REJECTIONS
            .iter()
            .find(|(rule, _, _)| *rule == key)
            .map(|(_, label, hint)| (*label, *hint))
    }

    #[must_use]
    pub fn has_verdicts(&self) -> bool {
        self.total() > 0
    }

    #[must_use]
    pub fn share(part: usize, whole: usize) -> f64 {
        if whole == 0 { 0.0 } else { part as f64 / whole as f64 }
    }
}

#[cfg(test)]
mod tests {
    use super::BlockFilter;

    struct FilterFixture;

    impl FilterFixture {
        fn filter() -> BlockFilter {
            serde_json::from_str(
                r#"{"verdicts":{"accepted":1200,"coinbase":1,"not_two_outputs":1500,"too_many_inputs":80,"op_return":200,"oversized_witness":19,"spends_unconfirmed_parent":0,"replacement":0},"accepted_outputs":2400,"admitted_outputs":2100,"price":80000.0,"near_round_usd":700,"inside_band":250}"#,
            )
            .expect("filter")
        }
    }

    #[test]
    fn rejections_skip_empty_rules_and_list_the_largest_first() {
        let filter = FilterFixture::filter();
        let labels: Vec<&str> = filter.rejections().iter().map(|(label, _, _)| *label).collect();

        assert_eq!(labels, vec!["not 2 outputs", "OP_RETURN", "> 5 inputs", "big witness", "coinbase"]);
        assert_eq!(filter.accepted(), 1_200);
        assert_eq!(filter.total(), 3_000);
        assert_eq!(filter.inside_band, Some(250));
    }

    #[test]
    fn a_rule_is_found_by_its_key_and_acceptance_is_no_rule() {
        assert_eq!(BlockFilter::rejection("too_many_inputs").map(|(label, _)| label), Some("> 5 inputs"));
        assert!(BlockFilter::rejection(BlockFilter::ACCEPTED).is_none());
    }

    #[test]
    fn a_filter_from_an_old_daemon_is_empty() {
        let filter: BlockFilter = serde_json::from_str("{}").expect("filter");

        assert!(!filter.has_verdicts());
        assert!(filter.near_round_usd.is_none());
        assert!((BlockFilter::share(1, 0)).abs() < f64::EPSILON);
        assert!((BlockFilter::share(1, 4) - 0.25).abs() < f64::EPSILON);
    }
}
