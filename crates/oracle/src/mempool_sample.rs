use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::tx::Transaction;

use crate::{AdmittedSample, OutputFilter};

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
        }
    }

    /// Filter one transaction, keeping whatever outputs clear, and report how
    /// many were kept.
    pub fn absorb(&mut self, transaction: &Transaction, at: BlockTime) -> usize {
        self.transactions += 1;
        let admitted: Vec<_> = self.filter.admit(transaction).collect();
        for amount in &admitted {
            self.sample.push(*amount, self.height, at);
        }
        if !admitted.is_empty() {
            self.admitted_transactions += 1;
        }
        admitted.len()
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
