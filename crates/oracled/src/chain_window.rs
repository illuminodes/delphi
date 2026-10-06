use std::fmt;

/// A confirmed-chain price window, sized in blocks at the 10-minute target.
/// Each block gets one audited price per window, over the blocks ending there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChainWindow {
    Hourly,
    Daily,
}

impl ChainWindow {
    pub const ALL: [Self; 2] = [Self::Hourly, Self::Daily];

    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|window| window.tag() == tag)
    }

    #[must_use]
    pub const fn blocks(self) -> usize {
        match self {
            Self::Hourly => 6,
            Self::Daily => 144,
        }
    }

    /// The window name carried in every note about this window.
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
    fn a_tag_names_its_window() {
        for window in ChainWindow::ALL {
            assert_eq!(ChainWindow::from_tag(window.tag()), Some(window));
        }
        assert_eq!(ChainWindow::from_tag("chain-1w"), None);
    }

    #[test]
    fn the_deepest_window_is_the_daily_one() {
        assert_eq!(ChainWindow::deepest(), ChainWindow::Daily);
    }
}
