use std::collections::HashSet;

use mantle::amount::Satoshi;
use mantle::hash::Hash256;
use mantle::tx::{OutPoint, Transaction, TxHash};

use crate::TransactionVerdict;

#[derive(Debug, Default)]
pub struct OutputFilter {
    seen_txids: HashSet<Hash256>,
    spent_outpoints: HashSet<OutPoint>,
    txid_buffer: Vec<u8>,
}

impl OutputFilter {
    pub const MIN_OUTPUT: Satoshi = Satoshi::new(1_000);
    pub const MAX_OUTPUT: Satoshi = Satoshi::new(10_000_000_000_000);
    pub const MAX_INPUTS: usize = 5;
    pub const REQUIRED_OUTPUTS: usize = 2;
    pub const MAX_WITNESS_BYTES: usize = 500;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn admit(&mut self, transaction: &Transaction) -> impl Iterator<Item = Satoshi> + use<> {
        self.screen(transaction).1.into_iter()
    }

    /// The verdict on `transaction` with the outputs it contributes, which are none unless it
    /// is accepted. The transaction is remembered either way, so later spends of it or of its
    /// inputs are caught.
    pub fn screen(&mut self, transaction: &Transaction) -> (TransactionVerdict, Vec<Satoshi>) {
        let verdict = self.verdict(transaction);
        let outputs = if verdict.is_accepted() {
            transaction
                .outputs
                .iter()
                .map(|output| output.value)
                .filter(|amount| *amount > Self::MIN_OUTPUT && *amount < Self::MAX_OUTPUT)
                .collect()
        } else {
            Vec::new()
        };
        self.remember(transaction);
        (verdict, outputs)
    }

    fn verdict(&self, transaction: &Transaction) -> TransactionVerdict {
        if Self::is_coinbase(transaction) {
            TransactionVerdict::Coinbase
        } else if transaction.inputs.len() > Self::MAX_INPUTS {
            TransactionVerdict::TooManyInputs
        } else if transaction.outputs.len() != Self::REQUIRED_OUTPUTS {
            TransactionVerdict::NotTwoOutputs
        } else if Self::has_op_return(transaction) {
            TransactionVerdict::OpReturn
        } else if Self::has_oversized_witness(transaction) {
            TransactionVerdict::OversizedWitness
        } else if transaction
            .inputs
            .iter()
            .any(|input| self.seen_txids.contains(&input.previous_output.txid))
        {
            TransactionVerdict::SpendsUnconfirmedParent
        } else if transaction
            .inputs
            .iter()
            .any(|input| self.spent_outpoints.contains(&input.previous_output))
        {
            TransactionVerdict::Replacement
        } else {
            TransactionVerdict::Accepted
        }
    }

    fn remember(&mut self, transaction: &Transaction) {
        if let Ok(txid) = transaction.txid_into(&mut self.txid_buffer) {
            self.seen_txids.insert(txid);
        }
        if !Self::is_coinbase(transaction) {
            for input in &transaction.inputs {
                self.spent_outpoints.insert(input.previous_output);
            }
        }
    }

    #[must_use]
    pub fn distinct_spends(&self) -> usize {
        self.spent_outpoints.len()
    }

    fn is_coinbase(transaction: &Transaction) -> bool {
        transaction.inputs.iter().any(|input| {
            input.previous_output.txid == Hash256::ZERO && input.previous_output.vout == u32::MAX
        })
    }

    fn has_op_return(transaction: &Transaction) -> bool {
        transaction
            .outputs
            .iter()
            .any(|output| output.script_pubkey.first() == Some(&0x6a))
    }

