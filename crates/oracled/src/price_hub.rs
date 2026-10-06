use std::time::{Duration, Instant};

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::tx::Transaction;
use mantle_oracle::PriceBand;

use crate::{ChainSnapshot, MempoolWindow, NotePayload, NoteSink, Outcome, RelayPublisher};

/// Joins the chain prices, the live mempool window and the publisher.
///
/// Chain snapshots set the anchor band and reset the mempool window whenever
/// the tip moves. Mempool prices are computed on a fixed interval and only
/// while an anchor exists, so nothing is published from the mempool before
/// the chain has priced its newest blocks.
pub struct PriceHub<S: NoteSink> {
    publisher: RelayPublisher<S>,
    window: MempoolWindow,
    tolerance: f64,
    interval: Duration,
    anchor: Option<f64>,
    tip: Option<BlockHeight>,
    last_tick: Option<Instant>,
}

impl<S: NoteSink> PriceHub<S> {
    #[must_use]
    pub fn new(
        publisher: RelayPublisher<S>,
        window: MempoolWindow,
        tolerance: f64,
        interval: Duration,
    ) -> Self {
        Self {
            publisher,
            window,
            tolerance,
            interval,
            anchor: None,
            tip: None,
            last_tick: None,
        }
    }

    /// Publish every chain window and fit in the snapshot, and restart the
    /// mempool window if the tip is not the one it was counting from.
    pub fn apply_chain(&mut self, snapshot: &ChainSnapshot, now: Instant) -> Vec<(String, Outcome)> {
        self.anchor = snapshot.anchor_price().or(self.anchor);
        if self.tip != Some(snapshot.tip) {
            self.window.reset(snapshot.tip);
            self.tip = Some(snapshot.tip);
        }
        let prices = snapshot.updates.iter().map(|update| self.offer(update, now));
        let prices: Vec<(String, Outcome)> = prices.collect();
        let fits = snapshot.fits.iter().map(|fit| self.offer(fit, now));
        let fits: Vec<(String, Outcome)> = fits.collect();
        prices.into_iter().chain(fits).collect()
    }

    fn offer(&mut self, payload: &impl NotePayload, now: Instant) -> (String, Outcome) {
        (payload.window().to_string(), self.publisher.offer(payload, now))
    }

    pub fn absorb(&mut self, transaction: &Transaction, at: BlockTime) {
        self.window.absorb(transaction, at);
    }

    /// Price the mempool window if the interval has passed and an anchor
    /// exists, and publish its fit whenever the price itself goes out. Empty
    /// means nothing was offered to the publisher.
    pub fn tick(&mut self, now: Instant) -> Vec<(String, Outcome)> {
        if self
            .last_tick
            .is_some_and(|last| now.saturating_duration_since(last) < self.interval)
        {
            return Vec::new();
        }
        self.last_tick = Some(now);
        let Some(band) = self.anchor.and_then(|anchor| PriceBand::around(anchor, self.tolerance)) else {
            return Vec::new();
        };
        let Some(update) = self.window.update(band) else {
            return Vec::new();
        };
        let price = self.offer(&update, now);
        let published = matches!(price.1, Outcome::Published(_));
        let mut outcomes = vec![price];
        if published && let Some(fit) = self.window.fit(band) {
            outcomes.push(self.offer(&fit, now));
        }
        outcomes
    }

    #[must_use]
    pub const fn anchor(&self) -> Option<f64> {
        self.anchor
    }

    #[must_use]
    pub const fn tip(&self) -> Option<BlockHeight> {
        self.tip
    }

    #[must_use]
    pub fn mempool_outputs(&self) -> usize {
        self.window.outputs()
    }

