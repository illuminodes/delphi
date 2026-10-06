/// Why [`crate::OutputFilter`] took or turned away one transaction. A transaction that breaks
/// several rules gets the first one in [`Self::ALL`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionVerdict {
    Accepted,
    Coinbase,
    TooManyInputs,
    NotTwoOutputs,
    OpReturn,
    OversizedWitness,
    SpendsUnconfirmedParent,
    Replacement,
}

impl TransactionVerdict {
    pub const ALL: [Self; 8] = [
        Self::Accepted,
        Self::Coinbase,
        Self::TooManyInputs,
        Self::NotTwoOutputs,
        Self::OpReturn,
        Self::OversizedWitness,
        Self::SpendsUnconfirmedParent,
        Self::Replacement,
    ];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Coinbase => "coinbase",
            Self::TooManyInputs => "too_many_inputs",
            Self::NotTwoOutputs => "not_two_outputs",
            Self::OpReturn => "op_return",
            Self::OversizedWitness => "oversized_witness",
            Self::SpendsUnconfirmedParent => "spends_unconfirmed_parent",
            Self::Replacement => "replacement",
        }
    }

    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Accepted)
    }
}

#[cfg(test)]
mod tests {
    use super::TransactionVerdict;

    #[test]
    fn every_verdict_has_its_own_key() {
        let mut keys: Vec<&str> = TransactionVerdict::ALL.iter().map(|verdict| verdict.key()).collect();
        keys.sort_unstable();
        keys.dedup();

        assert_eq!(keys.len(), TransactionVerdict::ALL.len());
        assert!(TransactionVerdict::ALL.iter().filter(|verdict| verdict.is_accepted()).count() == 1);
    }
}
