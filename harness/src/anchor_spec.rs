/// Which confirmed blocks price an anchor: the newest `blocks` blocks that
/// are at least `lag` seconds older than the pool's first moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorSpec {
    blocks: usize,
    lag: u32,
}

impl AnchorSpec {
    #[must_use]
    pub const fn new(blocks: usize, lag: u32) -> Self {
        Self { blocks, lag }
    }

    #[must_use]
    pub const fn blocks(self) -> usize {
        self.blocks
    }

    #[must_use]
    pub const fn lag(self) -> u32 {
        self.lag
    }

    #[must_use]
    pub const fn cutoff(self, pool_start: u32) -> u32 {
        pool_start.saturating_sub(self.lag)
    }

    #[must_use]
    pub fn label(self) -> String {
        if self.lag == 0 {
            format!("{} blocks", self.blocks)
        } else {
            format!("{} blocks, {:.1}h stale", self.blocks, f64::from(self.lag) / 3_600.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AnchorSpec;

    #[test]
    fn the_cutoff_moves_back_by_the_lag() {
        assert_eq!(AnchorSpec::new(6, 3_600).cutoff(10_000), 6_400);
    }

    #[test]
    fn the_cutoff_saturates_at_zero() {
        assert_eq!(AnchorSpec::new(6, 3_600).cutoff(100), 0);
    }

    #[test]
    fn a_fresh_anchor_is_labelled_without_staleness() {
        assert_eq!(AnchorSpec::new(6, 0).label(), "6 blocks");
        assert_eq!(AnchorSpec::new(6, 7_200).label(), "6 blocks, 2.0h stale");
    }
}
