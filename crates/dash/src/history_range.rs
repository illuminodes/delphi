/// How many days of audited history the dashboard asks the relay for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HistoryRange {
    days: u32,
}

impl Default for HistoryRange {
    fn default() -> Self {
        Self { days: Self::DEFAULT_DAYS }
    }
}

impl HistoryRange {
    pub const DEFAULT_DAYS: u32 = 7;
    pub const MIN_DAYS: u32 = 1;
    pub const MAX_DAYS: u32 = 365;
    const SECONDS_PER_DAY: i64 = 86_400;
    const BLOCKS_PER_DAY: u32 = 144;
    const MIN_NOTES: u32 = 1_200;

    #[must_use]
    pub fn days(days: u32) -> Self {
        Self {
            days: days.clamp(Self::MIN_DAYS, Self::MAX_DAYS),
        }
    }

    /// Reads what the user typed: a whole number of days, clamped to the allowed span.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        text.trim().parse::<u32>().ok().map(Self::days)
    }

    #[must_use]
    pub const fn day_count(&self) -> u32 {
        self.days
    }

    #[must_use]
    pub const fn seconds(&self) -> i64 {
        self.days as i64 * Self::SECONDS_PER_DAY
    }

    #[must_use]
    pub const fn since(&self, now: i64) -> i64 {
        now - self.seconds()
    }

    /// How many notes one window can carry over the range, with room for slow and fast days.
    #[must_use]
    pub fn note_capacity(&self) -> u32 {
        (self.days * Self::BLOCKS_PER_DAY * 5 / 4).max(Self::MIN_NOTES)
    }

    #[must_use]
    pub fn label(&self) -> String {
        if self.days == 1 {
            "1 day".to_string()
        } else {
            format!("{} days", self.days)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryRange;

    #[test]
    fn the_default_is_one_week() {
        let range = HistoryRange::default();

        assert_eq!(range.day_count(), 7);
        assert_eq!(range.since(1_000_000), 1_000_000 - 7 * 86_400);
        assert_eq!(range.label(), "7 days");
    }

    #[test]
    fn typed_days_are_read_and_clamped() {
        assert_eq!(HistoryRange::parse(" 30 "), Some(HistoryRange::days(30)));
        assert_eq!(HistoryRange::parse("0"), Some(HistoryRange::days(1)));
        assert_eq!(HistoryRange::parse("9999").map(|range| range.day_count()), Some(HistoryRange::MAX_DAYS));
        assert_eq!(HistoryRange::parse("week"), None);
        assert_eq!(HistoryRange::parse("-3"), None);
    }

    #[test]
    fn the_capacity_covers_every_block_of_the_range() {
        assert_eq!(HistoryRange::days(1).note_capacity(), 1_200);
        assert!(HistoryRange::days(30).note_capacity() >= 30 * 144);
        assert_eq!(HistoryRange::days(1).label(), "1 day");
    }
}
