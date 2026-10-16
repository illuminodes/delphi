use std::collections::VecDeque;

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::tx::Transaction;

use mantle::amount::Satoshi;

use crate::{AdmittedSample, OutputFilter, TransactionVerdict};

/// Accumulates admitted outputs from unconfirmed transactions.
///
/// Mempool transactions carry no height or time of their own, so each output
/// is stamped with the chain tip the collector was told about and its local
/// arrival time. Pricing reads neither field — [`crate::CentralPriceFinder`]
/// works from amounts alone — so the stamps only shape the reported bounds.
pub struct MempoolCollector {
    filter: OutputFilter,
    sample: AdmittedSample,
    height: BlockHeight,
    transactions: u64,
    admitted_transactions: u64,
    window: Option<usize>,
    windowed_outputs: VecDeque<usize>,
}

impl MempoolCollector {
    #[must_use]
    pub fn new(height: BlockHeight) -> Self {
        Self {
            filter: OutputFilter::new(),
            sample: AdmittedSample::new(),
            height,
            transactions: 0,
            admitted_transactions: 0,
            window: None,
            windowed_outputs: VecDeque::new(),
        }
    }

    /// Keep only the outputs of the most recent `transactions` admitted
    /// transactions.
    #[must_use]
    pub fn with_window(height: BlockHeight, transactions: usize) -> Self {
        Self {
            window: Some(transactions),
            ..Self::new(height)
        }
    }

    #[must_use]
    pub const fn window(&self) -> Option<usize> {
        self.window
    }

    #[must_use]
    pub fn windowed_transactions(&self) -> usize {
        self.windowed_outputs.len()
    }

    /// Filter one transaction, keeping whatever outputs clear, and report how
    /// many were kept.
    pub fn absorb(&mut self, transaction: &Transaction, at: BlockTime) -> usize {
        self.screen(transaction, at).1.len()
    }

    /// [`Self::absorb`], returning the filter's verdict and the outputs kept.
    pub fn screen(&mut self, transaction: &Transaction, at: BlockTime) -> (TransactionVerdict, Vec<Satoshi>) {
        self.transactions += 1;
        let (verdict, admitted) = self.filter.screen(transaction);
        self.sample.push_group(&admitted, self.height, at);
        if !admitted.is_empty() {
            self.admitted_transactions += 1;
            self.windowed_outputs.push_back(admitted.len());
            self.trim_to_window();
        }
        (verdict, admitted)
    }

    fn trim_to_window(&mut self) {
        let Some(window) = self.window else {
            return;
        };
        let mut evicted = 0;
        while self.windowed_outputs.len() > window {
            evicted += self.windowed_outputs.pop_front().unwrap_or_default();
        }
        if evicted > 0 {
            self.sample.evict_oldest(evicted);
        }
    }

    #[must_use]
    pub const fn sample(&self) -> &AdmittedSample {
        &self.sample
    }

    #[must_use]
    pub fn into_sample(self) -> AdmittedSample {
        self.sample
    }

    #[must_use]
    pub const fn transactions(&self) -> u64 {
        self.transactions
    }

    #[must_use]
    pub const fn admitted_transactions(&self) -> u64 {
        self.admitted_transactions
    }

