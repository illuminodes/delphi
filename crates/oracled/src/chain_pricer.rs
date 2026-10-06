use mantle::difficulty::BlockHeight;
use mantle_oracle::RoughPriceEstimator;

use crate::{BlockRecord, ChainWindow, FitUpdate, PriceUpdate, RecordLog, SeriesPoint};

/// Confirmed-chain timelines for every window the record log can fill.
///
/// A timeline holds one price per `ChainWindow::STEP_BLOCKS`, each over a
/// full window of blocks, from the oldest the log allows up to the tip.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChainPricer;

/// The chain state after a synchronization pass, ready to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainSnapshot {
    pub tip: BlockHeight,
    pub updates: Vec<PriceUpdate>,
    pub fits: Vec<FitUpdate>,
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
        let fits = ChainWindow::ALL
            .into_iter()
            .filter_map(|window| self.fit(log, window))
            .collect();
        Some(ChainSnapshot { tip, updates, fits })
    }

    /// The evidence behind the window's newest price.
    #[must_use]
    pub fn fit(&self, log: &RecordLog, window: ChainWindow) -> Option<FitUpdate> {
        let fit = log
            .sample_ending_before_tip(window.blocks(), 0)?
            .fit(&RoughPriceEstimator::new(), None)
            .ok()?;
        Some(FitUpdate::new(window.tag(), "chain", fit))
    }

    /// The window's timeline, headlined by its newest price. `None` while the
    /// log cannot fill one window ending at the tip.
    #[must_use]
    pub fn price(&self, log: &RecordLog, window: ChainWindow) -> Option<PriceUpdate> {
        let mut points = Vec::new();
        let mut headline = None;
        let mut newer = 0;
        while let Some(sample) = log.sample_ending_before_tip(window.blocks(), newer) {
            if let Ok(report) = sample.price() {
                points.push(SeriesPoint::from_report(&report));
                if newer == 0 {
                    headline = Some(report);
                }
            }
            newer += ChainWindow::STEP_BLOCKS;
        }
        points.reverse();
        Some(PriceUpdate::chain(window.tag(), &headline?).with_series(points))
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
    fn the_hourly_timeline_steps_one_hour_back_at_a_time() {
        let fixture = LogFixture::new("timeline");
        let log = fixture.log_with(30, 400, 80_000.0);
        let update = ChainPricer::new()
            .price(&log, ChainWindow::Hourly)
            .expect("price");
        let stops: Vec<u32> = update.series().iter().map(|point| point.stop_height.to_u32()).collect();

        assert_eq!(stops, vec![6, 12, 18, 24, 30]);
        assert_eq!(update.series().last().map(|point| point.price), Some(update.price()));
    }

    #[test]
    fn every_point_is_priced_near_the_market() {
        let fixture = LogFixture::new("timeline-prices");
        let log = fixture.log_with(30, 400, 80_000.0);
        let update = ChainPricer::new()
            .price(&log, ChainWindow::Hourly)
            .expect("price");

        for point in update.series() {
            assert!((point.price - 80_000.0).abs() / 80_000.0 < 0.02, "{}", point.price);
        }
    }

    #[test]
    fn the_timeline_json_carries_the_points() {
        let fixture = LogFixture::new("timeline-json");
        let log = fixture.log_with(18, 400, 80_000.0);
        let update = ChainPricer::new()
            .price(&log, ChainWindow::Hourly)
            .expect("price");
        let json: serde_json::Value = serde_json::from_str(&update.to_json()).expect("json");

        assert_eq!(json["points"].as_array().map(Vec::len), Some(3));
        assert_eq!(json["points"][2]["stop_height"], 18);
    }

    #[test]
    fn each_priced_window_gets_a_fit_at_its_headline_price() {
        let fixture = LogFixture::new("fits");
        let log = fixture.log_with(150, 60, 80_000.0);
        let snapshot = ChainPricer::new().snapshot(&log).expect("snapshot");
        let windows: Vec<&str> = snapshot.fits.iter().map(crate::NotePayload::window).collect();

        assert_eq!(windows, vec!["fit-chain-1h", "fit-chain-1d"]);
        for (update, fit) in snapshot.updates.iter().zip(&snapshot.fits) {
            assert!((fit.fit().report.central_price - update.price()).abs() < 1e-6);
        }
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
