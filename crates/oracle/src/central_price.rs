use std::collections::HashSet;

use crate::IntradayPricePoints;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CentralPrice {
    pub price: f64,
    pub deviation: f64,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CentralPriceFinder;

impl CentralPriceFinder {
    pub const TIGHT_RANGE: f64 = 0.05;
    pub const DEVIATION_RANGE: f64 = 0.10;

    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn find(&self, points: &IntradayPricePoints, rough_price: f64) -> Option<CentralPrice> {
        let prices: Vec<f64> = points.as_slice().iter().map(|point| point.price).collect();
        let mut center = rough_price;
        let mut seen = HashSet::new();
        loop {
            let (next, _, _) = Self::central_within(&prices, center, Self::TIGHT_RANGE)?;
            if next.to_bits() == center.to_bits() || !seen.insert(next.to_bits()) {
                center = next;
                break;
            }
            center = next;
        }
        let (_, median_deviation, sample_count) =
            Self::central_within(&prices, center, Self::DEVIATION_RANGE)?;
        Some(CentralPrice {
            price: center,
            deviation: median_deviation / (2.0 * Self::DEVIATION_RANGE * center),
            sample_count,
        })
    }

    fn central_within(prices: &[f64], center: f64, range: f64) -> Option<(f64, f64, usize)> {
        let lower = center * (1.0 - range);
        let upper = center * (1.0 + range);
        let mut included: Vec<f64> = prices
            .iter()
            .copied()
            .filter(|price| *price > lower && *price < upper)
            .collect();
        if included.is_empty() {
            return None;
        }
        included.sort_by(f64::total_cmp);
        let central = included[(included.len() - 1) / 2];
        let mut deviations: Vec<f64> = included
            .iter()
            .map(|price| (price - central).abs())
            .collect();
        deviations.sort_by(f64::total_cmp);
        let deviation = if deviations.len().is_multiple_of(2) {
            deviations[deviations.len() / 2 - 1].midpoint(deviations[deviations.len() / 2])
        } else {
            deviations[deviations.len() / 2]
        };
        Some((central, deviation, included.len()))
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};

    use crate::IntradayPricePoints;

    use super::CentralPriceFinder;

    #[test]
    fn converges_on_median_price_and_deviation() {
        let mut points = IntradayPricePoints::new();
        for sats in [99_011, 100_011, 101_011] {
            points.insert(
                Satoshi::new(sats),
                100_000.0,
                BlockHeight::new(1),
                BlockTime::new(1),
            );
        }

        let result = CentralPriceFinder::new()
            .find(&points, 100_000.0)
            .expect("points surround rough estimate");

        assert!(
            (result.price - 99_989.001_21).abs() < 0.01,
            "price: {}",
            result.price
        );
        assert_eq!(result.sample_count, 3);
        assert!(result.deviation > 0.0);
    }

    #[test]
    fn rejects_empty_points() {
        assert!(
            CentralPriceFinder::new()
                .find(&IntradayPricePoints::new(), 100_000.0)
                .is_none()
        );
    }
}
