use crate::IntradayPricePoint;

/// Counts of round-USD implied prices in log-spaced bins centred on a price.
///
/// Bin `i` covers prices from `center * ratio^(i - half)` to the next edge,
/// where `ratio` spreads `bins` bins evenly over `center / (1 + reach)` to
/// `center * (1 + reach)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpliedHistogram {
    center: f64,
    reach: f64,
    counts: Vec<u32>,
}

impl ImpliedHistogram {
    pub const DEFAULT_BINS: usize = 60;
    pub const DEFAULT_REACH: f64 = 0.25;

    #[must_use]
    pub fn build(points: &[IntradayPricePoint], center: f64, reach: f64, bins: usize) -> Option<Self> {
        let usable = center.is_finite() && center > 0.0 && reach.is_finite() && reach > 0.0 && bins > 0;
        if !usable {
            return None;
        }
        let mut histogram = Self {
            center,
            reach,
            counts: vec![0; bins],
        };
        for point in points {
            if let Some(index) = histogram.index_of(point.price) {
                histogram.counts[index] += 1;
            }
        }
        Some(histogram)
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn index_of(&self, price: f64) -> Option<usize> {
        if !price.is_finite() || price <= 0.0 {
            return None;
        }
        let span = 2.0 * (1.0 + self.reach).ln();
        let offset = (price / self.low()).ln() / span;
        if !(0.0..1.0).contains(&offset) {
            return None;
        }
        Some(((offset * self.counts.len() as f64) as usize).min(self.counts.len() - 1))
    }

    #[must_use]
    pub const fn center(&self) -> f64 {
        self.center
    }

    #[must_use]
    pub fn low(&self) -> f64 {
        self.center / (1.0 + self.reach)
    }

    #[must_use]
    pub fn high(&self) -> f64 {
        self.center * (1.0 + self.reach)
    }

    #[must_use]
    pub fn counts(&self) -> &[u32] {
        &self.counts
    }

    #[must_use]
    pub fn total(&self) -> u32 {
        self.counts.iter().sum()
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};

    use super::ImpliedHistogram;
    use crate::IntradayPricePoint;

    struct PointFixture;

    impl PointFixture {
        fn at(prices: &[f64]) -> Vec<IntradayPricePoint> {
            prices
                .iter()
                .map(|price| IntradayPricePoint {
                    price: *price,
                    usd: 100,
                    height: BlockHeight::new(1),
                    time: BlockTime::new(1),
                })
                .collect()
        }
    }

    #[test]
    fn the_center_lands_in_the_middle_bin() {
        let histogram = ImpliedHistogram::build(&PointFixture::at(&[100.0]), 100.0, 0.25, 60).expect("histogram");

        assert_eq!(histogram.counts()[30], 1);
        assert_eq!(histogram.total(), 1);
    }

    #[test]
    fn prices_outside_the_reach_are_not_counted() {
        let histogram =
            ImpliedHistogram::build(&PointFixture::at(&[70.0, 130.0, f64::NAN, -1.0]), 100.0, 0.25, 60).expect("histogram");

        assert_eq!(histogram.total(), 0);
    }

    #[test]
    fn the_edges_are_symmetric_in_ratio() {
        let histogram = ImpliedHistogram::build(&[], 100.0, 0.25, 10).expect("histogram");

        assert!((histogram.low() - 80.0).abs() < 1e-9);
        assert!((histogram.high() - 125.0).abs() < 1e-9);
    }

    #[test]
    fn the_extreme_bins_hold_prices_just_inside_the_edges() {
        let histogram = ImpliedHistogram::build(&PointFixture::at(&[80.01, 124.99]), 100.0, 0.25, 10).expect("histogram");

        assert_eq!(histogram.counts()[0], 1);
        assert_eq!(histogram.counts()[9], 1);
    }

    #[test]
    fn an_unusable_center_gives_no_histogram() {
        assert!(ImpliedHistogram::build(&[], 0.0, 0.25, 10).is_none());
        assert!(ImpliedHistogram::build(&[], 100.0, 0.25, 0).is_none());
    }
}
