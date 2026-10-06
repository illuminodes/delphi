use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use mantle::difficulty::BlockHeight;
use mantle::hash::Hash256;

use crate::{AuditLedger, AuditRecord, AuditStatus, ChainPricer, ChainWindow, RecordLog};

/// Finds the audit records the ledger has not seen, newest first, so the tip
/// goes out before the history behind it.
///
/// A record handed out once is not handed out again, even before the ledger
/// confirms it, and neither is a block that cannot be priced.
pub struct AuditBackfill {
    pricer: ChainPricer,
    ledger: Arc<Mutex<AuditLedger>>,
    handed: HashSet<(ChainWindow, u32, Hash256)>,
}

impl AuditBackfill {
    #[must_use]
    pub fn new(ledger: Arc<Mutex<AuditLedger>>) -> Self {
        Self {
            pricer: ChainPricer::new(),
            ledger,
            handed: HashSet::new(),
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
                let status = self
                    .ledger
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .status(window, height, hash);
                let replaces = match status {
                    AuditStatus::Done => continue,
                    AuditStatus::New => None,
                    AuditStatus::Replaces(id) => Some(id),
                };
                if let Some(record) = self.pricer.audit(log, window, height) {
                    batch.push(match replaces {
                        Some(id) => record.replacing(id),
                        None => record,
                    });
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
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use mantle::difficulty::BlockHeight;
    use mantle::hash::Hash256;

    use super::AuditBackfill;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{AuditLedger, ChainWindow, NotePayload, RecordLog};

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
        let mut backfill = AuditBackfill::new(Arc::new(Mutex::new(AuditLedger::volatile())));

        let first = backfill.next_batch(&log, 2);
        let rest = backfill.next_batch(&log, 100);

        assert_eq!(BackfillFixture::heights(&first), vec![9, 8]);
        assert_eq!(BackfillFixture::heights(&rest), vec![7, 6]);
        assert!(backfill.next_batch(&log, 100).is_empty());
    }

    #[test]
    fn records_already_in_the_ledger_are_skipped() {
        let fixture = BackfillFixture::new("skip");
        let log = fixture.log(8);
        let ledger = Arc::new(Mutex::new(AuditLedger::volatile()));
        let tip_hash = Hash256::hash(&8_u32.to_le_bytes());
        ledger
            .lock()
            .expect("lock")
            .record(ChainWindow::Hourly, BlockHeight::new(8), tip_hash, "seen")
            .expect("record");
        let mut backfill = AuditBackfill::new(ledger);

        assert_eq!(BackfillFixture::heights(&backfill.next_batch(&log, 100)), vec![7, 6]);
    }

    #[test]
    fn a_block_replaced_by_a_reorg_points_at_the_old_note() {
        let fixture = BackfillFixture::new("reorg");
        let log = fixture.log(7);
        let ledger = Arc::new(Mutex::new(AuditLedger::volatile()));
        ledger
            .lock()
            .expect("lock")
            .record(ChainWindow::Hourly, BlockHeight::new(7), Hash256::hash(b"orphan"), "old-id")
            .expect("record");
        let mut backfill = AuditBackfill::new(ledger);
        let batch = backfill.next_batch(&log, 1);

        assert_eq!(batch[0].replaces(), Some("old-id"));
        assert_eq!(batch[0].window(), "chain-1h");
    }
}
