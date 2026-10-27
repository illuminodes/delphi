/// Peak and mean of a run of evenly spaced readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SampleSeries {
    count: u64,
    sum: u128,
    peak: u64,
}

impl SampleSeries {
    pub fn add(&mut self, value: u64) {
        self.count += 1;
        self.sum += u128::from(value);
        self.peak = self.peak.max(value);
    }

    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    #[must_use]
    pub const fn peak(&self) -> u64 {
        self.peak
    }

    /// The mean rounded down; zero with no readings.
    #[must_use]
    pub fn mean(&self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        u64::try_from(self.sum / u128::from(self.count)).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::SampleSeries;

    #[test]
    fn peak_and_mean_follow_the_readings() {
        let mut series = SampleSeries::default();
        for value in [10, 40, 20, 30] {
            series.add(value);
        }

        assert_eq!(series.count(), 4);
        assert_eq!(series.peak(), 40);
        assert_eq!(series.mean(), 25);
    }

    #[test]
    fn an_empty_series_is_zero() {
        let series = SampleSeries::default();

        assert_eq!(series.peak(), 0);
        assert_eq!(series.mean(), 0);
    }

    #[test]
    fn large_readings_do_not_overflow() {
        let mut series = SampleSeries::default();
        series.add(u64::MAX);
        series.add(u64::MAX);

        assert_eq!(series.mean(), u64::MAX);
    }
}
