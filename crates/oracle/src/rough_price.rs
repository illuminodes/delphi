use crate::{OutputHistogram, PriceStencil, SlideProfile};

#[derive(Debug, Clone)]
pub struct RoughPriceEstimator {
    stencil: PriceStencil,
}

impl Default for RoughPriceEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl RoughPriceEstimator {
    const CENTER_USD_100_BIN: usize = 601;
    const MIN_SLIDE: i32 = -141;
    const MAX_SLIDE: i32 = 201;

    #[must_use]
    pub fn new() -> Self {
        Self {
            stencil: PriceStencil::new(),
        }
    }

    /// The stencil score at every slide the estimate searches.
    #[must_use]
    pub fn profile(&self, histogram: &OutputHistogram) -> Option<SlideProfile> {
        let center_start = self.center_start()?;
        let scores = (Self::MIN_SLIDE..Self::MAX_SLIDE)
            .map(|slide| {
                self.window(histogram, center_start, slide)
                    .map(|curve| self.stencil.score(curve, slide < 150))
            })
            .collect::<Option<Vec<f64>>>()?;
        Some(SlideProfile::new(Self::MIN_SLIDE, scores))
    }

    #[must_use]
    pub fn estimate(&self, histogram: &OutputHistogram) -> Option<f64> {
        self.estimate_from(histogram, &self.profile(histogram)?)
    }

    /// [`Self::estimate`] reusing a profile already computed for `histogram`.
    #[must_use]
    pub fn estimate_from(&self, histogram: &OutputHistogram, profile: &SlideProfile) -> Option<f64> {
        let center_start = self.center_start()?;
        let (best_slide, best_score) = profile.best()?;
        if !best_score.is_finite() || best_score <= 0.0 {
            return None;
        }

        let up_score = self
            .stencil
            .score(self.window(histogram, center_start, best_slide + 1)?, false);
        let down_score = self
            .stencil
            .score(self.window(histogram, center_start, best_slide - 1)?, false);
        let (neighbor, neighbor_score) = if down_score > up_score {
            (-1, down_score)
        } else {
            (1, up_score)
        };
        let best_price = Self::price_at_slide(histogram, best_slide)?;
        let neighbor_price = Self::price_at_slide(histogram, best_slide + neighbor)?;
        let average_score = profile.average();
        let best_weight = best_score - average_score;
        let neighbor_weight = (neighbor_score - average_score).abs();
        let weight_sum = best_weight + neighbor_weight;
        if weight_sum <= f64::EPSILON {
            Some(best_price)
        } else {
            Some((best_weight * best_price + neighbor_weight * neighbor_price) / weight_sum)
        }
    }

    fn center_start(&self) -> Option<usize> {
        Self::CENTER_USD_100_BIN.checked_sub(self.stencil.len().div_ceil(2))
    }

    fn window<'a>(
        &self,
        histogram: &'a OutputHistogram,
        center_start: usize,
        slide: i32,
    ) -> Option<&'a [f64]> {
        let start = usize::try_from(i32::try_from(center_start).ok()? + slide).ok()?;
        histogram.counts().get(start..start + self.stencil.len())
    }

    fn price_at_slide(histogram: &OutputHistogram, slide: i32) -> Option<f64> {
        let index = usize::try_from(
            i32::try_from(Self::CENTER_USD_100_BIN).expect("center bin fits i32") + slide,
        )
        .ok()?;
        let btc = histogram.bins().btc_at(index)?;
        (btc > 0.0).then_some(100.0 / btc)
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;

    use crate::OutputHistogram;

    use super::RoughPriceEstimator;

    #[test]
    fn estimates_price_from_round_usd_outputs() {
        let mut histogram = OutputHistogram::new();
        for _ in 0..100 {
            assert!(histogram.insert(Satoshi::new(100_000)));
        }
        assert!(histogram.normalize_and_clip());

        let estimate = RoughPriceEstimator::new()
            .estimate(&histogram)
            .expect("synthetic histogram has a signal");

        assert!(
            (estimate - 100_000.0).abs() / 100_000.0 < 0.03,
            "estimate: {estimate}"
        );
    }

    #[test]
    fn rejects_empty_histogram() {
        assert!(
            RoughPriceEstimator::new()
                .estimate(&OutputHistogram::new())
                .is_none()
        );
    }
}
