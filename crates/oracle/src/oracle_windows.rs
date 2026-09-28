use std::fmt;

use mantle::difficulty::BlockHeight;

use crate::{MempoolCollector, RangeSelection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleWindowsError {
    EmptyChainWindow,
    EmptyMempoolWindow,
}

impl fmt::Display for OracleWindowsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyChainWindow => formatter.write_str("chain window must cover at least one block"),
            Self::EmptyMempoolWindow => {
                formatter.write_str("mempool window must cover at least one transaction")
            }
        }
    }
}

impl std::error::Error for OracleWindowsError {}

/// How much history each price source looks at: the last `chain_blocks`
/// confirmed blocks, and the last `mempool_transactions` admitted mempool
/// transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OracleWindows {
    chain_blocks: usize,
    mempool_transactions: usize,
}

impl OracleWindows {
    pub const DEFAULT_CHAIN_BLOCKS: usize = 144;
    pub const DEFAULT_MEMPOOL_TRANSACTIONS: usize = 400;

    #[allow(clippy::missing_errors_doc)]
    pub const fn new(
        chain_blocks: usize,
        mempool_transactions: usize,
    ) -> Result<Self, OracleWindowsError> {
        if chain_blocks == 0 {
            return Err(OracleWindowsError::EmptyChainWindow);
        }
        if mempool_transactions == 0 {
            return Err(OracleWindowsError::EmptyMempoolWindow);
        }
        Ok(Self {
            chain_blocks,
            mempool_transactions,
        })
    }

    #[must_use]
    pub const fn chain_blocks(self) -> usize {
        self.chain_blocks
    }

    #[must_use]
    pub const fn mempool_transactions(self) -> usize {
        self.mempool_transactions
    }

    #[must_use]
    pub const fn chain_selection(self) -> RangeSelection {
        RangeSelection::RecentBlocks(self.chain_blocks)
    }

    #[must_use]
    pub fn mempool_collector(self, height: BlockHeight) -> MempoolCollector {
        MempoolCollector::with_window(height, self.mempool_transactions)
    }
}

impl Default for OracleWindows {
    fn default() -> Self {
        Self {
            chain_blocks: Self::DEFAULT_CHAIN_BLOCKS,
            mempool_transactions: Self::DEFAULT_MEMPOOL_TRANSACTIONS,
        }
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockHeight;

    use crate::RangeSelection;

    use super::{OracleWindows, OracleWindowsError};

    #[test]
    fn the_two_windows_are_set_independently() {
        let windows = OracleWindows::new(6, 250).expect("both windows are non-empty");
        assert_eq!(windows.chain_blocks(), 6);
        assert_eq!(windows.mempool_transactions(), 250);
    }

    #[test]
    fn a_zero_chain_window_is_rejected() {
        assert_eq!(
            OracleWindows::new(0, 10),
            Err(OracleWindowsError::EmptyChainWindow)
        );
    }

    #[test]
    fn a_zero_mempool_window_is_rejected() {
        assert_eq!(
            OracleWindows::new(10, 0),
            Err(OracleWindowsError::EmptyMempoolWindow)
        );
    }

    #[test]
    fn the_chain_window_selects_recent_blocks() {
        let windows = OracleWindows::new(12, 50).expect("valid");
        assert_eq!(windows.chain_selection(), RangeSelection::RecentBlocks(12));
    }

    #[test]
    fn the_mempool_window_configures_the_collector() {
        let windows = OracleWindows::new(12, 50).expect("valid");
        let collector = windows.mempool_collector(BlockHeight::new(1));
        assert_eq!(collector.window(), Some(50));
    }
}
