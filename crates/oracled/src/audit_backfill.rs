use std::collections::HashSet;

use mantle::difficulty::BlockHeight;
use mantle::hash::Hash256;

use crate::{AuditRecord, ChainPricer, ChainWindow, RecordLog};

/// Finds the audit records not yet published, newest first, so the tip goes
/// out before the history behind it.
///
/// It starts from the ledger's published set as it was at startup and adds
/// each record it hands out, so nothing is handed out twice. The hub, which
/// owns the ledger, decides whether a handed record replaces an earlier note.
pub struct AuditBackfill {
    pricer: ChainPricer,
    handed: HashSet<(ChainWindow, u32, Hash256)>,
}

impl AuditBackfill {
    #[must_use]
    pub fn new(published: HashSet<(ChainWindow, u32, Hash256)>) -> Self {
        Self {
            pricer: ChainPricer::new(),
            handed: published,
        }
    }

    /// Up to `limit` records still to publish.
    pub fn next_batch(&mut self, log: &RecordLog, limit: usize) -> Vec<AuditRecord> {
        let mut batch = Vec::new();
        let blocks: Vec<(BlockHeight, Hash256)> = log
            .records()
            .filter(|record| !record.lacks_hash())
            .map(|record| (record.height(), record.hash()))
            .collect();
        for (height, hash) in blocks.into_iter().rev() {
            for window in ChainWindow::ALL {
                if batch.len() >= limit {
                    return batch;
                }
                if !self.handed.insert((window, height.to_u32(), hash)) {
                    continue;
                }
                if let Some(record) = self.pricer.audit(log, window, height) {
                    batch.push(record);
                }
            }
        }
        batch
    }

    /// Forget the handed-out records below the log, which can never return.
    pub fn prune(&mut self, log: &RecordLog) {
        if let Some(oldest) = log.oldest_height() {
            self.handed.retain(|(_, height, _)| *height >= oldest.to_u32());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    use mantle::hash::Hash256;

    use super::AuditBackfill;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{ChainWindow, RecordLog};

    struct BackfillFixture {
        dir: PathBuf,
    }

    impl BackfillFixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("oracled-backfill-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn log(&self, blocks: u32) -> RecordLog {
            let mut log = RecordLog::open(self.dir.join("records.log"), 1_008).expect("open");
            let mut market = SyntheticMarket::new(80_000.0, 9);
            for height in 1..=blocks {
                log.append(market.record(height, 300)).expect("append");
            }
            log
        }

        fn heights(batch: &[crate::AuditRecord]) -> Vec<u32> {
            batch.iter().map(|record| record.report().stop_height.to_u32()).collect()
        }
    }

    impl Drop for BackfillFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn batches_run_newest_first_and_never_repeat() {
        let fixture = BackfillFixture::new("order");
        let log = fixture.log(9);
        let mut backfill = AuditBackfill::new(HashSet::new());

        let first = backfill.next_batch(&log, 2);
        let rest = backfill.next_batch(&log, 100);

        assert_eq!(BackfillFixture::heights(&first), vec![9, 8]);
        assert_eq!(BackfillFixture::heights(&rest), vec![7, 6]);
        assert!(backfill.next_batch(&log, 100).is_empty());
    }

    #[test]
    fn records_already_published_are_skipped() {
        let fixture = BackfillFixture::new("skip");
        let log = fixture.log(8);
        let published = HashSet::from([(ChainWindow::Hourly, 8, Hash256::hash(&8_u32.to_le_bytes()))]);
        let mut backfill = AuditBackfill::new(published);

        assert_eq!(BackfillFixture::heights(&backfill.next_batch(&log, 100)), vec![7, 6]);
    }

    #[test]
    fn a_block_published_under_another_hash_is_handed_out_again() {
        let fixture = BackfillFixture::new("reorg");
        let log = fixture.log(7);
        let published = HashSet::from([(ChainWindow::Hourly, 7, Hash256::hash(b"orphan"))]);
        let mut backfill = AuditBackfill::new(published);

        assert_eq!(BackfillFixture::heights(&backfill.next_batch(&log, 1)), vec![7]);
    }
}
