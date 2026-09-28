/// A multiplicative price interval around a price the oracle already trusts,
/// symmetric in ratio: a tolerance of 0.25 spans 0.8x to 1.25x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceBand {
    low: f64,
    high: f64,
}

impl PriceBand {
    #[must_use]
    pub fn around(anchor: f64, tolerance: f64) -> Option<Self> {
        let usable = anchor.is_finite() && anchor > 0.0 && tolerance.is_finite() && tolerance > 0.0;
        usable.then(|| Self {
            low: anchor / (1.0 + tolerance),
            high: anchor * (1.0 + tolerance),
        })
    }

    #[must_use]
    pub const fn low(self) -> f64 {
        self.low
    }

    #[must_use]
    pub const fn high(self) -> f64 {
        self.high
    }

    #[must_use]
    pub fn contains(self, price: f64) -> bool {
        price >= self.low && price <= self.high
    }
}

#[cfg(test)]
mod tests {
    use super::PriceBand;

    #[test]
    fn the_band_is_symmetric_in_ratio() {
        let band = PriceBand::around(100.0, 0.25).expect("valid band");
        assert!((band.low() - 80.0).abs() < 1e-9);
        assert!((band.high() - 125.0).abs() < 1e-9);
    }

    #[test]
    fn a_band_excludes_the_octave_neighbours_below_full_tolerance() {
        let band = PriceBand::around(100.0, 0.5).expect("valid band");
        assert!(!band.contains(200.0));
        assert!(!band.contains(50.0));
        assert!(band.contains(140.0));
    }

    #[test]
    fn unusable_anchors_and_tolerances_give_no_band() {
        assert!(PriceBand::around(0.0, 0.25).is_none());
        assert!(PriceBand::around(f64::NAN, 0.25).is_none());
        assert!(PriceBand::around(100.0, 0.0).is_none());
        assert!(PriceBand::around(100.0, f64::INFINITY).is_none());
    }
}
