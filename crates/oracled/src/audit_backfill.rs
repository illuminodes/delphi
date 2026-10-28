use std::collections::HashSet;

use mantle::difficulty::BlockHeight;
use mantle::hash::Hash256;

use crate::{AuditRecord, ChainPricer, ChainWindow, NotePayload, RecordLog};

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

    /// Up to `limit` records still to publish, stopping no higher than
    /// `settled`: an audit is signed for good, so it waits until enough
    /// peers agree on its block.
    pub fn next_batch(&mut self, log: &RecordLog, limit: usize, settled: BlockHeight) -> Vec<AuditRecord> {
        let mut batch = Vec::new();
        let blocks: Vec<(BlockHeight, Hash256)> = log
            .records()
            .filter(|record| !record.lacks_hash() && record.height() <= settled)
            .map(|record| (record.height(), record.hash()))
            .collect();
        let deepest = ChainWindow::deepest();
        for (height, hash) in blocks.into_iter().rev() {
            let mut deepest_price = None;
            for window in Self::deepest_first() {
                if batch.len() >= limit {
                    return batch;
                }
                if !self.handed.insert((window, height.to_u32(), hash)) {
                    continue;
                }
                let reference = if window == deepest {
                    None
                } else {
                    deepest_price.or_else(|| self.pricer.reference(log, window, height))
                };
                if let Some(record) = self.pricer.audit_against(log, window, height, reference) {
                    if window == deepest {
                        deepest_price = Some(record.price());
                    }
                    batch.push(record);
                }
            }
        }
        batch
    }

    /// The windows with the deepest first, so its price can bound the rest.
    fn deepest_first() -> [ChainWindow; 2] {
        let mut windows = ChainWindow::ALL;
        windows.sort_by_key(|window| std::cmp::Reverse(window.blocks()));
        windows
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

    use mantle::difficulty::BlockHeight;
    use mantle::hash::Hash256;

    use super::AuditBackfill;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{ChainWindow, RecordLog};

    struct BackfillFixture {
        dir: PathBuf,
    }

    impl BackfillFixture {
        const ALL: BlockHeight = BlockHeight::new(u32::MAX);

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

        let first = backfill.next_batch(&log, 2, BackfillFixture::ALL);
        let rest = backfill.next_batch(&log, 100, BackfillFixture::ALL);

        assert_eq!(BackfillFixture::heights(&first), vec![9, 8]);
        assert_eq!(BackfillFixture::heights(&rest), vec![7, 6]);
        assert!(backfill.next_batch(&log, 100, BackfillFixture::ALL).is_empty());
    }

    #[test]
    fn records_already_published_are_skipped() {
        let fixture = BackfillFixture::new("skip");
        let log = fixture.log(8);
        let published = HashSet::from([(ChainWindow::Hourly, 8, Hash256::hash(&8_u32.to_le_bytes()))]);
        let mut backfill = AuditBackfill::new(published);

        assert_eq!(BackfillFixture::heights(&backfill.next_batch(&log, 100, BackfillFixture::ALL)), vec![7, 6]);
    }

    #[test]
    fn nothing_above_the_settled_height_is_handed_out_until_it_settles() {
        let fixture = BackfillFixture::new("settled");
        let log = fixture.log(9);
        let mut backfill = AuditBackfill::new(HashSet::new());

        assert_eq!(BackfillFixture::heights(&backfill.next_batch(&log, 100, BlockHeight::new(7))), vec![7, 6]);
        assert_eq!(BackfillFixture::heights(&backfill.next_batch(&log, 100, BlockHeight::new(9))), vec![9, 8]);
    }

    #[test]
    fn a_block_published_under_another_hash_is_handed_out_again() {
        let fixture = BackfillFixture::new("reorg");
        let log = fixture.log(7);
        let published = HashSet::from([(ChainWindow::Hourly, 7, Hash256::hash(b"orphan"))]);
        let mut backfill = AuditBackfill::new(published);

        assert_eq!(BackfillFixture::heights(&backfill.next_batch(&log, 1, BackfillFixture::ALL)), vec![7]);
    }
}
