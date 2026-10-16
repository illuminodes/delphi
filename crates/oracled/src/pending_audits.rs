use std::collections::HashMap;

use mantle::difficulty::BlockHeight;
use mantle::hash::Hash256;

use crate::ChainWindow;

/// One audit note handed to the relay, keyed by its id, until the relay
/// says whether it took it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingAudit {
    pub window: ChainWindow,
    pub height: BlockHeight,
    pub hash: Hash256,
}

/// Audit notes waiting for the relay's `OK`.
///
/// Acks lost to a reconnect would leave entries behind forever, so past
/// [`Self::LIMIT`] the oldest waits are forgotten; those audits are not in
/// the ledger and go out again after a restart.
#[derive(Debug, Default)]
pub struct PendingAudits {
    waiting: HashMap<String, (u64, PendingAudit)>,
    sequence: u64,
}

impl PendingAudits {
    pub const LIMIT: usize = 4_096;

    pub fn insert(&mut self, id: String, audit: PendingAudit) {
        self.sequence += 1;
        self.waiting.insert(id, (self.sequence, audit));
        if self.waiting.len() > Self::LIMIT {
            let cutoff = self.sequence - (Self::LIMIT / 2) as u64;
            self.waiting.retain(|_, (sequence, _)| *sequence > cutoff);
        }
    }

    pub fn take(&mut self, id: &str) -> Option<PendingAudit> {
        self.waiting.remove(id).map(|(_, audit)| audit)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockHeight;
    use mantle::hash::Hash256;

    use super::{PendingAudit, PendingAudits};
    use crate::ChainWindow;

    fn audit(height: u32) -> PendingAudit {
        PendingAudit {
            window: ChainWindow::Hourly,
            height: BlockHeight::new(height),
            hash: Hash256::hash(&height.to_le_bytes()),
        }
    }

    #[test]
    fn an_audit_is_taken_once_by_its_id() {
        let mut pending = PendingAudits::default();
        pending.insert("a".to_string(), audit(1));

        assert_eq!(pending.take("a"), Some(audit(1)));
        assert_eq!(pending.take("a"), None);
        assert!(pending.is_empty());
    }

    #[test]
    fn past_the_limit_the_oldest_waits_are_forgotten() {
        let mut pending = PendingAudits::default();
        let total = u32::try_from(PendingAudits::LIMIT).expect("fits") + 1;
        for height in 0..total {
            pending.insert(height.to_string(), audit(height));
        }

        assert!(pending.len() <= PendingAudits::LIMIT);
        assert_eq!(pending.take("0"), None);
        assert_eq!(pending.take(&(total - 1).to_string()), Some(audit(total - 1)));
    }
}
