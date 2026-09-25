use mantle_oracle::{PriceStencil, RoughPriceEstimator};

/// One rough-estimator configuration to compare: the smooth-term weight and
/// the slide below which the smooth term applies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StencilVariant {
    pub smooth_weight: f64,
    pub smooth_below: i32,
}

impl StencilVariant {
    #[must_use]
    pub const fn production() -> Self {
        Self {
            smooth_weight: PriceStencil::DEFAULT_SMOOTH_WEIGHT,
            smooth_below: RoughPriceEstimator::DEFAULT_SMOOTH_BELOW,
        }
    }

    /// Every pairing of `weights` with `smooth_below`, production first.
    #[must_use]
    pub fn grid(weights: &[f64], smooth_below: &[i32]) -> Vec<Self> {
        let mut variants = vec![Self::production()];
        for below in smooth_below {
            for weight in weights {
                let variant = Self {
                    smooth_weight: *weight,
                    smooth_below: *below,
                };
                if !variants.contains(&variant) {
                    variants.push(variant);
                }
            }
        }
        variants
    }

    #[must_use]
    pub fn estimator(self) -> RoughPriceEstimator {
        RoughPriceEstimator::with_stencil(PriceStencil::with_smooth_weight(self.smooth_weight))
            .with_smooth_below(self.smooth_below)
    }

    #[must_use]
    pub fn label(self) -> String {
        format!("w={:<5} <{}", self.smooth_weight, self.smooth_below)
    }
}

#[cfg(test)]
mod tests {
    use super::StencilVariant;

    #[test]
    fn grid_leads_with_production_and_skips_its_duplicate() {
        let grid = StencilVariant::grid(&[0.65, 2.0], &[150, 201]);
        assert_eq!(grid[0], StencilVariant::production());
        assert_eq!(grid.len(), 4);
    }

    #[test]
    fn estimator_carries_the_weight() {
        let variant = StencilVariant {
            smooth_weight: 3.0,
            smooth_below: 201,
        };
        let estimator = variant.estimator();
        assert!((estimator.stencil().smooth_weight() - 3.0).abs() < f64::EPSILON);
    }
}
