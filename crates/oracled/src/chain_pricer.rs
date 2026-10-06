use mantle::difficulty::BlockHeight;
use mantle_oracle::RoughPriceEstimator;

use crate::{AuditRecord, BlockRecord, ChainWindow, FitUpdate, RecordLog};

/// Prices chain windows out of the record log.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChainPricer;

/// The chain state after a synchronization pass, ready to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainSnapshot {
    pub tip: BlockHeight,
    /// The hourly price at the tip, which anchors the mempool estimate.
    pub anchor: Option<f64>,
    /// Audit records not yet published, newest first.
    pub audits: Vec<AuditRecord>,
    /// The evidence behind each window's price at the tip.
    pub fits: Vec<FitUpdate>,
}

impl ChainPricer {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// The window's final price over the blocks ending at `height`, or `None`
    /// while the log does not hold them all, the stop block has no hash, or
    /// the outputs cannot be priced.
    #[must_use]
    pub fn audit(&self, log: &RecordLog, window: ChainWindow, height: BlockHeight) -> Option<AuditRecord> {
        let stop = log.get(height).filter(|record| !record.lacks_hash())?;
        let tip = log.tip().map(BlockRecord::height)?.to_u32();
        let newer = usize::try_from(tip.checked_sub(height.to_u32())?).ok()?;
        let report = log
            .sample_ending_before_tip(window.blocks(), newer)?
            .price()
            .ok()?;
        Some(AuditRecord::new(window, report, stop.hash()))
    }

    #[must_use]
    pub fn anchor(&self, log: &RecordLog) -> Option<f64> {
        let report = log.sample(ChainWindow::Hourly.blocks())?.price().ok()?;
        Some(report.central_price)
    }

    /// The evidence behind every window's price at the tip.
    #[must_use]
    pub fn fits(&self, log: &RecordLog) -> Vec<FitUpdate> {
        ChainWindow::ALL
            .into_iter()
            .filter_map(|window| self.fit(log, window))
            .collect()
    }

    #[must_use]
    pub fn fit(&self, log: &RecordLog, window: ChainWindow) -> Option<FitUpdate> {
        let fit = log
            .sample(window.blocks())?
            .fit(&RoughPriceEstimator::new(), None)
            .ok()?;
        Some(FitUpdate::new(window.tag(), "chain", fit))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use mantle::difficulty::BlockHeight;
    use mantle::hash::Hash256;

    use super::ChainPricer;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{BlockRecord, ChainWindow, NotePayload, RecordLog};

    struct LogFixture {
        dir: PathBuf,
    }

    impl LogFixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("oracled-pricer-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn log_with(&self, blocks: u32, outputs_per_block: usize, price: f64) -> RecordLog {
            let mut log = RecordLog::open(self.dir.join("records.log"), 1_008).expect("open");
            let mut market = SyntheticMarket::new(price, 21);
            for height in 1..=blocks {
                log.append(market.record(height, outputs_per_block)).expect("append");
            }
            log
        }
    }

    impl Drop for LogFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn an_audit_covers_exactly_the_window_ending_at_its_height() {
        let fixture = LogFixture::new("audit");
        let log = fixture.log_with(20, 400, 80_000.0);
        let record = ChainPricer::new()
            .audit(&log, ChainWindow::Hourly, BlockHeight::new(12))
            .expect("audit");

        assert_eq!(record.report().start_height, BlockHeight::new(7));
        assert_eq!(record.report().stop_height, BlockHeight::new(12));
        assert_eq!(record.stop_hash(), Hash256::hash(&12_u32.to_le_bytes()));
        assert!((record.price() - 80_000.0).abs() / 80_000.0 < 0.02, "{}", record.price());
    }

    #[test]
    fn an_audit_needs_the_whole_window_in_the_log() {
        let fixture = LogFixture::new("audit-short");
        let log = fixture.log_with(20, 400, 80_000.0);
        let pricer = ChainPricer::new();

        assert!(pricer.audit(&log, ChainWindow::Hourly, BlockHeight::new(5)).is_none());
        assert!(pricer.audit(&log, ChainWindow::Hourly, BlockHeight::new(6)).is_some());
        assert!(pricer.audit(&log, ChainWindow::Hourly, BlockHeight::new(21)).is_none());
        assert!(pricer.audit(&log, ChainWindow::Daily, BlockHeight::new(20)).is_none());
    }

    #[test]
    fn a_stop_block_without_a_hash_is_not_audited() {
        let fixture = LogFixture::new("audit-nohash");
        let mut log = fixture.log_with(10, 400, 80_000.0);
        let mut market = SyntheticMarket::new(80_000.0, 4);
        let tip = market.record(11, 400);
        log.append(BlockRecord::new(tip.height(), tip.time(), Hash256::ZERO, tip.amounts().to_vec()))
            .expect("append");

        assert!(ChainPricer::new().audit(&log, ChainWindow::Hourly, BlockHeight::new(11)).is_none());
    }

    #[test]
    fn the_anchor_is_the_hourly_price_at_the_tip() {
        let fixture = LogFixture::new("anchor");
        let log = fixture.log_with(12, 400, 80_000.0);
        let pricer = ChainPricer::new();
        let at_tip = pricer
            .audit(&log, ChainWindow::Hourly, BlockHeight::new(12))
            .expect("audit");

        assert_eq!(pricer.anchor(&log), Some(at_tip.price()));
        let empty = LogFixture::new("anchor-empty");
        assert!(pricer.anchor(&empty.log_with(0, 0, 1.0)).is_none());
    }

    #[test]
    fn each_window_the_log_can_fill_gets_a_fit() {
        let fixture = LogFixture::new("fits");
        let log = fixture.log_with(150, 60, 80_000.0);
        let fits = ChainPricer::new().fits(&log);
        let windows: Vec<&str> = fits.iter().map(NotePayload::window).collect();

        assert_eq!(windows, vec!["fit-chain-1h", "fit-chain-1d"]);
    }
}
