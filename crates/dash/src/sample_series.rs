use crate::{SeriesPoint, SplitMix};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleSeries {
    seed: u64,
    count: usize,
}

impl SampleSeries {
    const START_TIME: i64 = 1_767_225_600;
    const SPACING: i64 = 600;
    const START_HEIGHT: u32 = 930_000;
    const OPENING: f64 = 100_000.0;
    const VOLATILITY: f64 = 0.0035;
    const REVERSION: f64 = 0.008;

    #[must_use]
    pub const fn new(seed: u64, count: usize) -> Self {
        Self { seed, count }
    }

    #[must_use]
    pub fn points(&self) -> Vec<SeriesPoint> {
        let mut random = SplitMix::new(self.seed);
        let mut log_price = 0.0_f64;
        (0..self.count)
            .map(|index| {
                if index > 0 {
                    log_price += Self::VOLATILITY * random.gaussian() - Self::REVERSION * log_price;
                }
                let step = u32::try_from(index).unwrap_or(u32::MAX);
                SeriesPoint::new(
                    Self::OPENING * log_price.exp(),
                    None,
                    0,
                    Some(Self::START_HEIGHT.saturating_add(step)),
                    Self::START_TIME + i64::from(step) * Self::SPACING,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::SampleSeries;

    #[test]
    fn a_series_has_the_requested_length() {
        assert_eq!(SampleSeries::new(1, 144).points().len(), 144);
        assert!(SampleSeries::new(1, 0).points().is_empty());
    }

    #[test]
    fn the_same_seed_draws_the_same_series() {
        assert_eq!(SampleSeries::new(5, 50).points(), SampleSeries::new(5, 50).points());
        assert_ne!(SampleSeries::new(5, 50).points(), SampleSeries::new(6, 50).points());
    }

    #[test]
    fn times_and_heights_advance_one_step_per_point() {
        let points = SampleSeries::new(3, 20).points();

        for pair in points.windows(2) {
            assert_eq!(pair[1].stop_time() - pair[0].stop_time(), 600);
            assert_eq!(pair[1].stop_height().unwrap() - pair[0].stop_height().unwrap(), 1);
        }
    }

    #[test]
    fn every_price_is_plottable() {
        assert!(SampleSeries::new(11, 1_000).points().iter().all(|point| point.is_plottable()));
    }
}
