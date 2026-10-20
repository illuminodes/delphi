use mantle::difficulty::BlockHeight;

use crate::ChainQuorum;

/// The hourly chain price that bounds the mempool estimate, with the block it
/// was computed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub price: f64,
    pub height: BlockHeight,
}

impl Anchor {
    /// Largest ratio between the hourly and the daily price, either way,
    /// before the hourly price is not trusted as an anchor.
    pub const MAX_DAILY_RATIO: f64 = 1.25;
    /// Blocks after which an anchor that was not refreshed is dropped. An
    /// anchor is computed at the settled height, at least
    /// [`ChainQuorum::DEPTH`] below the tip, so that depth is allowed on top
    /// of the six blocks it may go without a refresh.
    pub const MAX_AGE_BLOCKS: u32 = ChainQuorum::DEPTH + 6;

    /// The hourly price as an anchor at `height`, when the daily price agrees
    /// with it within [`Self::MAX_DAILY_RATIO`]. Without a daily price the
    /// hourly one is taken as it is.
    #[must_use]
    pub fn checked(hourly: f64, daily: Option<f64>, height: BlockHeight) -> Option<Self> {
        if !(hourly.is_finite() && hourly > 0.0) {
            return None;
        }
        let agrees = daily.is_none_or(|daily| {
            let ratio = hourly / daily;
            ratio.is_finite() && (1.0 / Self::MAX_DAILY_RATIO..=Self::MAX_DAILY_RATIO).contains(&ratio)
        });
        agrees.then_some(Self { price: hourly, height })
    }

    /// Blocks between the anchor and `tip`.
    #[must_use]
    pub fn age(self, tip: BlockHeight) -> u32 {
        tip.to_u32().saturating_sub(self.height.to_u32())
    }

    #[must_use]
    pub fn is_fresh_at(self, tip: BlockHeight) -> bool {
        self.age(tip) <= Self::MAX_AGE_BLOCKS
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockHeight;

    use super::Anchor;

    #[test]
    fn an_hourly_price_near_the_daily_one_anchors() {
        let anchor = Anchor::checked(80_000.0, Some(78_000.0), BlockHeight::new(10));
        assert_eq!(anchor.map(|anchor| anchor.price), Some(80_000.0));
    }

    #[test]
    fn an_anchor_at_the_settled_height_is_fresh_at_the_tip_above_it() {
        let settled = BlockHeight::new(100);
        let anchor = Anchor::checked(80_000.0, None, settled).expect("anchor");

        assert!(anchor.is_fresh_at(BlockHeight::new(100 + crate::ChainQuorum::DEPTH + 1)));
    }

    #[test]
    fn an_hourly_price_off_by_an_octave_is_refused() {
        assert!(Anchor::checked(160_000.0, Some(80_000.0), BlockHeight::new(10)).is_none());
        assert!(Anchor::checked(40_000.0, Some(80_000.0), BlockHeight::new(10)).is_none());
    }

    #[test]
    fn without_a_daily_price_the_hourly_one_is_taken() {
        assert!(Anchor::checked(80_000.0, None, BlockHeight::new(10)).is_some());
        assert!(Anchor::checked(f64::NAN, None, BlockHeight::new(10)).is_none());
    }

    #[test]
    fn an_anchor_goes_stale_past_its_age_limit() {
        let anchor = Anchor::checked(80_000.0, None, BlockHeight::new(10)).expect("anchor");
        assert!(anchor.is_fresh_at(BlockHeight::new(10 + Anchor::MAX_AGE_BLOCKS)));
        assert!(!anchor.is_fresh_at(BlockHeight::new(11 + Anchor::MAX_AGE_BLOCKS)));
        assert_eq!(anchor.age(BlockHeight::new(9)), 0);
    }
}
