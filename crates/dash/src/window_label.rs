#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowLabel {
    title: &'static str,
    detail: &'static str,
    rank: u8,
}

impl WindowLabel {
    pub const DAILY: &'static str = "chain-1d";
    const UNKNOWN_RANK: u8 = u8::MAX;
    const RETIRED: [&'static str; 1] = ["chain-1w"];

    #[must_use]
    pub fn is_retired(window: &str) -> bool {
        Self::RETIRED.contains(&window)
    }

    #[must_use]
    pub fn of(window: &str) -> Self {
        match window {
            "mempool" => Self::known("Mempool", "live, since the last block", 0),
            "chain-1h" => Self::known("Hourly", "6 blocks", 1),
            Self::DAILY => Self::known("Daily", "144 blocks", 2),
            _ => Self {
                title: "",
                detail: "",
                rank: Self::UNKNOWN_RANK,
            },
        }
    }

    const fn known(title: &'static str, detail: &'static str, rank: u8) -> Self {
        Self { title, detail, rank }
    }

    #[must_use]
    pub const fn is_known(&self) -> bool {
        self.rank != Self::UNKNOWN_RANK
    }

    #[must_use]
    pub const fn rank(&self) -> u8 {
        self.rank
    }

    #[must_use]
    pub fn title(&self, window: &str) -> String {
        if self.is_known() {
            self.title.to_string()
        } else {
            window.to_string()
        }
    }

    #[must_use]
    pub const fn detail(&self) -> &'static str {
        self.detail
    }
}

#[cfg(test)]
mod tests {
    use super::WindowLabel;

    #[test]
    fn the_daemon_windows_are_ranked_live_first_then_by_length() {
        let ranks: Vec<u8> = ["mempool", "chain-1h", "chain-1d"]
            .iter()
            .map(|window| WindowLabel::of(window).rank())
            .collect();

        assert_eq!(ranks, vec![0, 1, 2]);
    }

    #[test]
    fn an_unknown_window_keeps_its_own_name_and_sorts_last() {
        let label = WindowLabel::of("chain-4h");

        assert!(!label.is_known());
        assert_eq!(label.title("chain-4h"), "chain-4h");
        assert!(label.rank() > WindowLabel::of("chain-1d").rank());
    }

    #[test]
    fn a_known_window_gets_a_readable_title() {
        assert_eq!(WindowLabel::of("chain-1d").title("chain-1d"), "Daily");
        assert_eq!(WindowLabel::of("chain-1d").detail(), "144 blocks");
    }

    #[test]
    fn the_weekly_window_is_retired() {
        assert!(WindowLabel::is_retired("chain-1w"));
        assert!(!WindowLabel::is_retired("chain-1d"));
    }
}
