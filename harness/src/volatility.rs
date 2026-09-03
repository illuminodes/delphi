/// One window scored against the reference, keeping the sign of the error.
///
/// A rally makes a lagging estimator read low and a falling market makes it
/// read high, so the sign carries information that an absolute deviation
/// discards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolatilityPoint {
    midpoint: u32,
    estimate: f64,
    reference: f64,
    reference_start: f64,
    reference_stop: f64,
    samples: usize,
}

impl VolatilityPoint {
    #[must_use]
    pub const fn new(
        midpoint: u32,
        estimate: f64,
        reference: f64,
        reference_start: f64,
        reference_stop: f64,
        samples: usize,
    ) -> Self {
        Self {
            midpoint,
            estimate,
            reference,
            reference_start,
            reference_stop,
            samples,
        }
    }

    #[must_use]
    pub const fn midpoint(self) -> u32 {
        self.midpoint
    }

    #[must_use]
    pub const fn estimate(self) -> f64 {
        self.estimate
    }

    #[must_use]
    pub const fn reference(self) -> f64 {
        self.reference
    }

    #[must_use]
    pub const fn samples(self) -> usize {
        self.samples
    }

    /// Signed relative error: negative when the oracle reads below the market.
    #[must_use]
    pub fn signed_error(self) -> f64 {
        (self.estimate - self.reference) / self.reference
    }

    #[must_use]
    pub fn absolute_error(self) -> f64 {
        self.signed_error().abs()
    }

    /// How far the market itself moved across the window, as a fraction of the
    /// opening price. This is the volatility the estimator had to track.
    #[must_use]
    pub fn market_move(self) -> f64 {
        if self.reference_start <= 0.0 {
            return f64::NAN;
        }
        (self.reference_stop - self.reference_start) / self.reference_start
    }

    /// Whether the window's own price movement exceeds `threshold`.
    #[must_use]
    pub fn is_volatile(self, threshold: f64) -> bool {
        self.market_move().abs() > threshold
    }

    /// Whether the estimate missed by more than half the reference — the
    /// stencil resolved the wrong log-scale bin rather than being merely
    /// noisy. These are a separate failure mode from tracking error and would
    /// otherwise dominate any mean.
    #[must_use]
    pub fn is_blowup(self) -> bool {
        !self.absolute_error().is_finite() || self.absolute_error() > 0.5
    }
}

/// Collects scored windows and splits them by whether the market was moving,
/// so calm and volatile behaviour can be compared directly.
pub struct VolatilityStudy {
    points: Vec<VolatilityPoint>,
    threshold: f64,
}

impl VolatilityStudy {
    #[must_use]
    pub const fn new(threshold: f64) -> Self {
        Self {
            points: Vec::new(),
            threshold,
        }
    }

    pub fn record(&mut self, point: VolatilityPoint) {
        self.points.push(point);
    }

