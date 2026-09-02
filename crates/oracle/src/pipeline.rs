use std::fmt;

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::tx::Transaction;

use crate::{AdmittedSample, OracleReport, OutputFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleError {
    NoTransactions,
    EmptyHistogram,
    RoughPriceUnavailable,
    CentralPriceUnavailable,
}

impl fmt::Display for OracleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTransactions => formatter.write_str("no transactions were processed"),
            Self::EmptyHistogram => formatter.write_str("no qualifying histogram outputs"),
            Self::RoughPriceUnavailable => {
                formatter.write_str("rough price could not be estimated")
            }
            Self::CentralPriceUnavailable => {
                formatter.write_str("central price could not be found")
            }
        }
    }
}

impl std::error::Error for OracleError {}

#[derive(Debug, Default)]
pub struct OraclePipeline {
    filter: OutputFilter,
    admitted: AdmittedSample,
}

impl OraclePipeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, height: BlockHeight, time: BlockTime, transaction: &Transaction) {
        for amount in self.filter.admit(transaction) {
            self.admitted.push(amount, height, time);
        }
    }

    pub fn process_all<'a>(
        &mut self,
        transactions: impl IntoIterator<Item = (BlockHeight, BlockTime, &'a Transaction)>,
    ) {
        for (height, time, transaction) in transactions {
            self.process(height, time, transaction);
        }
    }

    #[must_use]
    pub fn sample(&self) -> &AdmittedSample {
        &self.admitted
    }

    #[must_use]
    pub fn into_sample(self) -> AdmittedSample {
        self.admitted
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn finish(self) -> Result<OracleReport, OracleError> {
        self.admitted.price()
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime, Locktime, Sequence, TxVersion};
    use mantle::hash::Hash256;
    use mantle::script::{ScriptPubKey, ScriptSig, Witness};
    use mantle::tx::{OutPoint, Transaction, TxIn, TxOut};

    use super::{OracleError, OraclePipeline};

    struct PipelineFixture;

    impl PipelineFixture {
        fn transaction(id: u8, amount: u64) -> Transaction {
            Transaction {
                version: TxVersion::new(2),
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes([id; 32]),
                        vout: 0,
                    },
                    script_sig: ScriptSig::from_slice(&[]),
                    sequence: Sequence::new(u32::MAX),
                    witness: Witness::empty(),
                }],
                outputs: vec![
                    TxOut {
                        value: Satoshi::new(amount),
                        script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                    },
                    TxOut {
                        value: Satoshi::new(amount + 37),
                        script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                    },
                ],
                locktime: Locktime::new(0),
            }
        }
    }

    #[test]
    fn runs_end_to_end_over_transaction_stream() {
        let transactions: Vec<_> = (1_u8..100)
            .map(|id| PipelineFixture::transaction(id, 99_000 + u64::from(id) * 30))
            .collect();
        let mut pipeline = OraclePipeline::new();
        pipeline.process_all(transactions.iter().enumerate().map(|(index, transaction)| {
            (
                BlockHeight::new(800_000 + u32::try_from(index).expect("index fits u32")),
                BlockTime::new(1_700_000_000 + u32::try_from(index).expect("index fits u32")),
                transaction,
            )
        }));

        let report = pipeline.finish().expect("synthetic signal produces report");

        assert!(report.central_price > 90_000.0);
        assert!(report.central_price < 110_000.0);
        assert!(report.sample_count > 0);
        assert_eq!(report.start_height, BlockHeight::new(800_000));
        assert_eq!(report.stop_height, BlockHeight::new(800_098));
    }

    #[test]
    fn rejects_an_empty_stream() {
        assert_eq!(
            OraclePipeline::new().finish(),
            Err(OracleError::NoTransactions)
        );
    }
}