    #[must_use]
    pub const fn publisher(&self) -> &RelayPublisher<S> {
        &self.publisher
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle_oracle::OracleReport;
    use nostro2::NostrNote;
    use nostro2_signer::NostrKeypair;
    use nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;

    use super::PriceHub;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{
        ChainSnapshot, ChainWindow, MempoolEstimator, MempoolWindow, NoteSink, Outcome, PriceSigner,
        PriceUpdate, PublishPolicy, RelayPublisher,
    };

    #[derive(Default)]
    struct RecordingSink {
        notes: RefCell<Vec<NostrNote>>,
    }

    impl NoteSink for RecordingSink {
        fn deliver(&self, note: &NostrNote) -> Result<(), String> {
            self.notes.borrow_mut().push(note.clone());
            Ok(())
        }
    }

    struct HubFixture;

    impl HubFixture {
        const INTERVAL: Duration = Duration::from_secs(10);

        fn hub() -> PriceHub<RecordingSink> {
            let mut policy = PublishPolicy::new(0.001, Duration::from_mins(1));
            for window in ChainWindow::ALL {
                policy = policy.with_rule(window.tag(), 0.0, Duration::from_mins(5));
                policy = policy.with_rule(&format!("fit-{}", window.tag()), 0.0, Duration::from_mins(5));
            }
            let publisher = RelayPublisher::new(
                RecordingSink::default(),
                PriceSigner::new(<NostrKeypair as KeypairTrait>::generate()),
                policy,
            );
            let window = MempoolWindow::new(
                "mempool",
                BlockHeight::new(0),
                MempoolEstimator::new(200, 10),
                3,
            );
            PriceHub::new(publisher, window, 0.15, Self::INTERVAL)
        }

        fn report(price: f64, stop: u32) -> OracleReport {
            OracleReport {
                central_price: price,
                deviation: 0.01,
                sample_count: 5_000,
                start_height: BlockHeight::new(stop - 5),
                stop_height: BlockHeight::new(stop),
                start_time: BlockTime::new(1),
                stop_time: BlockTime::new(2),
            }
        }

        fn snapshot(price: f64, tip: u32, windows: &[ChainWindow]) -> ChainSnapshot {
            ChainSnapshot {
                tip: BlockHeight::new(tip),
                updates: windows
                    .iter()
                    .map(|window| PriceUpdate::chain(window.tag(), &Self::report(price, tip)))
                    .collect(),
                fits: Vec::new(),
            }
        }

        fn fill(hub: &mut PriceHub<RecordingSink>, transactions: usize, price: f64) {
            let mut market = SyntheticMarket::new(price, 17);
            for _ in 0..transactions {
                hub.absorb(&market.transaction(), BlockTime::new(1));
            }
        }

        fn tags(hub: &PriceHub<RecordingSink>) -> Vec<String> {
            hub.publisher()
                .sink()
                .notes
                .borrow()
                .iter()
                .map(|note| note.tags.find_tags_ref("d")[0].to_string())
                .collect()
        }
    }

    #[test]
    fn a_snapshot_publishes_each_of_its_chain_windows() {
        let mut hub = HubFixture::hub();
        let snapshot = HubFixture::snapshot(80_000.0, 100, &ChainWindow::ALL);
        let outcomes = hub.apply_chain(&snapshot, Instant::now());

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(_, outcome)| matches!(outcome, Outcome::Published(_))));
        assert_eq!(HubFixture::tags(&hub), vec!["chain-1h", "chain-1d"]);
    }

    #[test]
    fn a_new_block_publishes_chain_windows_again_without_waiting() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[ChainWindow::Hourly]), start);
        hub.apply_chain(&HubFixture::snapshot(80_050.0, 101, &[ChainWindow::Hourly]), start);

        assert_eq!(HubFixture::tags(&hub).len(), 2);
    }

    #[test]
    fn an_unchanged_chain_price_waits_for_the_chain_heartbeat() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        let snapshot = HubFixture::snapshot(80_000.0, 100, &[ChainWindow::Hourly]);
        hub.apply_chain(&snapshot, start);
        let held = hub.apply_chain(&snapshot, start + Duration::from_secs(30));
        let beat = hub.apply_chain(&snapshot, start + Duration::from_mins(5));

        assert!(matches!(held[0].1, Outcome::Held));
        assert!(matches!(beat[0].1, Outcome::Published(_)));
    }

    #[test]
    fn a_new_tip_resets_the_mempool_window_and_the_same_tip_keeps_it() {
        let mut hub = HubFixture::hub();
        let now = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[ChainWindow::Hourly]), now);
        HubFixture::fill(&mut hub, 50, 80_000.0);
        let filled = hub.mempool_outputs();
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[ChainWindow::Hourly]), now);

        assert!(filled > 0);
        assert_eq!(hub.mempool_outputs(), filled);

        hub.apply_chain(&HubFixture::snapshot(80_000.0, 101, &[ChainWindow::Hourly]), now);
        assert_eq!(hub.mempool_outputs(), 0);
        assert_eq!(hub.tip(), Some(BlockHeight::new(101)));
    }

    #[test]
    fn nothing_is_priced_before_the_chain_provides_an_anchor() {
        let mut hub = HubFixture::hub();
        HubFixture::fill(&mut hub, 300, 80_000.0);

        assert!(hub.tick(Instant::now()).is_empty());
        assert!(HubFixture::tags(&hub).is_empty());
    }

    #[test]
    fn an_anchored_full_window_publishes_a_mempool_price() {
        let mut hub = HubFixture::hub();
        let now = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(81_000.0, 100, &[ChainWindow::Hourly]), now);
        HubFixture::fill(&mut hub, 300, 80_000.0);
        let outcomes = hub.tick(now);

        assert!(matches!(outcomes[0].1, Outcome::Published(_)));
        let notes = hub.publisher().sink().notes.borrow();
        let note = notes
            .iter()
            .find(|note| note.tags.find_tags_ref("d") == ["mempool"])
            .expect("note");
        assert!(note.content.contains("\"band_low\""));
        assert!(note.content.contains("\"transactions\""));
    }

    #[test]
    fn ticks_inside_the_interval_do_nothing() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(81_000.0, 100, &[ChainWindow::Hourly]), start);
        HubFixture::fill(&mut hub, 300, 80_000.0);
        hub.tick(start);

        assert!(hub.tick(start + Duration::from_secs(5)).is_empty());
        assert!(!hub.tick(start + HubFixture::INTERVAL).is_empty());
    }

    #[test]
    fn a_steady_mempool_price_is_held_between_heartbeats() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(81_000.0, 100, &[ChainWindow::Hourly]), start);
        HubFixture::fill(&mut hub, 300, 80_000.0);
        hub.tick(start);
        let second = hub.tick(start + HubFixture::INTERVAL);
        let third = hub.tick(start + Duration::from_mins(1));

        assert_eq!(second.len(), 1);
        assert!(matches!(second[0].1, Outcome::Held));
        assert!(matches!(third[0].1, Outcome::Published(_)));
    }

    #[test]
    fn a_published_mempool_price_is_followed_by_its_fit() {
        let mut hub = HubFixture::hub();
        let now = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(81_000.0, 100, &[ChainWindow::Hourly]), now);
        HubFixture::fill(&mut hub, 300, 80_000.0);
        let windows: Vec<String> = hub.tick(now).into_iter().map(|(window, _)| window).collect();

        assert_eq!(windows, vec!["mempool", "fit-mempool"]);
        assert!(HubFixture::tags(&hub).contains(&"fit-mempool".to_string()));
    }

    #[test]
    fn the_anchor_survives_a_snapshot_without_an_hourly_window() {
        let mut hub = HubFixture::hub();
        let now = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[ChainWindow::Hourly]), now);
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 101, &[ChainWindow::Daily]), now);

        assert_eq!(hub.anchor(), Some(80_000.0));
    }

    #[test]
    fn a_price_outside_the_anchor_band_is_not_published() {
        let mut hub = HubFixture::hub();
        let now = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(120_000.0, 100, &[ChainWindow::Hourly]), now);
        HubFixture::fill(&mut hub, 300, 80_000.0);
        hub.tick(now);

        let notes = hub.publisher().sink().notes.borrow();
        let mempool_prices: Vec<f64> = notes
            .iter()
            .filter(|note| note.tags.find_tags_ref("d") == ["mempool"])
            .filter_map(|note| {
                note.content
                    .split("\"price\":")
                    .nth(1)?
                    .split(',')
                    .next()?
                    .parse()
                    .ok()
            })
            .collect();
        let band = (120_000.0 / 1.15)..=(120_000.0 * 1.15);
        assert!(mempool_prices.iter().all(|price| band.contains(price)), "{mempool_prices:?}");
    }
}
