use std::time::{SystemTime, UNIX_EPOCH};

use mantle::difficulty::BlockTime;

/// How long ago the tip header was stamped, by the wall clock.
///
/// Blocks come about every ten minutes, so a tip older than
/// [`Self::STALE_SECS`] means the peers are behind or withholding blocks,
/// or the network has stalled; the price is then old.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TipAge {
    seconds: u64,
}

impl TipAge {
    pub const STALE_SECS: u64 = 2 * 60 * 60;

    /// The age of a tip stamped `tip_time` at `now_unix`; a tip stamped in
    /// the future is zero seconds old.
    #[must_use]
    pub fn at(tip_time: BlockTime, now_unix: u64) -> Self {
        Self {
            seconds: now_unix.saturating_sub(u64::from(tip_time)),
        }
    }

    #[must_use]
    pub fn now(tip_time: BlockTime) -> Self {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        Self::at(tip_time, now_unix)
    }

    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.seconds
    }

    #[must_use]
    pub const fn is_stale(self) -> bool {
        self.seconds > Self::STALE_SECS
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockTime;

    use super::TipAge;

    #[test]
    fn the_age_is_the_wall_clock_minus_the_header_time() {
        assert_eq!(TipAge::at(BlockTime::new(1_000), 1_600).seconds(), 600);
    }

    #[test]
    fn a_tip_from_the_future_is_zero_seconds_old() {
        assert_eq!(TipAge::at(BlockTime::new(2_000), 1_000).seconds(), 0);
    }

    #[test]
    fn a_tip_is_stale_past_two_hours() {
        let stamped = BlockTime::new(1_000_000);

        assert!(!TipAge::at(stamped, 1_000_000 + TipAge::STALE_SECS).is_stale());
        assert!(TipAge::at(stamped, 1_000_001 + TipAge::STALE_SECS).is_stale());
    }
}
