use mantle::amount::Satoshi;

use crate::LogScaleBins;

#[derive(Debug, Clone)]
pub struct OutputHistogram {
    bins: LogScaleBins,
    counts: Vec<f64>,
}

impl Default for OutputHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputHistogram {
    pub const FIRST_INCLUDED_BIN: usize = 201;
    pub const LAST_INCLUDED_BIN: usize = 1_600;
    pub const MAX_NORMALIZED_COUNT: f64 = 0.008;
    const ROUND_BTC_BINS: [usize; 18] = [
        201, 401, 461, 496, 540, 601, 661, 696, 740, 801, 861, 896, 940, 1_001, 1_061, 1_096,
        1_140, 1_201,
    ];

    #[must_use]
    pub fn new() -> Self {
        let bins = LogScaleBins::new();
        let counts = vec![0.0; bins.len()];
        Self { bins, counts }
    }

    pub fn insert(&mut self, amount: Satoshi) -> bool {
        let Some(index) = self.bins.index_of(amount) else {
            return false;
        };
        self.counts[index] += 1.0;
        true
    }

    pub fn smooth_round_btc_amounts(&mut self) {
        for index in Self::ROUND_BTC_BINS {
            self.counts[index] = self.counts[index - 1].midpoint(self.counts[index + 1]);
        }
    }

    pub fn normalize_and_clip(&mut self) -> bool {
        self.counts[..Self::FIRST_INCLUDED_BIN].fill(0.0);
        self.counts[Self::LAST_INCLUDED_BIN + 1..].fill(0.0);
        let sum: f64 = self.counts[Self::FIRST_INCLUDED_BIN..=Self::LAST_INCLUDED_BIN]
            .iter()
            .sum();
        if sum == 0.0 {
            return false;
        }
        for count in &mut self.counts[Self::FIRST_INCLUDED_BIN..=Self::LAST_INCLUDED_BIN] {
            *count = (*count / sum).min(Self::MAX_NORMALIZED_COUNT);
        }
        true
    }

    #[must_use]
    pub fn counts(&self) -> &[f64] {
        &self.counts
    }

    #[must_use]
    pub fn bins(&self) -> &LogScaleBins {
        &self.bins
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;

    use super::OutputHistogram;

    #[test]
    fn inserts_into_logarithmic_bin() {
        let mut histogram = OutputHistogram::new();

        assert!(histogram.insert(Satoshi::new(1_000)));
        assert!((histogram.counts()[201] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn smooths_round_btc_bin_from_neighbors() {
        let mut histogram = OutputHistogram::new();
        assert!(histogram.insert(Satoshi::new(990)));
        assert!(histogram.insert(Satoshi::new(1_020)));
        assert!(histogram.insert(Satoshi::new(1_020)));

        histogram.smooth_round_btc_amounts();

        assert!((histogram.counts()[201] - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn normalizes_clips_and_clears_excluded_ranges() {
        let mut histogram = OutputHistogram::new();
        for _ in 0..200 {
            assert!(histogram.insert(Satoshi::new(1_000)));
        }
        assert!(histogram.insert(Satoshi::new(2_000)));
        assert!(histogram.insert(Satoshi::new(100)));

        assert!(histogram.normalize_and_clip());

        assert!(histogram.counts()[1].abs() < f64::EPSILON);
        assert!(
            (histogram.counts()[201] - OutputHistogram::MAX_NORMALIZED_COUNT).abs() < f64::EPSILON
        );
    }

    #[test]
    fn reports_empty_normalization() {
        let mut histogram = OutputHistogram::new();

        assert!(!histogram.normalize_and_clip());
    }
}
