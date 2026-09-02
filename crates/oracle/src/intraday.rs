use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntradayPricePoint {
    pub price: f64,
    pub height: BlockHeight,
    pub time: BlockTime,
}

#[derive(Debug, Clone, Default)]
pub struct IntradayPricePoints {
    points: Vec<IntradayPricePoint>,
}

impl IntradayPricePoints {
    const ROUND_USD_AMOUNTS: [u32; 14] = [
        5, 10, 15, 20, 25, 30, 40, 50, 100, 150, 200, 300, 500, 1_000,
    ];
    const WIDE_RANGE: f64 = 0.25;
    const ROUND_SAT_TOLERANCE: f64 = 0.000_1;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn insert(
        &mut self,
        amount: Satoshi,
        rough_price: f64,
        height: BlockHeight,
        time: BlockTime,
    ) {
        let btc = amount.to_u64() as f64 / 100_000_000.0;
        if Self::is_micro_round(amount) {
            return;
        }
        for usd in Self::ROUND_USD_AMOUNTS {
            let expected = f64::from(usd) / rough_price;
            if btc > expected * (1.0 - Self::WIDE_RANGE)
                && btc < expected * (1.0 + Self::WIDE_RANGE)
            {
                self.points.push(IntradayPricePoint {
                    price: f64::from(usd) / btc,
                    height,
                    time,
                });
            }
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[IntradayPricePoint] {
        &self.points
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    #[allow(clippy::cast_precision_loss)]
    fn is_micro_round(amount: Satoshi) -> bool {
        let sats = amount.to_u64();
        let step = match sats {
            5_000..100_000 => 1_000,
            100_000..1_000_000 => 10_000,
            1_000_000..10_000_000 => 100_000,
            10_000_000..100_000_000 => 1_000_000,
            _ => return false,
        };
        let nearest = ((sats + step / 2) / step) * step;
        let distance = sats.abs_diff(nearest) as f64;
        distance < nearest as f64 * Self::ROUND_SAT_TOLERANCE
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};

    use super::IntradayPricePoints;

    #[test]
    fn creates_price_point_near_round_usd_amount() {
        let mut points = IntradayPricePoints::new();
        points.insert(
            Satoshi::new(101_001),
            100_000.0,
            BlockHeight::new(800_000),
            BlockTime::new(1_700_000_000),
        );

        assert_eq!(points.len(), 1);
        assert!((points.as_slice()[0].price - 99_008.920_703).abs() < 0.01);
    }

    #[test]
    fn excludes_micro_round_satoshi_amounts() {
        let mut points = IntradayPricePoints::new();
        points.insert(
            Satoshi::new(100_000),
            100_000.0,
            BlockHeight::new(800_000),
            BlockTime::new(1_700_000_000),
        );

        assert!(points.is_empty());
    }
}
