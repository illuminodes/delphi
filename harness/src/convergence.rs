/// One sample size, scored over many random draws against a reference price.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvergenceRow {
    sample_size: usize,
    attempted: usize,
    priced: usize,
    median_error: f64,
    p90_error: f64,
    within_one_percent: f64,
    within_five_percent: f64,
}

impl ConvergenceRow {
    #[must_use]
    pub const fn sample_size(&self) -> usize {
        self.sample_size
    }

    #[must_use]
    pub const fn attempted(&self) -> usize {
        self.attempted
    }

    /// Draws that produced a price at all; the rest were too thin to resolve.
    #[must_use]
    pub const fn priced(&self) -> usize {
        self.priced
    }

    #[must_use]
    pub const fn median_error(&self) -> f64 {
        self.median_error
    }

    #[must_use]
    pub const fn p90_error(&self) -> f64 {
        self.p90_error
    }

    #[must_use]
    pub const fn within_one_percent(&self) -> f64 {
        self.within_one_percent
    }

    #[must_use]
    pub const fn within_five_percent(&self) -> f64 {
        self.within_five_percent
    }

    /// Share of attempted draws that both priced and landed inside the band.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn reliability(&self, within: f64) -> f64 {
        if self.attempted == 0 {
            return 0.0;
        }
        let hit_rate = if within <= 0.01 {
            self.within_one_percent
        } else {
            self.within_five_percent
        };
        hit_rate * (self.priced as f64 / self.attempted as f64)
    }
}

#[derive(Debug, Clone)]
pub struct ConvergenceTrial {
    sample_size: usize,
    attempted: usize,
    errors: Vec<f64>,
}

impl ConvergenceTrial {
    #[must_use]
    pub const fn new(sample_size: usize) -> Self {
        Self {
            sample_size,
            attempted: 0,
            errors: Vec::new(),
        }
    }

    pub fn record_attempt(&mut self) {
        self.attempted += 1;
    }

    pub fn record(&mut self, estimate: f64, reference: f64) {
        self.attempted += 1;
        if reference > 0.0 && estimate.is_finite() {
            self.errors.push((estimate - reference).abs() / reference);
        }
    }

    #[must_use]
    pub fn finish(mut self) -> ConvergenceRow {
        self.errors.sort_by(f64::total_cmp);
        let priced = self.errors.len();
        ConvergenceRow {
            sample_size: self.sample_size,
            attempted: self.attempted,
            priced,
            median_error: Self::quantile(&self.errors, 0.5),
            p90_error: Self::quantile(&self.errors, 0.9),
            within_one_percent: Self::share_within(&self.errors, 0.01),
            within_five_percent: Self::share_within(&self.errors, 0.05),
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn quantile(sorted: &[f64], fraction: f64) -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        let rank = (fraction * (sorted.len() - 1) as f64).round();
        let index = (rank as usize).min(sorted.len() - 1);
        sorted[index]
    }

    #[allow(clippy::cast_precision_loss)]
    fn share_within(errors: &[f64], band: f64) -> f64 {
        if errors.is_empty() {
            return 0.0;
        }
        let hits = errors.iter().filter(|error| **error <= band).count();
        hits as f64 / errors.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::ConvergenceTrial;

    #[test]
    fn an_untouched_trial_prices_nothing() {
        let row = ConvergenceTrial::new(100).finish();
        assert_eq!(row.priced(), 0);
        assert_eq!(row.attempted(), 0);
        assert!(row.median_error().is_nan());
    }

    #[test]
    fn failed_attempts_count_but_do_not_score() {
        let mut trial = ConvergenceTrial::new(50);
        trial.record_attempt();
        trial.record(100.0, 100.0);
        let row = trial.finish();
        assert_eq!(row.attempted(), 2);
        assert_eq!(row.priced(), 1);
    }

    #[test]
    fn an_exact_estimate_has_no_error() {
        let mut trial = ConvergenceTrial::new(10);
        trial.record(50_000.0, 50_000.0);
        let row = trial.finish();
        assert!(row.median_error().abs() < f64::EPSILON);
        assert!((row.within_one_percent() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn bands_separate_close_from_distant_estimates() {
        let mut trial = ConvergenceTrial::new(10);
        trial.record(100.5, 100.0);
        trial.record(103.0, 100.0);
        trial.record(120.0, 100.0);
        let row = trial.finish();
        assert!((row.within_one_percent() - 1.0 / 3.0).abs() < 1e-9);
        assert!((row.within_five_percent() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn reliability_discounts_draws_that_never_priced() {
        let mut trial = ConvergenceTrial::new(10);
        trial.record(100.0, 100.0);
        trial.record_attempt();
        let row = trial.finish();
        assert!((row.reliability(0.01) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn quantiles_track_the_error_spread() {
        let mut trial = ConvergenceTrial::new(10);
        for error in 0..10 {
            trial.record(100.0 + f64::from(error), 100.0);
        }
        let row = trial.finish();
        assert!((row.median_error() - 0.05).abs() < 1e-9, "{}", row.median_error());
        assert!((row.p90_error() - 0.08).abs() < 1e-9, "{}", row.p90_error());
    }
}
