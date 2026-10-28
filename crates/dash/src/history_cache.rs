use std::collections::{BTreeMap, HashMap};

use nostr_minions::NostrNote;

use crate::NoteKind;

/// The audit notes kept in this browser, the newest [`Self::PER_WINDOW`] per window, so a revisit
/// draws the history at once and asks the relay only for what came after.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryCache {
    windows: HashMap<String, BTreeMap<(i64, String), NostrNote>>,
}

impl HistoryCache {
    pub const PER_WINDOW: usize = 2_500;
    /// How far before the newest kept note the next query reaches back, so a block that settled
    /// late or replaced another is still heard.
    pub const OVERLAP_SECONDS: i64 = 6 * 3_600;

    /// Keeps an audit note. Returns `None` when it was not kept (not an audit, already held, or
    /// older than everything in bounds), else the ids pushed out to stay within bounds.
    pub fn keep(&mut self, note: &NostrNote) -> Option<Vec<String>> {
        if note.kind != NoteKind::AUDIT {
            return None;
        }
        let (Some(window), Some(id)) = (NoteKind::window(note), note.id.clone()) else {
            return None;
        };
        let held = self.windows.entry(window.to_string()).or_default();
        let key = (note.created_at, id.clone());
        if held.contains_key(&key) {
            return None;
        }
        held.insert(key, note.clone());
        let mut dropped = Vec::new();
        while held.len() > Self::PER_WINDOW {
            if let Some(((_, old), _)) = held.pop_first() {
                dropped.push(old);
            }
        }
        (!dropped.contains(&id)).then_some(dropped)
    }

    /// Where the query for `window` should start: just before the newest kept note when the kept
    /// notes already reach back to `range_since`, else `range_since` itself.
    #[must_use]
    pub fn since(&self, window: &str, range_since: i64) -> i64 {
        let Some(held) = self.windows.get(window) else {
            return range_since;
        };
        match (held.first_key_value(), held.last_key_value()) {
            (Some(((oldest, _), _)), Some(((newest, _), _))) if *oldest <= range_since + Self::OVERLAP_SECONDS => {
                range_since.max(newest - Self::OVERLAP_SECONDS)
            }
            _ => range_since,
        }
    }

    pub fn notes(&self) -> impl Iterator<Item = &NostrNote> {
        self.windows.values().flat_map(BTreeMap::values)
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::HistoryCache;
    use crate::NoteKind;

    struct CacheFixture;

    impl CacheFixture {
        fn audit(window: &str, at: i64) -> NostrNote {
            let mut note = NostrNoteBuilder::new().kind(NoteKind::AUDIT).content("{}").tag("w", window).build();
            note.created_at = at;
            note.id = Some(format!("{window}-{at}"));
            note
        }
    }

    #[test]
    fn only_new_audit_notes_are_kept() {
        let mut cache = HistoryCache::default();
        let mut live = CacheFixture::audit("chain-1h", 5);
        live.kind = NoteKind::LIVE;

        assert_eq!(cache.keep(&CacheFixture::audit("chain-1h", 5)), Some(Vec::new()));
        assert_eq!(cache.keep(&CacheFixture::audit("chain-1h", 5)), None);
        assert_eq!(cache.keep(&live), None);
        assert_eq!(cache.notes().count(), 1);
    }

    #[test]
    fn the_oldest_note_is_dropped_past_the_bound_and_an_older_one_is_refused() {
        let mut cache = HistoryCache::default();
        for at in 1..=HistoryCache::PER_WINDOW as i64 {
            cache.keep(&CacheFixture::audit("chain-1d", at + 10));
        }

        assert_eq!(cache.keep(&CacheFixture::audit("chain-1d", 1)), None);
        assert_eq!(cache.keep(&CacheFixture::audit("chain-1d", 100_000)), Some(vec!["chain-1d-11".to_string()]));
        assert_eq!(cache.notes().count(), HistoryCache::PER_WINDOW);
    }

    #[test]
    fn the_query_starts_near_the_newest_note_only_when_the_cache_covers_the_range() {
        let mut cache = HistoryCache::default();
        cache.keep(&CacheFixture::audit("chain-1h", 1_000));
        cache.keep(&CacheFixture::audit("chain-1h", 100_000));

        assert_eq!(cache.since("chain-1h", 2_000), 100_000 - HistoryCache::OVERLAP_SECONDS);
        assert_eq!(cache.since("chain-1h", 99_000), 99_000);
        assert_eq!(cache.since("chain-1h", -50_000), -50_000);
        assert_eq!(cache.since("chain-1d", 2_000), 2_000);
    }
}
