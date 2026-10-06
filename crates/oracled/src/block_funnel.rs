use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};
use mantle_oracle::{CentralPriceFinder, IntradayPricePoints};

/// How many of a block's admitted outputs the price model used, measured against one price:
/// those within reach of a round-USD amount, and those whose implied price lies inside the
/// central finder's band around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockFunnel {
    pub near_round_usd: usize,
    pub inside_band: usize,
}

impl BlockFunnel {
    #[must_use]
    pub fn of(amounts: &[Satoshi], price: f64) -> Self {
        let lower = price * (1.0 - CentralPriceFinder::DEVIATION_RANGE);
        let upper = price * (1.0 + CentralPriceFinder::DEVIATION_RANGE);
        let mut funnel = Self {
            near_round_usd: 0,
            inside_band: 0,
        };
        for amount in amounts {
            let mut points = IntradayPricePoints::new();
            points.insert(*amount, price, BlockHeight::new(0), BlockTime::new(0));
            if points.is_empty() {
                continue;
            }
            funnel.near_round_usd += 1;
            if points.as_slice().iter().any(|point| point.price > lower && point.price < upper) {
                funnel.inside_band += 1;
            }
        }
        funnel
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;

    use super::BlockFunnel;

    #[test]
    fn outputs_are_sorted_by_how_close_they_sit_to_a_round_dollar_amount() {
        let price = 100_000.0;
        let amounts = [
            Satoshi::new(100_300),
            Satoshi::new(118_000),
            Satoshi::new(135_000),
            Satoshi::new(100_000),
            Satoshi::new(2_000_000_000),
        ];
        let funnel = BlockFunnel::of(&amounts, price);

        assert_eq!(funnel.near_round_usd, 3);
        assert_eq!(funnel.inside_band, 1);
    }

    #[test]
    fn no_outputs_fill_nothing() {
        assert_eq!(BlockFunnel::of(&[], 50_000.0), BlockFunnel { near_round_usd: 0, inside_band: 0 });
    }
}
