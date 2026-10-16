use delphi_oracle::{AdmittedSample, PriceBand, RoughPriceEstimator};

/// A small deterministic generator for bootstrap resampling.
#[derive(Debug, Clone)]
pub struct BootstrapRng {
    state: u64,
}

impl BootstrapRng {
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        Self {
            state: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
        }
    }

    pub const fn next_u64(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    #[allow(clippy::cast_possible_truncation)]
    pub const fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}

/// A price from the unconfirmed pool, with the spread of its bootstrap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MempoolEstimate {
    pub price: f64,
    pub band_low: f64,
    pub band_high: f64,
    pub outputs: usize,
    pub successful_draws: usize,
}

/// Prices a mempool sample inside a chain-derived band.
///
/// The band is the only outside input, and it comes from confirmed blocks.
/// The spread is the 10th to 90th percentile of prices from resampling the
/// sample's outputs with replacement.
#[derive(Debug, Clone, Copy)]
pub struct MempoolEstimator {
    min_outputs: usize,
    draws: usize,
}

impl MempoolEstimator {
    #[must_use]
    pub const fn new(min_outputs: usize, draws: usize) -> Self {
        Self { min_outputs, draws }
    }

    #[must_use]
    pub const fn min_outputs(&self) -> usize {
        self.min_outputs
    }

    /// `None` while the sample is too small, when no price falls inside
    /// `band`, or when fewer than half the resamples find one.
    #[must_use]
    pub fn estimate(
        &self,
        sample: &AdmittedSample,
        band: PriceBand,
        rng: &mut BootstrapRng,
    ) -> Option<MempoolEstimate> {
        if sample.len() < self.min_outputs {
            return None;
        }
        let rough = RoughPriceEstimator::new();
        let price = sample.price_within(&rough, band).ok()?.central_price;
        let mut prices = self.resampled_prices(sample, band, &rough, rng);
        if prices.len() * 2 < self.draws {
            return None;
        }
        prices.sort_by(f64::total_cmp);
        Some(MempoolEstimate {
            price,
            band_low: Self::percentile(&prices, 10),
            band_high: Self::percentile(&prices, 90),
            outputs: sample.len(),
            successful_draws: prices.len(),
        })
    }

    fn resampled_prices(
        &self,
        sample: &AdmittedSample,
        band: PriceBand,
        rough: &RoughPriceEstimator,
        rng: &mut BootstrapRng,
    ) -> Vec<f64> {
        let outputs = sample.len();
        (0..self.draws)
            .filter_map(|_| {
                let picks: Vec<usize> = (0..outputs).map(|_| rng.below(outputs)).collect();
                sample
                    .subset(&picks)
                    .price_within(rough, band)
                    .ok()
                    .map(|report| report.central_price)
            })
            .collect()
    }

    fn percentile(sorted: &[f64], percent: usize) -> f64 {
        let index = (sorted.len() - 1) * percent / 100;
        sorted[index]
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use delphi_oracle::{AdmittedSample, PriceBand};

    use super::{BootstrapRng, MempoolEstimator};
    use crate::synthetic_market::SyntheticMarket;

    struct SampleFixture;

    impl SampleFixture {
        fn at_price(price: f64, outputs: usize, seed: u64) -> AdmittedSample {
            SyntheticMarket::new(price, seed).sample(outputs)
        }
    }

    #[test]
    fn the_generator_is_deterministic_and_bounded() {
        let mut first = BootstrapRng::seeded(7);
        let mut second = BootstrapRng::seeded(7);

        assert_eq!(first.next_u64(), second.next_u64());
        assert!((0..1_000).all(|_| first.below(13) < 13));
        assert_eq!(first.below(0), 0);
    }

    #[test]
    fn different_seeds_give_different_streams() {
        assert_ne!(
            BootstrapRng::seeded(1).next_u64(),
            BootstrapRng::seeded(2).next_u64()
        );
    }

    #[test]
    fn a_thin_sample_yields_no_estimate() {
        let sample = SampleFixture::at_price(80_000.0, 50, 1);
        let band = PriceBand::around(80_000.0, 0.15).expect("band");

        assert!(
            MempoolEstimator::new(200, 20)
                .estimate(&sample, band, &mut BootstrapRng::seeded(1))
                .is_none()
        );
    }

    #[test]
    fn a_round_usd_sample_is_priced_inside_the_band_with_a_spread() {
        let sample = SampleFixture::at_price(80_000.0, 600, 3);
        let band = PriceBand::around(82_000.0, 0.15).expect("band");
        let started = Instant::now();
        let estimate = MempoolEstimator::new(200, 30)
            .estimate(&sample, band, &mut BootstrapRng::seeded(9))
            .expect("estimate");
        eprintln!("30 draws over 600 outputs took {:?}", started.elapsed());

        assert!(band.contains(estimate.price));
        assert!((estimate.price - 80_000.0).abs() / 80_000.0 < 0.02, "{estimate:?}");
        assert!(estimate.band_low <= estimate.band_high);
        assert!(estimate.band_low > 70_000.0 && estimate.band_high < 95_000.0);
        assert_eq!(estimate.outputs, 600);
        assert!(estimate.successful_draws * 2 >= 30);
    }

    #[test]
    fn a_band_that_excludes_the_true_price_does_not_return_it() {
        let sample = SampleFixture::at_price(80_000.0, 600, 3);
        let band = PriceBand::around(120_000.0, 0.05).expect("band");
        let estimate = MempoolEstimator::new(200, 10).estimate(&sample, band, &mut BootstrapRng::seeded(1));

        assert!(estimate.is_none_or(|found| band.contains(found.price)));
    }

    #[test]
    fn the_same_seed_reproduces_the_estimate() {
        let sample = SampleFixture::at_price(80_000.0, 400, 5);
        let band = PriceBand::around(80_000.0, 0.15).expect("band");
        let estimator = MempoolEstimator::new(200, 15);

        assert_eq!(
            estimator.estimate(&sample, band, &mut BootstrapRng::seeded(4)),
            estimator.estimate(&sample, band, &mut BootstrapRng::seeded(4))
        );
    }
}
