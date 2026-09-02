use mantle::amount::Satoshi;

#[derive(Debug, Clone)]
pub struct LogScaleBins {
    btc_edges: Vec<f64>,
}

impl Default for LogScaleBins {
    fn default() -> Self {
        Self::new()
    }
}

impl LogScaleBins {
    pub const BINS_PER_DECADE: usize = 200;
    pub const FIRST_EXPONENT: i32 = -6;
    pub const LAST_EXPONENT: i32 = 6;
    pub const SATOSHIS_PER_BTC: f64 = 100_000_000.0;

    #[must_use]
    pub fn new() -> Self {
        let mut btc_edges = Vec::with_capacity(12 * Self::BINS_PER_DECADE + 1);
        btc_edges.push(0.0);
        for exponent in Self::FIRST_EXPONENT..Self::LAST_EXPONENT {
            for offset in 0_u32..200 {
                let fraction = f64::from(offset) / 200.0;
                btc_edges.push(10_f64.powf(f64::from(exponent) + fraction));
            }
        }
        Self { btc_edges }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.btc_edges.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.btc_edges.is_empty()
    }

    #[must_use]
    pub fn btc_at(&self, index: usize) -> Option<f64> {
        self.btc_edges.get(index).copied()
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn index_of(&self, amount: Satoshi) -> Option<usize> {
        let btc = amount.to_u64() as f64 / Self::SATOSHIS_PER_BTC;
        let insertion = self.btc_edges.partition_point(|edge| *edge <= btc);
        insertion
            .checked_sub(1)
            .filter(|index| *index < self.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;

    use super::LogScaleBins;

    #[test]
    fn builds_two_hundred_bins_per_decade() {
        let bins = LogScaleBins::new();

        assert_eq!(bins.len(), 2_401);
        assert_eq!(bins.btc_at(0), Some(0.0));
        assert_eq!(bins.btc_at(1), Some(0.000_001));
        assert!((bins.btc_at(201).expect("edge exists") - 0.000_01).abs() < 1e-15);
        assert!((bins.btc_at(1_201).expect("edge exists") - 1.0).abs() < 1e-12);
    }

    #[test]
    fn maps_amounts_to_the_lower_edge() {
        let bins = LogScaleBins::new();

        assert_eq!(bins.index_of(Satoshi::new(100)), Some(1));
        assert_eq!(bins.index_of(Satoshi::new(1_000)), Some(201));
        assert_eq!(bins.index_of(Satoshi::new(100_000_000)), Some(1_201));
        assert_eq!(bins.index_of(Satoshi::new(0)), Some(0));
    }

    #[test]
    fn rejects_amounts_past_the_last_bounded_bin() {
        let bins = LogScaleBins::new();

        assert_eq!(bins.index_of(Satoshi::new(10_000_000_000_000_000)), None);
    }
}
