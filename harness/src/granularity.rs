#[derive(Debug, Clone, PartialEq)]
pub struct GranularityRow {
    label: String,
    duration: u32,
    attempted: usize,
    scored: usize,
    median_deviation: f64,
    p90_deviation: f64,
    worst_deviation: f64,
    within_one_percent: f64,
    blowups: usize,
    mean_samples: f64,
    mean_blocks: f64,
}

impl GranularityRow {
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn duration(&self) -> u32 {
        self.duration
    }

    #[must_use]
    pub const fn attempted(&self) -> usize {
        self.attempted
    }

    #[must_use]
    pub const fn scored(&self) -> usize {
        self.scored
    }

    #[must_use]
    pub const fn median_deviation(&self) -> f64 {
        self.median_deviation
    }

    #[must_use]
    pub const fn p90_deviation(&self) -> f64 {
        self.p90_deviation
    }

    #[must_use]
    pub const fn worst_deviation(&self) -> f64 {
        self.worst_deviation
    }

    /// Share of scored windows landing within one percent of the reference.
    #[must_use]
    pub const fn within_one_percent(&self) -> f64 {
        self.within_one_percent
    }

    /// Windows that missed by more than half the reference price — the oracle
    /// resolved the wrong order of magnitude rather than being merely noisy.
    #[must_use]
    pub const fn blowups(&self) -> usize {
        self.blowups
    }

    #[must_use]
    pub const fn mean_samples(&self) -> f64 {
        self.mean_samples
    }

    #[must_use]
    pub const fn mean_blocks(&self) -> f64 {
        self.mean_blocks
    }
}

/// Accumulates per-window deviations for one window duration and reduces them
/// to a single comparable row.
pub struct GranularitySweep {
    label: String,
    duration: u32,
    attempted: usize,
    deviations: Vec<f64>,
    samples: Vec<f64>,
    blocks: Vec<f64>,
}

impl GranularitySweep {
    #[must_use]
    pub fn new(label: impl Into<String>, duration: u32) -> Self {
        Self {
            label: label.into(),
            duration,
            attempted: 0,
            deviations: Vec::new(),
            samples: Vec::new(),
            blocks: Vec::new(),
        }
    }

    pub fn record_attempt(&mut self) {
        self.attempted += 1;
    }

    pub fn record(&mut self, estimate: f64, reference: f64, samples: usize, blocks: usize) {
        self.attempted += 1;
        if reference <= 0.0 || !estimate.is_finite() {
            return;
        }
        self.deviations
            .push(((estimate - reference) / reference).abs());
        #[allow(clippy::cast_precision_loss)]
        self.samples.push(samples as f64);
        #[allow(clippy::cast_precision_loss)]
        self.blocks.push(blocks as f64);
    }

    #[must_use]
    pub fn finish(mut self) -> GranularityRow {
        self.deviations
            .sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        GranularityRow {
            label: self.label,
            duration: self.duration,
            attempted: self.attempted,
            scored: self.deviations.len(),
            median_deviation: Self::median(&self.deviations),
            p90_deviation: Self::quantile(&self.deviations, 0.9),
            worst_deviation: self.deviations.last().copied().unwrap_or(f64::NAN),
            within_one_percent: Self::share_below(&self.deviations, 0.01),
            blowups: self
                .deviations
                .iter()
                .filter(|deviation| **deviation > 0.5)
                .count(),
            mean_samples: Self::mean(&self.samples),
            mean_blocks: Self::mean(&self.blocks),
        }
    }

