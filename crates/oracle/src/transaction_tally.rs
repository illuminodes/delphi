use crate::TransactionVerdict;

/// How many transactions of one block got each [`TransactionVerdict`], and how many outputs the
/// accepted ones carried before and after the amount limits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransactionTally {
    counts: [usize; TransactionVerdict::ALL.len()],
    accepted_outputs: usize,
    admitted_outputs: usize,
}

impl TransactionTally {
    pub fn record(&mut self, verdict: TransactionVerdict, outputs: usize, admitted: usize) {
        self.counts[Self::slot(verdict)] += 1;
        if verdict.is_accepted() {
            self.accepted_outputs += outputs;
            self.admitted_outputs += admitted;
        }
    }

    #[must_use]
    pub fn count(&self, verdict: TransactionVerdict) -> usize {
        self.counts[Self::slot(verdict)]
    }

    #[must_use]
    pub fn total(&self) -> usize {
        self.counts.iter().sum()
    }

    /// Outputs carried by accepted transactions.
    #[must_use]
    pub const fn accepted_outputs(&self) -> usize {
        self.accepted_outputs
    }

    /// Accepted outputs whose amount lies within the filter's limits.
    #[must_use]
    pub const fn admitted_outputs(&self) -> usize {
        self.admitted_outputs
    }

    fn slot(verdict: TransactionVerdict) -> usize {
        TransactionVerdict::ALL
            .iter()
            .position(|held| *held == verdict)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::TransactionTally;
    use crate::TransactionVerdict;

    #[test]
    fn verdicts_are_counted_and_only_accepted_outputs_add_up() {
        let mut tally = TransactionTally::default();
        tally.record(TransactionVerdict::Accepted, 2, 1);
        tally.record(TransactionVerdict::Accepted, 2, 2);
        tally.record(TransactionVerdict::OpReturn, 3, 0);
        tally.record(TransactionVerdict::Coinbase, 1, 0);

        assert_eq!(tally.count(TransactionVerdict::Accepted), 2);
        assert_eq!(tally.count(TransactionVerdict::OpReturn), 1);
        assert_eq!(tally.count(TransactionVerdict::Replacement), 0);
        assert_eq!(tally.total(), 4);
        assert_eq!(tally.accepted_outputs(), 4);
        assert_eq!(tally.admitted_outputs(), 3);
    }
}
