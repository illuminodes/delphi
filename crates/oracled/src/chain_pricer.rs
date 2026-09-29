use mantle::difficulty::BlockHeight;

use crate::{BlockRecord, ChainWindow, PriceUpdate, RecordLog};

/// Confirmed-chain prices for every window the record log can fill.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChainPricer;

/// The chain state after a synchronization pass, ready to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainSnapshot {
    pub tip: BlockHeight,
    pub updates: Vec<PriceUpdate>,
}

impl ChainSnapshot {
    /// The shortest window's price, which anchors the mempool estimate.
    #[must_use]
    pub fn anchor_price(&self) -> Option<f64> {
        let hourly = ChainWindow::Hourly.tag();
        self.updates
            .iter()
            .find(|update| update.window() == hourly)
            .map(PriceUpdate::price)
    }
}

impl ChainPricer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// A price for each window with enough records behind it. Windows that
    /// are not yet full, or whose outputs cannot be priced, are left out.
    #[must_use]
    pub fn snapshot(&self, log: &RecordLog) -> Option<ChainSnapshot> {
        let tip = log.tip().map(BlockRecord::height)?;
        let updates = ChainWindow::ALL
            .into_iter()
            .filter_map(|window| self.price(log, window))
            .collect();
        Some(ChainSnapshot { tip, updates })
    }

    #[must_use]
    pub fn price(&self, log: &RecordLog, window: ChainWindow) -> Option<PriceUpdate> {
        let sample = log.sample(window.blocks())?;
        let report = sample.price().ok()?;
        Some(PriceUpdate::chain(window.tag(), &report))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use mantle::difficulty::BlockHeight;

    use super::ChainPricer;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{ChainWindow, PriceSource, PriceUpdate, RecordLog};

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
    fn an_empty_log_has_no_snapshot() {
        let fixture = LogFixture::new("empty");
        let log = fixture.log_with(0, 0, 80_000.0);

        assert!(ChainPricer::new().snapshot(&log).is_none());
    }

    #[test]
    fn only_windows_the_log_can_fill_are_priced() {
        let fixture = LogFixture::new("partial");
        let log = fixture.log_with(10, 400, 80_000.0);
        let snapshot = ChainPricer::new().snapshot(&log).expect("snapshot");
        let windows: Vec<&str> = snapshot.updates.iter().map(PriceUpdate::window).collect();

        assert_eq!(windows, vec![ChainWindow::Hourly.tag()]);
        assert_eq!(snapshot.tip, BlockHeight::new(10));
    }

    #[test]
    fn the_hourly_price_uses_exactly_the_newest_blocks() {
        let fixture = LogFixture::new("hourly");
        let log = fixture.log_with(12, 400, 80_000.0);
        let update = ChainPricer::new()
            .price(&log, ChainWindow::Hourly)
            .expect("price");
        let PriceSource::Chain {
            start_height,
            stop_height,
            ..
        } = *update.source()
        else {
            panic!("chain source expected");
        };

        assert_eq!(start_height, BlockHeight::new(7));
        assert_eq!(stop_height, BlockHeight::new(12));
        assert!((update.price() - 80_000.0).abs() / 80_000.0 < 0.02, "{}", update.price());
    }

    #[test]
    fn a_daily_window_appears_once_144_blocks_are_held() {
        let fixture = LogFixture::new("daily");
        let log = fixture.log_with(150, 60, 80_000.0);
        let snapshot = ChainPricer::new().snapshot(&log).expect("snapshot");
        let windows: Vec<&str> = snapshot.updates.iter().map(PriceUpdate::window).collect();

        assert_eq!(windows, vec![ChainWindow::Hourly.tag(), ChainWindow::Daily.tag()]);
    }

    #[test]
    fn the_anchor_is_the_hourly_price() {
        let fixture = LogFixture::new("anchor");
        let log = fixture.log_with(150, 60, 80_000.0);
        let snapshot = ChainPricer::new().snapshot(&log).expect("snapshot");
        let hourly = snapshot.updates[0].price();

        assert_eq!(snapshot.anchor_price(), Some(hourly));
    }

    #[test]
    fn a_snapshot_without_an_hourly_window_has_no_anchor() {
        let fixture = LogFixture::new("no-anchor");
        let log = fixture.log_with(3, 400, 80_000.0);
        let snapshot = ChainPricer::new().snapshot(&log).expect("snapshot");

        assert!(snapshot.updates.is_empty());
        assert!(snapshot.anchor_price().is_none());
    }
}