    fn quantile(sorted: &[f64], fraction: f64) -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        #[allow(clippy::cast_precision_loss)]
        let last = (sorted.len() - 1) as f64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let index = (last * fraction).round() as usize;
        sorted[index.min(sorted.len() - 1)]
    }

    fn share_below(deviations: &[f64], threshold: f64) -> f64 {
        if deviations.is_empty() {
            return f64::NAN;
        }
        let hits = deviations
            .iter()
            .filter(|deviation| **deviation <= threshold)
            .count();
        #[allow(clippy::cast_precision_loss)]
        let share = hits as f64 / deviations.len() as f64;
        share
    }

    fn median(sorted: &[f64]) -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        let middle = sorted.len() / 2;
        if sorted.len().is_multiple_of(2) {
            f64::midpoint(sorted[middle - 1], sorted[middle])
        } else {
            sorted[middle]
        }
    }

    fn mean(values: &[f64]) -> f64 {
        if values.is_empty() {
            return f64::NAN;
        }
        #[allow(clippy::cast_precision_loss)]
        let count = values.len() as f64;
        values.iter().sum::<f64>() / count
    }
}

#[cfg(test)]
mod tests {
    use super::GranularitySweep;

    #[test]
    fn reduces_deviations_to_median_and_worst() {
        let mut sweep = GranularitySweep::new("1h", 3_600);
        sweep.record(101.0, 100.0, 10, 6);
        sweep.record(103.0, 100.0, 20, 6);
        sweep.record(102.0, 100.0, 30, 6);
        let row = sweep.finish();

        assert_eq!(row.scored(), 3);
        assert_eq!(row.attempted(), 3);
        assert!((row.median_deviation() - 0.02).abs() < 1e-9);
        assert!((row.worst_deviation() - 0.03).abs() < 1e-9);
        assert!((row.mean_samples() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn median_of_an_even_count_is_the_midpoint() {
        let mut sweep = GranularitySweep::new("1h", 3_600);
        sweep.record(101.0, 100.0, 1, 1);
        sweep.record(103.0, 100.0, 1, 1);
        let row = sweep.finish();

        assert!((row.median_deviation() - 0.02).abs() < 1e-9);
    }

    #[test]
    fn attempts_without_a_score_still_count() {
        let mut sweep = GranularitySweep::new("5m", 300);
        sweep.record_attempt();
        sweep.record_attempt();
        sweep.record(100.0, 100.0, 5, 1);
        let row = sweep.finish();

        assert_eq!(row.attempted(), 3);
        assert_eq!(row.scored(), 1);
    }

    #[test]
    fn an_empty_sweep_reports_nan_without_panicking() {
        let row = GranularitySweep::new("1d", 86_400).finish();

        assert_eq!(row.scored(), 0);
        assert!(row.median_deviation().is_nan());
        assert!(row.p90_deviation().is_nan());
        assert!(row.worst_deviation().is_nan());
        assert!(row.within_one_percent().is_nan());
    }

    #[test]
    fn counts_windows_within_one_percent() {
        let mut sweep = GranularitySweep::new("1h", 3_600);
        sweep.record(100.5, 100.0, 1, 1);
        sweep.record(100.2, 100.0, 1, 1);
        sweep.record(140.0, 100.0, 1, 1);
        sweep.record(180.0, 100.0, 1, 1);
        let row = sweep.finish();

        assert!((row.within_one_percent() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn counts_order_of_magnitude_blowups() {
        let mut sweep = GranularitySweep::new("15m", 900);
        sweep.record(100.0, 100.0, 1, 1);
        sweep.record(250.0, 78_000.0, 1, 1);
        sweep.record(300.0, 78_000.0, 1, 1);
        let row = sweep.finish();

        assert_eq!(row.blowups(), 2);
    }

    #[test]
    fn p90_sits_in_the_upper_tail() {
        let mut sweep = GranularitySweep::new("1h", 3_600);
        for percent in 1..=10 {
            sweep.record(100.0 + f64::from(percent), 100.0, 1, 1);
        }
        let row = sweep.finish();

        assert!(row.p90_deviation() > row.median_deviation());
        assert!(row.p90_deviation() <= row.worst_deviation());
    }

    #[test]
    fn ignores_a_nonpositive_reference() {
        let mut sweep = GranularitySweep::new("1h", 3_600);
        sweep.record(100.0, 0.0, 5, 1);
        let row = sweep.finish();

        assert_eq!(row.attempted(), 1);
        assert_eq!(row.scored(), 0);
    }
}
