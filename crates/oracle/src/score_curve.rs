use crate::{OutputHistogram, RoughPriceEstimator, SlideProfile};

/// The rough stage's score at every candidate price, scaled so the profile
/// average is 0 and the best candidate is 1.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreCurve {
    prices: Vec<f64>,
    scores: Vec<f64>,
}

impl ScoreCurve {
    #[must_use]
    pub fn from_profile(histogram: &OutputHistogram, profile: &SlideProfile) -> Option<Self> {
        let (_, best) = profile.best()?;
        let average = profile.average();
        let lead = best - average;
        if !lead.is_finite() || lead <= f64::EPSILON {
            return None;
        }
        let (prices, scores) = (profile.first_slide()..)
            .zip(profile.scores())
            .filter_map(|(slide, score)| {
                let price = RoughPriceEstimator::price_at_slide(histogram, slide)?;
                Some((price, (score - average) / lead))
            })
            .unzip();
        Some(Self { prices, scores })
    }

    /// Candidate prices, in the estimator's slide order.
    #[must_use]
    pub fn prices(&self) -> &[f64] {
        &self.prices
    }

    #[must_use]
    pub fn scores(&self) -> &[f64] {
        &self.scores
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.prices.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;

    use super::ScoreCurve;
    use crate::{OutputHistogram, RoughPriceEstimator, SlideProfile};

    struct CurveFixture;

    impl CurveFixture {
        fn histogram() -> OutputHistogram {
            let mut histogram = OutputHistogram::new();
            for _ in 0..100 {
                assert!(histogram.insert(Satoshi::new(100_000)));
            }
            assert!(histogram.normalize_and_clip());
            histogram
        }
    }

    #[test]
    fn the_best_candidate_scores_one() {
        let histogram = CurveFixture::histogram();
        let profile = RoughPriceEstimator::new().profile(&histogram).expect("profile");
        let curve = ScoreCurve::from_profile(&histogram, &profile).expect("curve");
        let top = curve.scores().iter().copied().fold(f64::MIN, f64::max);

        assert!((top - 1.0).abs() < 1e-12, "{top}");
        assert_eq!(curve.len(), profile.scores().len());
    }

    #[test]
    fn prices_fall_as_the_slide_rises() {
        let histogram = CurveFixture::histogram();
        let profile = RoughPriceEstimator::new().profile(&histogram).expect("profile");
        let curve = ScoreCurve::from_profile(&histogram, &profile).expect("curve");

        assert!(curve.prices().windows(2).all(|pair| pair[1] < pair[0]));
    }

    #[test]
    fn a_flat_profile_has_no_curve() {
        let histogram = CurveFixture::histogram();
        let flat = SlideProfile::new(0, vec![1.0; 50]);

        assert!(ScoreCurve::from_profile(&histogram, &flat).is_none());
    }
}
