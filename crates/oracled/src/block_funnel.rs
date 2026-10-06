use mantle::amount::Satoshi;

use crate::OutputReading;

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
        let readings: Vec<OutputReading> = amounts.iter().map(|amount| OutputReading::of(*amount, price)).collect();
        Self {
            near_round_usd: readings.iter().filter(|reading| reading.near_round_usd()).count(),
            inside_band: readings.iter().filter(|reading| reading.inside_band).count(),
        }
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
