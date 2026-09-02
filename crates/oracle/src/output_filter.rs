use std::collections::HashSet;

use mantle::amount::Satoshi;
use mantle::hash::Hash256;
use mantle::tx::{Transaction, TxHash};

#[derive(Debug, Default)]
pub struct OutputFilter {
    seen_txids: HashSet<Hash256>,
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
        let same_day_spend = transaction
            .inputs
            .iter()
            .any(|input| self.seen_txids.contains(&input.previous_output.txid));
        let accepted = transaction.inputs.len() <= Self::MAX_INPUTS
            && transaction.outputs.len() == Self::REQUIRED_OUTPUTS
            && !Self::is_coinbase(transaction)
            && !Self::has_op_return(transaction)
            && !Self::has_oversized_witness(transaction)
            && !same_day_spend;
        let outputs = if accepted {
            transaction
                .outputs
                .iter()
                .map(|output| output.value)
                .filter(|amount| *amount > Self::MIN_OUTPUT && *amount < Self::MAX_OUTPUT)
                .collect()
        } else {
            Vec::new()
        };
        if let Ok(txid) = transaction.txid_into(&mut self.txid_buffer) {
            self.seen_txids.insert(txid);
        }
        outputs.into_iter()
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
    fn rejects_op_return_and_oversized_witness() {
        let mut filter = OutputFilter::new();
        let mut op_return = TransactionFixture::transaction(Hash256::from_bytes([1; 32]));
        op_return.outputs[0].script_pubkey = ScriptPubKey::from_slice(&[0x6a]);
        let mut oversized = TransactionFixture::transaction(Hash256::from_bytes([2; 32]));
        oversized.inputs[0].witness = Witness::from_slice(&[&[0; 501]]);

        assert_eq!(filter.admit(&op_return).count(), 0);
        assert_eq!(filter.admit(&oversized).count(), 0);
    }
}
