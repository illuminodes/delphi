use nostr_minions::NostrSubscription;

use crate::{HistoryRange, NoteKind};

/// The relay filters the dashboard opens: the stored audit history over a
/// range, the live stream, and the per-transaction stream.
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
    fn the_transaction_filter_asks_only_for_the_transaction_kind() {
        let filter = FeedFilter::new("ab").transactions();

        assert_eq!(filter.kinds.as_ref().map(|kinds| kinds.iter().copied().collect::<Vec<u32>>()), Some(vec![NoteKind::TRANSACTION]));
    }
}