    #[must_use]
    pub fn points(&self) -> &[VolatilityPoint] {
        &self.points
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    #[must_use]
    pub fn calm(&self) -> VolatilityBand {
        self.band("calm", |point| !point.is_volatile(self.threshold))
    }

    #[must_use]
    pub fn volatile(&self) -> VolatilityBand {
        self.band("volatile", |point| point.is_volatile(self.threshold))
    }

    #[must_use]
    pub fn overall(&self) -> VolatilityBand {
        self.band("all", |_| true)
    }

    fn band(&self, label: &str, keep: impl Fn(&VolatilityPoint) -> bool) -> VolatilityBand {
        let kept: Vec<VolatilityPoint> = self
            .points
            .iter()
            .copied()
            .filter(|point| keep(point))
            .collect();
        VolatilityBand::of(label, &kept)
    }

    /// Correlation between how far the market moved and the signed error.
    ///
    /// A lagging estimator reads low into a rally and high into a selloff, so
    /// systematic lag shows up as a strongly negative correlation.
    #[must_use]
    pub fn lag_correlation(&self) -> f64 {
        let tracked: Vec<&VolatilityPoint> = self
            .points
            .iter()
            .filter(|point| !point.is_blowup() && point.market_move().is_finite())
            .collect();
        let moves: Vec<f64> = tracked.iter().map(|point| point.market_move()).collect();
        let errors: Vec<f64> = tracked.iter().map(|point| point.signed_error()).collect();
        Correlation::pearson(&moves, &errors)
    }
}

/// Summary statistics for one group of windows.
#[derive(Debug, Clone, PartialEq)]
pub struct VolatilityBand {
    label: String,
    count: usize,
    blowups: usize,
    median_absolute: f64,
    median_signed: f64,
    worst_absolute: f64,
    mean_move: f64,
}

impl VolatilityBand {
    /// Statistics are taken over the windows the oracle actually tracked.
    /// Blowups are counted separately rather than averaged in, because a
    /// single 2x bin error swamps a mean built from sub-percent deviations.
    fn of(label: &str, points: &[VolatilityPoint]) -> Self {
        let tracked: Vec<&VolatilityPoint> =
            points.iter().filter(|point| !point.is_blowup()).collect();
        let mut absolute: Vec<f64> = tracked
            .iter()
            .map(|point| point.absolute_error())
            .collect();
        absolute.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let mut signed: Vec<f64> = tracked.iter().map(|point| point.signed_error()).collect();
        signed.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let moves: Vec<f64> = points
            .iter()
            .map(|point| point.market_move().abs())
            .filter(|value| value.is_finite())
            .collect();
        Self {
            label: label.to_string(),
            count: points.len(),
            blowups: points.len() - tracked.len(),
            median_absolute: Statistics::median(&absolute),
            median_signed: Statistics::median(&signed),
            worst_absolute: absolute.last().copied().unwrap_or(f64::NAN),
            mean_move: Statistics::mean(&moves),
        }
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub const fn median_absolute(&self) -> f64 {
        self.median_absolute
    }

    /// Median signed error — the bias. Near zero means errors cancel; a
    /// consistently negative value means the oracle trails the market.
    #[must_use]
    pub const fn median_signed(&self) -> f64 {
        self.median_signed
    }

    /// Windows where the stencil resolved the wrong order of magnitude,
    /// excluded from the tracking statistics above.
    #[must_use]
    pub const fn blowups(&self) -> usize {
        self.blowups
    }

    /// Worst error among the windows that were actually tracked.
    #[must_use]
    pub const fn worst_absolute(&self) -> f64 {
        self.worst_absolute
    }

    #[must_use]
    pub const fn mean_move(&self) -> f64 {
        self.mean_move
    }
}

struct Statistics;

impl Statistics {
    fn mean(values: &[f64]) -> f64 {
        if values.is_empty() {
            return f64::NAN;
        }
        #[allow(clippy::cast_precision_loss)]
        let count = values.len() as f64;
        values.iter().sum::<f64>() / count
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
}

struct Correlation;

impl Correlation {
    fn pearson(left: &[f64], right: &[f64]) -> f64 {
        if left.len() != right.len() || left.len() < 2 {
            return f64::NAN;
        }
        let mean_left = Statistics::mean(left);
        let mean_right = Statistics::mean(right);
        let mut covariance = 0.0;
        let mut variance_left = 0.0;
        let mut variance_right = 0.0;
        for (value_left, value_right) in left.iter().zip(right) {
            let delta_left = value_left - mean_left;
            let delta_right = value_right - mean_right;
            covariance += delta_left * delta_right;
            variance_left += delta_left * delta_left;
            variance_right += delta_right * delta_right;
        }
        if variance_left <= 0.0 || variance_right <= 0.0 {
            return f64::NAN;
        }
        covariance / (variance_left * variance_right).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::{Correlation, VolatilityPoint, VolatilityStudy};

    struct Points;

    impl Points {
        fn calm(midpoint: u32, estimate: f64) -> VolatilityPoint {
            VolatilityPoint::new(midpoint, estimate, 100.0, 100.0, 100.0, 1_000)
        }

        fn rallying(midpoint: u32, estimate: f64) -> VolatilityPoint {
            VolatilityPoint::new(midpoint, estimate, 100.0, 95.0, 105.0, 1_000)
        }
    }

    #[test]
    fn signed_error_keeps_direction() {
        assert!((Points::calm(0, 99.0).signed_error() + 0.01).abs() < 1e-9);
        assert!((Points::calm(0, 101.0).signed_error() - 0.01).abs() < 1e-9);
    }

    #[test]
    fn absolute_error_drops_the_sign() {
        assert!((Points::calm(0, 99.0).absolute_error() - 0.01).abs() < 1e-9);
    }

    #[test]
    fn market_move_measures_the_window_swing() {
        let point = Points::rallying(0, 100.0);
        assert!((point.market_move() - 0.105_263).abs() < 1e-5);
    }

    #[test]
    fn volatility_is_judged_against_the_threshold() {
        assert!(Points::rallying(0, 100.0).is_volatile(0.05));
        assert!(!Points::calm(0, 100.0).is_volatile(0.05));
    }

    #[test]
    fn bands_split_calm_from_volatile_windows() {
        let mut study = VolatilityStudy::new(0.05);
        study.record(Points::calm(0, 100.5));
        study.record(Points::calm(1, 99.5));
        study.record(Points::rallying(2, 97.0));
        let calm = study.calm();
        let volatile = study.volatile();

        assert_eq!(calm.count(), 2);
        assert_eq!(volatile.count(), 1);
        assert_eq!(study.overall().count(), 3);
    }

    #[test]
    fn median_signed_error_reveals_bias() {
        let mut study = VolatilityStudy::new(0.05);
        study.record(Points::calm(0, 98.0));
        study.record(Points::calm(1, 98.0));
        let band = study.calm();

        assert!((band.median_signed() + 0.02).abs() < 1e-9);
        assert!((band.median_absolute() - 0.02).abs() < 1e-9);
    }

    #[test]
    fn cancelling_errors_leave_no_bias() {
        let mut study = VolatilityStudy::new(0.05);
        study.record(Points::calm(0, 98.0));
        study.record(Points::calm(1, 102.0));

        assert!(study.calm().median_signed().abs() < 1e-9);
    }

    #[test]
    fn an_empty_band_reports_nan_without_panicking() {
        let study = VolatilityStudy::new(0.05);
        let band = study.overall();

        assert_eq!(band.count(), 0);
        assert!(band.median_absolute().is_nan());
        assert!(band.median_signed().is_nan());
    }

    #[test]
    fn a_doubled_estimate_counts_as_a_blowup() {
        assert!(Points::calm(0, 200.0).is_blowup());
        assert!(!Points::calm(0, 101.0).is_blowup());
    }

    #[test]
    fn blowups_are_counted_out_of_the_tracking_statistics() {
        let mut study = VolatilityStudy::new(0.05);
        study.record(Points::calm(0, 99.0));
        study.record(Points::calm(1, 101.0));
        study.record(Points::calm(2, 200.0));
        let band = study.calm();

        assert_eq!(band.count(), 3);
        assert_eq!(band.blowups(), 1);
        assert!((band.median_absolute() - 0.01).abs() < 1e-9);
        assert!(band.median_signed().abs() < 1e-9);
        assert!((band.worst_absolute() - 0.01).abs() < 1e-9);
    }

    #[test]
    fn a_blowup_does_not_destroy_the_lag_correlation() {
        let mut study = VolatilityStudy::new(0.05);
        for step in 0..5 {
            let move_size = f64::from(step) * 0.01;
            study.record(VolatilityPoint::new(
                step,
                100.0 * (1.0 - move_size * 0.5),
                100.0,
                100.0,
                100.0 * (1.0 + move_size),
                1_000,
            ));
        }
        study.record(VolatilityPoint::new(9, 200.0, 100.0, 100.0, 100.0, 1_000));

        assert!(study.lag_correlation() < -0.9);
    }

    #[test]
    fn perfect_lag_correlates_negatively() {
        let mut study = VolatilityStudy::new(0.05);
        for step in 0..5 {
            let move_size = f64::from(step) * 0.01;
            let reference = 100.0;
            let stop = reference * (1.0 + move_size);
            let estimate = reference * (1.0 - move_size * 0.5);
            study.record(VolatilityPoint::new(
                step,
                estimate,
                reference,
                reference,
                stop,
                1_000,
            ));
        }

        assert!(study.lag_correlation() < -0.9);
    }

    #[test]
    fn correlation_of_mismatched_lengths_is_nan() {
        assert!(Correlation::pearson(&[1.0, 2.0], &[1.0]).is_nan());
    }

    #[test]
    fn correlation_of_a_constant_series_is_nan() {
        assert!(Correlation::pearson(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]).is_nan());
    }
}
