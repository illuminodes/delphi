use mantle::difficulty::{BlockHeight, BlockTime};
use delphi_oracle::{PriceBand, RoughPriceEstimator};

use crate::{Anchor, AuditRecord, BlockRecord, ChainWindow, FitUpdate, NotePayload, RecentBlocks, RecordLog};

/// Prices chain windows out of the record log.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChainPricer;

/// The chain state after a synchronization pass, ready to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainSnapshot {
    pub tip: BlockHeight,
    /// The tip header's timestamp.
    pub tip_time: BlockTime,
    /// The newest height enough peer groups agree on; audits stop here.
    pub settled: Option<BlockHeight>,
    /// The hourly price at the tip, which anchors the mempool estimate.
    pub anchor: Option<Anchor>,
    /// Audit records not yet published, newest first.
    pub audits: Vec<AuditRecord>,
    /// The evidence behind each window's price at the tip.
    pub fits: Vec<FitUpdate>,
    /// The newest blocks read, newest first.
    pub blocks: RecentBlocks,
}

impl ChainPricer {
    /// How far, as a ratio either way, a shorter window may stray from the
    /// deepest window's price at the same block. A thin sample can lock onto
    /// a peak one octave (2x) off; 1.3x keeps those out and leaves room for a
    /// real intraday move.
    pub const OCTAVE_GUARD: f64 = 0.3;

    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// The window's final price over the blocks ending at `height`, or `None`
    /// while the log does not hold them all, the stop block has no hash, or
    /// the outputs cannot be priced.
    #[must_use]
    pub fn audit(&self, log: &RecordLog, window: ChainWindow, height: BlockHeight) -> Option<AuditRecord> {
        let reference = self.reference(log, window, height);
        self.audit_against(log, window, height, reference)
    }

    /// [`Self::audit`] with the deepest window's price at `height` already
    /// known; `None` leaves the price unbounded.
    #[must_use]
    pub fn audit_against(
        &self,
        log: &RecordLog,
        window: ChainWindow,
        height: BlockHeight,
        reference: Option<f64>,
    ) -> Option<AuditRecord> {
        let stop = log.get(height).filter(|record| !record.lacks_hash())?;
        let sample = log.sample_ending_before_tip(window.blocks(), Self::newer_than(log, height)?)?;
        let report = match Self::band(reference) {
            Some(band) => sample.price_within(&RoughPriceEstimator::new(), band),
            None => sample.price(),
        }
        .ok()?;
        Some(AuditRecord::new(window, report, stop.hash()))
    }

    /// The deepest window's price at `height`, which bounds every shorter
    /// window there, or `None` for the deepest window itself or while the log
    /// is too short to price it.
    #[must_use]
    pub fn reference(&self, log: &RecordLog, window: ChainWindow, height: BlockHeight) -> Option<f64> {
        let deepest = ChainWindow::deepest();
        if window == deepest {
            return None;
        }
        log.sample_ending_before_tip(deepest.blocks(), Self::newer_than(log, height)?)?
            .price()
            .ok()
            .map(|report| report.central_price)
    }

    fn band(reference: Option<f64>) -> Option<PriceBand> {
        PriceBand::around(reference?, Self::OCTAVE_GUARD)
    }

    /// The hourly price ending at `height`, when the daily price there, if
    /// the log holds a day, agrees with it.
    #[must_use]
    pub fn anchor(&self, log: &RecordLog, height: BlockHeight) -> Option<Anchor> {
        let daily = self.reference(log, ChainWindow::Hourly, height);
        let hourly = self
            .audit_against(log, ChainWindow::Hourly, height, daily)?
            .price();
        Anchor::checked(hourly, daily, height)
    }

    /// The evidence behind every window's price ending at `height`.
    #[must_use]
    pub fn fits(&self, log: &RecordLog, height: BlockHeight) -> Vec<FitUpdate> {
        let daily = self.reference(log, ChainWindow::Hourly, height);
        ChainWindow::ALL
            .into_iter()
            .filter_map(|window| {
                let reference = daily.filter(|_| window != ChainWindow::deepest());
                self.fit(log, window, height, reference)
            })
            .collect()
    }

    #[must_use]
    pub fn fit(
        &self,
        log: &RecordLog,
        window: ChainWindow,
        height: BlockHeight,
        reference: Option<f64>,
    ) -> Option<FitUpdate> {
        let fit = log
            .sample_ending_before_tip(window.blocks(), Self::newer_than(log, height)?)?
            .fit(&RoughPriceEstimator::new(), Self::band(reference))
            .ok()?;
        Some(FitUpdate::new(window.tag(), "chain", fit))
    }

    fn newer_than(log: &RecordLog, height: BlockHeight) -> Option<usize> {
        let tip = log.tip().map(BlockRecord::height)?.to_u32();
        usize::try_from(tip.checked_sub(height.to_u32())?).ok()
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

        let tip = BlockHeight::new(12);
        assert_eq!(pricer.anchor(&log, tip).map(|anchor| anchor.price), Some(at_tip.price()));
        assert_eq!(pricer.anchor(&log, tip).map(|anchor| anchor.height), Some(tip));
        let empty = LogFixture::new("anchor-empty");
        assert!(pricer.anchor(&empty.log_with(0, 0, 1.0), tip).is_none());
    }

    #[test]
    fn an_anchor_below_the_tip_prices_the_window_ending_there() {
        let fixture = LogFixture::new("anchor-settled");
        let log = fixture.log_with(12, 400, 80_000.0);
        let pricer = ChainPricer::new();
        let settled = BlockHeight::new(9);
        let audit = pricer.audit(&log, ChainWindow::Hourly, settled).expect("audit");

        assert_eq!(pricer.anchor(&log, settled).map(|anchor| anchor.price), Some(audit.price()));
        assert_eq!(pricer.anchor(&log, settled).map(|anchor| anchor.height), Some(settled));
        assert!(pricer.anchor(&log, BlockHeight::new(13)).is_none());
    }

    #[test]
    fn a_short_window_is_held_to_the_octave_of_its_reference() {
        let fixture = LogFixture::new("octave");
        let log = fixture.log_with(150, 400, 80_000.0);
        let pricer = ChainPricer::new();
        let height = BlockHeight::new(150);
        let daily = pricer.reference(&log, ChainWindow::Hourly, height).expect("daily");
        let held = pricer.audit(&log, ChainWindow::Hourly, height).expect("audit");
        let moved = pricer
            .audit_against(&log, ChainWindow::Hourly, height, Some(160_000.0))
            .expect("audit");

        assert!((held.price() / daily - 1.0).abs() < 0.05, "{}", held.price());
        assert!((moved.price() / 160_000.0 - 1.0).abs() < ChainPricer::OCTAVE_GUARD, "{}", moved.price());
    }

    #[test]
    fn the_deepest_window_has_no_reference() {
        let fixture = LogFixture::new("reference");
        let log = fixture.log_with(150, 60, 80_000.0);

        assert!(ChainPricer::new().reference(&log, ChainWindow::Daily, BlockHeight::new(150)).is_none());
    }

    #[test]
    fn each_window_the_log_can_fill_gets_a_fit() {
        let fixture = LogFixture::new("fits");
        let log = fixture.log_with(150, 60, 80_000.0);
        let fits = ChainPricer::new().fits(&log, BlockHeight::new(150));
        let windows: Vec<&str> = fits.iter().map(NotePayload::window).collect();

        assert_eq!(windows, vec!["fit-chain-1h", "fit-chain-1d"]);
    }
}