    fn has_oversized_witness(transaction: &Transaction) -> bool {
        transaction.inputs.iter().any(|input| {
            let mut total = 0;
            input.witness.iter().any(|item| {
                total += item.len();
                item.len() > Self::MAX_WITNESS_BYTES || total > Self::MAX_WITNESS_BYTES
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{Locktime, Sequence, TxVersion};
    use mantle::hash::Hash256;
    use mantle::script::{ScriptPubKey, ScriptSig, Witness};
    use mantle::tx::{OutPoint, Transaction, TxHash, TxIn, TxOut};

    use super::OutputFilter;
    use crate::TransactionVerdict;

    struct TransactionFixture;

    impl TransactionFixture {
        fn transaction(previous_txid: Hash256) -> Transaction {
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
                        value: Satoshi::new(50_000),
                        script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                    },
                    TxOut {
                        value: Satoshi::new(100_000),
                        script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                    },
                ],
                locktime: Locktime::new(0),
            }
        }
    }

    #[test]
    fn admits_outputs_from_matching_transactions() {
        let mut filter = OutputFilter::new();
        let transaction = TransactionFixture::transaction(Hash256::from_bytes([1; 32]));

        assert_eq!(
            filter.admit(&transaction).collect::<Vec<_>>(),
            vec![Satoshi::new(50_000), Satoshi::new(100_000)]
        );
    }

    #[test]
    fn rejects_same_day_spends_after_recording_parent() {
        let mut filter = OutputFilter::new();
        let parent = TransactionFixture::transaction(Hash256::from_bytes([1; 32]));
        let parent_txid = parent.txid().expect("fixture encodes");
        let child = TransactionFixture::transaction(parent_txid);

        assert_eq!(filter.admit(&parent).count(), 2);
        assert_eq!(filter.admit(&child).count(), 0);
    }

    #[test]
    fn rejects_a_replacement_spending_an_already_seen_outpoint() {
        let mut filter = OutputFilter::new();
        let original = TransactionFixture::transaction(Hash256::from_bytes([9; 32]));
        let mut replacement = TransactionFixture::transaction(Hash256::from_bytes([9; 32]));
        replacement.outputs[0].value = Satoshi::new(49_000);

        assert_eq!(filter.admit(&original).count(), 2);
        assert_ne!(
            original.txid().expect("fixture encodes"),
            replacement.txid().expect("fixture encodes")
        );
        assert_eq!(filter.admit(&replacement).count(), 0);
    }

    #[test]
    fn counts_distinct_spent_outpoints() {
        let mut filter = OutputFilter::new();
        let _ = filter
            .admit(&TransactionFixture::transaction(Hash256::from_bytes([1; 32])))
            .count();
        let _ = filter
            .admit(&TransactionFixture::transaction(Hash256::from_bytes([2; 32])))
            .count();

        assert_eq!(filter.distinct_spends(), 2);
    }

    #[test]
    fn rejects_op_return_and_oversized_witness() {
        let mut filter = OutputFilter::new();
        let mut op_return = TransactionFixture::transaction(Hash256::from_bytes([1; 32]));
        op_return.outputs[0].script_pubkey = ScriptPubKey::from_slice(&[0x6a]);
        let mut oversized = TransactionFixture::transaction(Hash256::from_bytes([2; 32]));
        oversized.inputs[0].witness = Witness::from_slice(&[&[0; 501]]);

        assert_eq!(filter.admit(&op_return).count(), 0);
        assert_eq!(filter.admit(&oversized).count(), 0);
    }

    #[test]
    fn each_rejection_names_its_rule() {
        let mut filter = OutputFilter::new();
        let parent = TransactionFixture::transaction(Hash256::from_bytes([1; 32]));
        let child = TransactionFixture::transaction(parent.txid().expect("fixture encodes"));
        let mut three_outputs = TransactionFixture::transaction(Hash256::from_bytes([3; 32]));
        three_outputs.outputs.push(three_outputs.outputs[0].clone());
        let mut wide = TransactionFixture::transaction(Hash256::from_bytes([4; 32]));
        wide.inputs = vec![wide.inputs[0].clone(); 6];
        let mut coinbase = TransactionFixture::transaction(Hash256::ZERO);
        coinbase.inputs[0].previous_output.vout = u32::MAX;
        let replacement = TransactionFixture::transaction(Hash256::from_bytes([1; 32]));

        assert_eq!(filter.screen(&parent).0, TransactionVerdict::Accepted);
        assert_eq!(filter.screen(&child).0, TransactionVerdict::SpendsUnconfirmedParent);
        assert_eq!(filter.screen(&three_outputs).0, TransactionVerdict::NotTwoOutputs);
        assert_eq!(filter.screen(&wide).0, TransactionVerdict::TooManyInputs);
        assert_eq!(filter.screen(&coinbase).0, TransactionVerdict::Coinbase);
        let (verdict, outputs) = filter.screen(&replacement);
        assert_eq!(verdict, TransactionVerdict::Replacement);
        assert!(outputs.is_empty());
    }

    #[test]
    fn an_accepted_transaction_drops_outputs_outside_the_amount_limits() {
        let mut filter = OutputFilter::new();
        let mut dust = TransactionFixture::transaction(Hash256::from_bytes([5; 32]));
        dust.outputs[0].value = OutputFilter::MIN_OUTPUT;
        let (verdict, outputs) = filter.screen(&dust);

        assert_eq!(verdict, TransactionVerdict::Accepted);
        assert_eq!(outputs, vec![Satoshi::new(100_000)]);
    }
}
