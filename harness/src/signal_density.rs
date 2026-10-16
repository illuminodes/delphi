use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};
use delphi_oracle::IntradayPricePoints;

/// How much round-USD signal a set of outputs carries at a known price.
///
/// Every output is fed through the same round-USD matcher the estimator uses,
/// with the true price standing in for the rough estimate. A candidate point
/// is `tight` when it lands within [`Self::TIGHT_BAND`] of the true price.
/// Noise spreads evenly across the ±25% matching band, so the tight share of
/// pure noise is [`Self::UNIFORM_TIGHT_SHARE`]; `enrichment` is the observed
/// tight share over that baseline.
#[derive(Debug, Clone, Default)]
pub struct SignalDensity {
    outputs: u64,
    candidates: u64,
    tight: u64,
}

impl SignalDensity {
    pub const TIGHT_BAND: f64 = 0.01;
    pub const UNIFORM_TIGHT_SHARE: f64 = 2.0 * Self::TIGHT_BAND / 0.5;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn absorb(&mut self, amount: Satoshi, true_price: f64) {
        self.outputs += 1;
        let mut points = IntradayPricePoints::new();
        points.insert(amount, true_price, BlockHeight::new(0), BlockTime::new(0));
        for point in points.as_slice() {
            self.candidates += 1;
            if ((point.price - true_price) / true_price).abs() <= Self::TIGHT_BAND {
                self.tight += 1;
            }
        }
    }

    #[must_use]
    pub const fn outputs(&self) -> u64 {
        self.outputs
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn candidates_per_output(&self) -> f64 {
        Self::ratio(self.candidates, self.outputs)
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn tight_per_output(&self) -> f64 {
        Self::ratio(self.tight, self.outputs)
    }

    #[must_use]
    pub fn tight_share(&self) -> f64 {
        Self::ratio(self.tight, self.candidates)
    }

    #[must_use]
    pub fn enrichment(&self) -> f64 {
        self.tight_share() / Self::UNIFORM_TIGHT_SHARE
    }

    #[allow(clippy::cast_precision_loss)]
    fn ratio(numerator: u64, denominator: u64) -> f64 {
        if denominator == 0 {
            return f64::NAN;
        }
        numerator as f64 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;

    use super::SignalDensity;

    #[test]
    fn an_exact_round_usd_output_is_tight() {
        let mut density = SignalDensity::new();
        density.absorb(Satoshi::new(123_457), 81_000.0);
        assert_eq!(density.outputs(), 1);
        assert!(density.tight_per_output() >= 1.0);
    }

    #[test]
    fn an_off_round_output_is_a_candidate_but_not_tight() {
        let mut density = SignalDensity::new();
        density.absorb(Satoshi::new(135_000 + 777), 81_000.0);
        assert!(density.candidates_per_output() >= 1.0);
        assert!(density.tight_per_output().abs() < f64::EPSILON);
    }

    #[test]
    fn empty_density_reports_nan() {
        let density = SignalDensity::new();
        assert!(density.tight_share().is_nan());
        assert!(density.enrichment().is_nan());
    }

    #[test]
    fn uniform_baseline_is_four_percent() {
        assert!((SignalDensity::UNIFORM_TIGHT_SHARE - 0.04).abs() < 1e-12);
    }
}
