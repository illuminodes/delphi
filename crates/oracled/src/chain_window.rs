use std::fmt;

/// A confirmed-chain price window, sized in blocks at the 10-minute target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChainWindow {
    Hourly,
    Daily,
    Weekly,
}

impl ChainWindow {
    pub const ALL: [Self; 3] = [Self::Hourly, Self::Daily, Self::Weekly];

    #[must_use]
    pub const fn blocks(self) -> usize {
        match self {
            Self::Hourly => 6,
            Self::Daily => 144,
            Self::Weekly => 1_008,
        }
    }

    /// The addressable-event `d` tag under which this window is published.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Hourly => "chain-1h",
            Self::Daily => "chain-1d",
            Self::Weekly => "chain-1w",
        }
    }

    /// The largest window, which sets how many blocks of history are kept.
    #[must_use]
    pub fn deepest() -> Self {
        Self::ALL
            .into_iter()
            .max_by_key(|window| window.blocks())
            .unwrap_or(Self::Weekly)
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
    fn windows_span_an_hour_a_day_and_a_week_of_blocks() {
        assert_eq!(ChainWindow::Hourly.blocks(), 6);
        assert_eq!(ChainWindow::Daily.blocks(), 6 * 24);
        assert_eq!(ChainWindow::Weekly.blocks(), 6 * 24 * 7);
    }

    #[test]
    fn every_window_has_its_own_tag() {
        let tags: HashSet<_> = ChainWindow::ALL.iter().map(|window| window.tag()).collect();
        assert_eq!(tags.len(), ChainWindow::ALL.len());
    }

    #[test]
    fn the_deepest_window_is_the_weekly_one() {
        assert_eq!(ChainWindow::deepest(), ChainWindow::Weekly);
    }
}
