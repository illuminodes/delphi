use std::fmt;

/// A confirmed-chain price window, sized in blocks at the 10-minute target.
/// Each window is published as a timeline of prices, one per `STEP_BLOCKS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChainWindow {
    Hourly,
    Daily,
}

impl ChainWindow {
    pub const ALL: [Self; 2] = [Self::Hourly, Self::Daily];
    pub const STEP_BLOCKS: usize = 6;

    #[must_use]
    pub const fn blocks(self) -> usize {
        match self {
            Self::Hourly => 6,
            Self::Daily => 144,
        }
    }

    /// The addressable-event `d` tag under which this window is published.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Hourly => "chain-1h",
            Self::Daily => "chain-1d",
        }
    }

    /// The largest window, which sets the least history a timeline needs.
    #[must_use]
    pub fn deepest() -> Self {
        Self::ALL
            .into_iter()
            .max_by_key(|window| window.blocks())
            .unwrap_or(Self::Daily)
    }
}

impl fmt::Display for ChainWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.tag())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::ChainWindow;

    #[test]
    fn windows_span_an_hour_and_a_day_of_blocks() {
        assert_eq!(ChainWindow::Hourly.blocks(), 6);
        assert_eq!(ChainWindow::Daily.blocks(), 6 * 24);
    }

    #[test]
    fn every_window_has_its_own_tag() {
        let tags: HashSet<_> = ChainWindow::ALL.iter().map(|window| window.tag()).collect();
        assert_eq!(tags.len(), ChainWindow::ALL.len());
    }

    #[test]
    fn the_deepest_window_is_the_daily_one() {
        assert_eq!(ChainWindow::deepest(), ChainWindow::Daily);
    }
}
