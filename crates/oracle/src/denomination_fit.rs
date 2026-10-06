use crate::{CentralPriceFinder, IntradayPricePoint, IntradayPricePoints};

/// How the outputs near one round-USD amount agree with the central price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DenominationFit {
    pub usd: u32,
    pub points: usize,
    pub inside: usize,
    pub median: Option<f64>,
}

impl DenominationFit {
    /// One fit per round-USD amount, in ascending USD order. `inside` counts
    /// points within the central finder's deviation range of `center`, and
    /// `median` is the median of those points.
    #[must_use]
    pub fn per_amount(points: &[IntradayPricePoint], center: f64) -> Vec<Self> {
        IntradayPricePoints::ROUND_USD_AMOUNTS
            .iter()
            .map(|usd| Self::for_amount(points, *usd, center))
            .collect()
    }

    fn for_amount(points: &[IntradayPricePoint], usd: u32, center: f64) -> Self {
        let lower = center * (1.0 - CentralPriceFinder::DEVIATION_RANGE);
        let upper = center * (1.0 + CentralPriceFinder::DEVIATION_RANGE);
        let mine: Vec<f64> = points
            .iter()
            .filter(|point| point.usd == usd)
            .map(|point| point.price)
            .collect();
        let mut inside: Vec<f64> = mine
            .iter()
            .copied()
            .filter(|price| *price > lower && *price < upper)
            .collect();
        inside.sort_by(f64::total_cmp);
        Self {
            usd,
            points: mine.len(),
            inside: inside.len(),
            median: (!inside.is_empty()).then(|| inside[(inside.len() - 1) / 2]),
        }
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};

    use super::DenominationFit;
    use crate::{IntradayPricePoint, IntradayPricePoints};

    struct PointFixture;

    impl PointFixture {
        fn point(usd: u32, price: f64) -> IntradayPricePoint {
            IntradayPricePoint {
                price,
                usd,
                height: BlockHeight::new(1),
                time: BlockTime::new(1),
            }
        }
    }

    #[test]
    fn every_round_amount_gets_a_row_in_order() {
        let fits = DenominationFit::per_amount(&[], 100.0);
        let amounts: Vec<u32> = fits.iter().map(|fit| fit.usd).collect();

        assert_eq!(amounts, IntradayPricePoints::ROUND_USD_AMOUNTS.to_vec());
        assert!(fits.iter().all(|fit| fit.points == 0 && fit.median.is_none()));
    }

    #[test]
    fn points_far_from_the_center_count_but_are_not_inside() {
        let points = [
            PointFixture::point(100, 99.0),
            PointFixture::point(100, 101.0),
            PointFixture::point(100, 120.0),
            PointFixture::point(20, 100.0),
        ];
        let fits = DenominationFit::per_amount(&points, 100.0);
        let hundred = fits.iter().find(|fit| fit.usd == 100).expect("row");

        assert_eq!(hundred.points, 3);
        assert_eq!(hundred.inside, 2);
        assert_eq!(hundred.median, Some(99.0));
    }
}
