use nostr_minions::NostrSubscription;

use crate::{ChainBlocks, FitNote, HistoryRange, NoteKind, OracleStatus};

/// The relay filters the dashboard opens: the stored audit history over a
/// range, the live stream, the stored copies of its repeated notes, and the
/// per-transaction stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedFilter {
    author: String,
}

impl FeedFilter {
    pub const WINDOWS: [&'static str; 2] = ["chain-1h", "chain-1d"];

    #[must_use]
    pub fn new(author: impl Into<String>) -> Self {
        Self { author: author.into() }
    }

    /// One filter per window, so a relay's per-filter limit cannot let the
    /// busier window crowd out the other.
    #[must_use]
    pub fn audits(&self, now: i64, range: HistoryRange) -> Vec<NostrSubscription> {
        let since = u64::try_from(range.since(now)).unwrap_or(0);
        Self::WINDOWS
            .into_iter()
            .map(|window| {
                let mut filter = NostrSubscription::new()
                    .kind(NoteKind::AUDIT)
                    .author(self.author.clone());
                filter.add_tag("w", window);
                filter.since = Some(since);
                filter.limit = Some(range.note_capacity());
                filter
            })
            .collect()
    }

    #[must_use]
    pub fn live(&self) -> NostrSubscription {
        NostrSubscription::new()
            .kind(NoteKind::LIVE)
            .author(self.author.clone())
    }

    /// The newest stored copy of the status, blocks and each chain fit, named so that addressable
    /// notes from older daemons are left out. The mempool fit is not stored; it streams every few
    /// seconds.
    #[must_use]
    pub fn stored(&self) -> NostrSubscription {
        let mut filter = NostrSubscription::new()
            .kind(NoteKind::STORED)
            .author(self.author.clone());
        let fits = Self::WINDOWS.map(|window| format!("{}{window}", FitNote::PREFIX));
        for window in [OracleStatus::WINDOW, ChainBlocks::WINDOW].into_iter().chain(fits.iter().map(String::as_str)) {
            filter.add_tag("d", &format!("{}{window}", NoteKind::STORED_PREFIX));
        }
        filter
    }

    #[must_use]
    pub fn transactions(&self) -> NostrSubscription {
        NostrSubscription::new()
            .kind(NoteKind::TRANSACTION)
            .author(self.author.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::FeedFilter;
    use crate::{HistoryRange, NoteKind};

    #[test]
    fn each_window_gets_its_own_audit_filter_reaching_back_over_the_range() {
        let range = HistoryRange::days(30);
        let filters = FeedFilter::new("ab").audits(10_000_000, range);

        assert_eq!(filters.len(), 2);
        for (filter, window) in filters.iter().zip(FeedFilter::WINDOWS) {
            assert_eq!(filter.since, Some(10_000_000 - 30 * 86_400));
            assert_eq!(filter.limit, Some(range.note_capacity()));
            let tags = filter.tags.as_ref().expect("tags");
            assert!(tags["w"].contains(window));
            assert!(filter.kinds.as_ref().is_some_and(|kinds| kinds.contains(&NoteKind::AUDIT)));
        }
    }

    #[test]
    fn the_live_filter_asks_for_the_ephemeral_kind() {
        let filter = FeedFilter::new("ab").live();

        assert!(filter.kinds.as_ref().is_some_and(|kinds| kinds.contains(&NoteKind::LIVE)));
        assert!(filter.since.is_none());
    }

    #[test]
    fn the_stored_filter_names_each_repeated_window() {
        let filter = FeedFilter::new("ab").stored();
        let names = &filter.tags.as_ref().expect("tags")["d"];

        assert!(filter.kinds.as_ref().is_some_and(|kinds| kinds.contains(&NoteKind::STORED)));
        assert!(names.contains("latest-status"));
        assert!(names.contains("latest-blocks"));
        assert!(names.contains("latest-fit-chain-1d"));
        assert!(!names.contains("chain-1h"));
    }

    #[test]
    fn the_transaction_filter_asks_only_for_the_transaction_kind() {
        let filter = FeedFilter::new("ab").transactions();

        assert_eq!(filter.kinds.as_ref().map(|kinds| kinds.iter().copied().collect::<Vec<u32>>()), Some(vec![NoteKind::TRANSACTION]));
    }
}
