use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::tx::Transaction;
use delphi_oracle::{MempoolCollector, PriceBand, RoughPriceEstimator, TransactionVerdict};

use crate::{BootstrapRng, EstimateJob, EstimateResult, FitUpdate, MempoolEstimator, PriceUpdate};

/// Unconfirmed transactions seen since the last confirmed block.
///
/// A new block confirms most of what the pool held, so the collector is
/// discarded and started again whenever the tip moves. The window is
/// therefore bounded by block time rather than by a transaction count.
pub struct MempoolWindow {
    name: String,
    collector: MempoolCollector,
    estimator: MempoolEstimator,
    rng: BootstrapRng,
}

impl MempoolWindow {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        height: BlockHeight,
        estimator: MempoolEstimator,
        seed: u64,
    ) -> Self {
        Self {
            name: name.into(),
            collector: MempoolCollector::new(height),
            estimator,
            rng: BootstrapRng::seeded(seed),
        }
    }

    pub fn absorb(&mut self, transaction: &Transaction, at: BlockTime) -> (TransactionVerdict, Vec<Satoshi>) {
        self.collector.screen(transaction, at)
    }

    /// Discard the pool and start counting from a new tip.
    pub fn reset(&mut self, height: BlockHeight) {
        self.collector = MempoolCollector::new(height);
    }

    #[must_use]
    pub fn outputs(&self) -> usize {
        self.collector.sample().len()
    }

    #[must_use]
    pub fn admitted_transactions(&self) -> u64 {
        self.collector.admitted_transactions()
    }

    #[must_use]
    pub const fn estimator(&self) -> MempoolEstimator {
        self.estimator
    }

    /// A copy of the sample to price inside `band`, or `None` while it is
    /// thinner than the estimator's minimum.
    #[must_use]
    pub fn job(&self, band: PriceBand, tip: Option<BlockHeight>) -> Option<EstimateJob> {
        let sample = self.collector.sample();
        if sample.len() < self.estimator.min_outputs() {
            return None;
        }
        Some(EstimateJob {
            name: self.name.clone(),
            sample: sample.clone(),
            admitted_transactions: self.collector.admitted_transactions(),
            band,
            tip,
        })
    }

    /// Run `job` here, on this window's own generator.
    pub fn run(&mut self, job: EstimateJob) -> EstimateResult {
        job.run(&self.estimator, &mut self.rng)
    }

    /// The evidence behind the window's price inside `band`, under the same
    /// minimum sample size as [`Self::update`].
    #[must_use]
    pub fn fit(&self, band: PriceBand) -> Option<FitUpdate> {
        let sample = self.collector.sample();
        if sample.len() < self.estimator.min_outputs() {
            return None;
        }
        let fit = sample.fit(&RoughPriceEstimator::new(), Some(band)).ok()?;
        Some(FitUpdate::new(&self.name, "mempool", fit))
    }

    /// The window's current price inside `band`, or `None` while the sample
    /// is thin or no price falls inside the band.
    pub fn update(&mut self, band: PriceBand) -> Option<PriceUpdate> {
        let job = self.job(band, None)?;
        self.run(job).update
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};
    use delphi_oracle::PriceBand;

    use super::MempoolWindow;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{MempoolEstimator, NotePayload};

    struct PoolFixture;

    impl PoolFixture {
        fn fill(window: &mut MempoolWindow, transactions: usize, price: f64) {
            let mut market = SyntheticMarket::new(price, 11);
            for _ in 0..transactions {
                let transaction = market.transaction();
                window.absorb(&transaction, BlockTime::new(1));
            }
        }
    }

    fn window() -> MempoolWindow {
        MempoolWindow::new("mempool", BlockHeight::new(1), MempoolEstimator::new(200, 20), 5)
    }

    fn band() -> PriceBand {
        PriceBand::around(80_000.0, 0.15).expect("band")
    }

    #[test]
    fn an_empty_window_has_no_update() {
        assert!(window().update(band()).is_none());
    }

    #[test]
    fn a_thin_window_has_no_update() {
        let mut window = window();
        PoolFixture::fill(&mut window, 30, 80_000.0);

        assert!(window.outputs() < 200);
        assert!(window.update(band()).is_none());
        assert!(window.job(band(), None).is_none());
    }

    #[test]
    fn a_full_window_prices_near_the_planted_price() {
        let mut window = window();
        PoolFixture::fill(&mut window, 300, 80_000.0);
        let update = window.update(band()).expect("update");

        assert_eq!(update.window(), "mempool");
        assert!((update.price() - 80_000.0).abs() / 80_000.0 < 0.02, "{}", update.price());
        assert_eq!(update.samples(), window.outputs());
        let (band_low, band_high) = update.band();
        assert_eq!(update.transactions() as u64, window.admitted_transactions());
        assert!(band_low <= update.price() + 1.0 && update.price() <= band_high + 1.0);
        assert!(update.is_publishable());
    }

    #[test]
    fn a_job_run_elsewhere_carries_its_tip_and_the_same_fit() {
        let mut window = window();
        PoolFixture::fill(&mut window, 300, 80_000.0);
        let job = window.job(band(), Some(BlockHeight::new(7))).expect("job");
        let estimator = window.estimator();

        let result = job.run(&estimator, &mut crate::BootstrapRng::seeded(5));

        assert_eq!(result.tip, Some(BlockHeight::new(7)));
        assert!(result.update.is_some());
        let fit = result.fit.expect("fit");
        assert_eq!(fit.fit().report, window.fit(band()).expect("fit").fit().report);
    }

    #[test]
    fn a_full_window_fits_at_its_published_price() {
        let mut window = window();
        PoolFixture::fill(&mut window, 300, 80_000.0);
        let update = window.update(band()).expect("update");
        let fit = window.fit(band()).expect("fit");

        assert!((fit.fit().report.central_price - update.price()).abs() < 1e-6);
        assert_eq!(fit.fit().band, Some(band()));
    }

    #[test]
    fn a_thin_window_has_no_fit() {
        let mut window = window();
        PoolFixture::fill(&mut window, 30, 80_000.0);

        assert!(window.fit(band()).is_none());
    }

    #[test]
    fn a_new_block_empties_the_window() {
        let mut window = window();
        PoolFixture::fill(&mut window, 300, 80_000.0);
        window.reset(BlockHeight::new(2));

        assert_eq!(window.outputs(), 0);
        assert_eq!(window.admitted_transactions(), 0);
        assert!(window.update(band()).is_none());
    }

    #[test]
    fn the_window_refills_after_a_reset() {
        let mut window = window();
        PoolFixture::fill(&mut window, 300, 80_000.0);
        window.reset(BlockHeight::new(2));
        PoolFixture::fill(&mut window, 300, 80_000.0);

        assert!(window.update(band()).is_some());
    }

    #[test]
    fn a_band_around_a_far_price_yields_nothing_near_the_true_price() {
        let mut window = window();
        PoolFixture::fill(&mut window, 300, 80_000.0);
        let far = PriceBand::around(160_000.0, 0.05).expect("band");

        assert!(window.update(far).is_none_or(|update| far.contains(update.price())));
    }
}