    /// Share of seen transactions that contributed at least one output.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn admission_rate(&self) -> f64 {
        if self.transactions == 0 {
            return 0.0;
        }
        self.admitted_transactions as f64 / self.transactions as f64
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime, Locktime, Sequence, TxVersion};
    use mantle::hash::Hash256;
    use mantle::script::{ScriptPubKey, ScriptSig, Witness};
    use mantle::tx::{OutPoint, Transaction, TxIn, TxOut};

    use super::MempoolCollector;

    struct TransactionFixture;

    impl TransactionFixture {
        fn spending(previous_txid: Hash256, first: u64, second: u64) -> Transaction {
            Transaction {
                version: TxVersion::new(2),
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: previous_txid,
                        vout: 0,
                    },
                    script_sig: ScriptSig::from_slice(&[]),
                    sequence: Sequence::new(u32::MAX),
                    witness: Witness::empty(),
                }],
                outputs: vec![
                    TxOut {
                        value: Satoshi::new(first),
                        script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                    },
                    TxOut {
                        value: Satoshi::new(second),
                        script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                    },
                ],
                locktime: Locktime::new(0),
            }
        }
    }

    fn collector() -> MempoolCollector {
        MempoolCollector::new(BlockHeight::new(900_000))
    }

    #[test]
    fn a_fresh_collector_holds_nothing() {
        let collector = collector();
        assert!(collector.sample().is_empty());
        assert_eq!(collector.transactions(), 0);
        assert!((collector.admission_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn absorbing_keeps_the_admitted_outputs() {
        let mut collector = collector();
        let transaction = TransactionFixture::spending(Hash256::from_bytes([1; 32]), 50_000, 70_000);

        assert_eq!(
            collector.absorb(&transaction, BlockTime::new(1_700_000_000)),
            2
        );
        assert_eq!(collector.sample().len(), 2);
        assert_eq!(collector.admitted_transactions(), 1);
    }

    #[test]
    fn outputs_carry_the_arrival_time_and_configured_height() {
        let mut collector = collector();
        let transaction = TransactionFixture::spending(Hash256::from_bytes([2; 32]), 50_000, 70_000);
        collector.absorb(&transaction, BlockTime::new(1_700_000_123));

        let output = collector.sample().outputs()[0];
        assert_eq!(output.height, BlockHeight::new(900_000));
        assert_eq!(output.time, BlockTime::new(1_700_000_123));
    }

    #[test]
    fn a_replacement_spending_a_seen_outpoint_adds_nothing() {
        let mut collector = collector();
        let original = TransactionFixture::spending(Hash256::from_bytes([9; 32]), 50_000, 70_000);
        let replacement =
            TransactionFixture::spending(Hash256::from_bytes([9; 32]), 49_000, 70_000);

        assert_eq!(collector.absorb(&original, BlockTime::new(1_700_000_000)), 2);
        assert_eq!(
            collector.absorb(&replacement, BlockTime::new(1_700_000_001)),
            0
        );
        assert_eq!(collector.sample().len(), 2);
    }

    #[test]
    fn a_window_keeps_only_the_latest_admitted_transactions() {
        let mut collector = MempoolCollector::with_window(BlockHeight::new(900_000), 2);
        for id in 1_u8..=4 {
            let transaction = TransactionFixture::spending(
                Hash256::from_bytes([id; 32]),
                10_000 * u64::from(id),
                10_000 * u64::from(id) + 1,
            );
            collector.absorb(&transaction, BlockTime::new(1_700_000_000 + u32::from(id)));
        }

        assert_eq!(collector.windowed_transactions(), 2);
        assert_eq!(collector.sample().len(), 4);
        assert_eq!(collector.sample().outputs()[0].amount.to_u64(), 30_000);
        assert_eq!(collector.admitted_transactions(), 4);
    }

    #[test]
    fn rejected_transactions_do_not_consume_the_window() {
        let mut collector = MempoolCollector::with_window(BlockHeight::new(900_000), 1);
        let admitted = TransactionFixture::spending(Hash256::from_bytes([5; 32]), 50_000, 70_000);
        let mut rejected =
            TransactionFixture::spending(Hash256::from_bytes([6; 32]), 50_000, 70_000);
        rejected.outputs[0].script_pubkey = ScriptPubKey::from_slice(&[0x6a]);

        collector.absorb(&admitted, BlockTime::new(1_700_000_000));
        collector.absorb(&rejected, BlockTime::new(1_700_000_001));

        assert_eq!(collector.sample().len(), 2);
    }

    #[test]
    fn an_unwindowed_collector_keeps_everything() {
        let mut collector = collector();
        for id in 1_u8..=5 {
            let transaction = TransactionFixture::spending(
                Hash256::from_bytes([id; 32]),
                10_000 * u64::from(id),
                10_000 * u64::from(id) + 1,
            );
            collector.absorb(&transaction, BlockTime::new(1_700_000_000));
        }
        assert_eq!(collector.sample().len(), 10);
        assert_eq!(collector.window(), None);
    }

    #[test]
    fn the_admission_rate_counts_transactions_not_outputs() {
        let mut collector = collector();
        let admitted = TransactionFixture::spending(Hash256::from_bytes([3; 32]), 50_000, 70_000);
        let mut rejected =
            TransactionFixture::spending(Hash256::from_bytes([4; 32]), 50_000, 70_000);
        rejected.outputs[0].script_pubkey = ScriptPubKey::from_slice(&[0x6a]);

        collector.absorb(&admitted, BlockTime::new(1_700_000_000));
        collector.absorb(&rejected, BlockTime::new(1_700_000_001));

        assert_eq!(collector.transactions(), 2);
        assert_eq!(collector.admitted_transactions(), 1);
        assert!((collector.admission_rate() - 0.5).abs() < f64::EPSILON);
    }
}
