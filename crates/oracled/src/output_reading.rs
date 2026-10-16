use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};
use delphi_oracle::{CentralPriceFinder, IntradayPricePoints};

/// What one admitted output says at a given price: the round dollar amount it most likely paid,
/// the price that amount implies, and whether that price lies inside the central finder's band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputReading {
    pub usd: Option<u32>,
    pub implied: Option<f64>,
    pub inside_band: bool,
}

impl OutputReading {
    #[must_use]
    pub fn of(amount: Satoshi, price: f64) -> Self {
        let mut points = IntradayPricePoints::new();
        points.insert(amount, price, BlockHeight::new(0), BlockTime::new(0));
        let closest = points
            .as_slice()
            .iter()
            .min_by(|left, right| (left.price - price).abs().total_cmp(&(right.price - price).abs()));
        let lower = price * (1.0 - CentralPriceFinder::DEVIATION_RANGE);
        let upper = price * (1.0 + CentralPriceFinder::DEVIATION_RANGE);
        Self {
            usd: closest.map(|point| point.usd),
            implied: closest.map(|point| point.price),
            inside_band: closest.is_some_and(|point| point.price > lower && point.price < upper),
        }
    }

    #[must_use]
    pub const fn near_round_usd(&self) -> bool {
        self.usd.is_some()
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;

    use super::OutputReading;

    #[test]
    fn an_output_near_a_round_amount_names_it_and_its_price() {
        let reading = OutputReading::of(Satoshi::new(100_300), 100_000.0);

        assert_eq!(reading.usd, Some(100));
        assert!(reading.implied.is_some_and(|price| (price - 99_700.9).abs() < 1.0));
        assert!(reading.inside_band);
    }

    #[test]
    fn an_output_far_from_any_round_amount_reads_nothing() {
        let reading = OutputReading::of(Satoshi::new(2_000_000_000), 100_000.0);

        assert!(!reading.near_round_usd());
        assert!(!reading.inside_band);
    }

    #[test]
    fn a_round_number_of_sats_is_not_read_as_dollars() {
        assert!(!OutputReading::of(Satoshi::new(100_000), 100_000.0).near_round_usd());
    }

    #[test]
    fn an_output_near_a_round_amount_can_still_fall_outside_the_band() {
        let reading = OutputReading::of(Satoshi::new(118_000), 100_000.0);

        assert!(reading.near_round_usd());
        assert!(!reading.inside_band);
    }
}
