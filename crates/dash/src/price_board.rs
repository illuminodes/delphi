use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{PriceNote, SeriesPoint, WindowLabel};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingest {
    Newer,
    Older,
    Replaced,
    Rejected,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct WindowState {
    latest: Option<PriceNote>,
    history: BTreeMap<i64, PriceNote>,
}

/// Every price heard per window: the newest, and a history keyed by block
/// height for chain windows and by arrival for the mempool. A reorg
/// replacement removes the note it names, whichever arrives first.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceBoard {
    windows: HashMap<String, WindowState>,
    replaced: HashSet<String>,
    history_limit: usize,
    rejected: usize,
}

impl Default for PriceBoard {
    fn default() -> Self {
        Self::new(Self::DEFAULT_HISTORY)
    }
}

impl PriceBoard {
    pub const DEFAULT_HISTORY: usize = 1_200;

    #[must_use]
    pub fn new(history_limit: usize) -> Self {
        Self {
            windows: HashMap::new(),
            replaced: HashSet::new(),
            history_limit: history_limit.max(2),
            rejected: 0,
        }
    }

    pub fn accept(&mut self, note: &nostr_minions::NostrNote) -> Ingest {
        if let Ok(price) = PriceNote::from_note(note) {
            self.record(price)
        } else {
            self.rejected += 1;
            Ingest::Rejected
        }
    }

    pub fn record(&mut self, price: PriceNote) -> Ingest {
        if price.id().is_some_and(|id| self.replaced.contains(id)) {
            return Ingest::Replaced;
        }
        if let Some(old) = price.replaces() {
            self.replaced.insert(old.to_string());
            for state in self.windows.values_mut() {
                state.history.retain(|_, held| held.id() != Some(old));
                if state.latest.as_ref().is_some_and(|latest| latest.id() == Some(old)) {
                    state.latest = None;
                }
            }
        }
        let limit = self.history_limit;
        let state = self.windows.entry(price.window().to_string()).or_default();
        state.history.insert(price.history_key(), price.clone());
        while state.history.len() > limit {
            state.history.pop_first();
        }
        let newer = state
            .latest
            .as_ref()
            .is_none_or(|latest| price.history_key() >= latest.history_key());
        if newer {
            state.latest = Some(price);
            Ingest::Newer
        } else {
            Ingest::Older
        }
    }

    #[must_use]
    pub const fn rejected(&self) -> usize {
        self.rejected
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.windows.values().all(|state| state.latest.is_none())
    }

    #[must_use]
    pub fn latest(&self, window: &str) -> Option<&PriceNote> {
        self.windows.get(window)?.latest.as_ref()
    }

    /// The window's history, oldest first.
    #[must_use]
    pub fn history(&self, window: &str) -> Vec<SeriesPoint> {
        self.windows
            .get(window)
            .map(|state| state.history.values().map(PriceNote::point).collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn ordered(&self) -> Vec<&PriceNote> {
        let mut notes: Vec<&PriceNote> = self
            .windows
            .values()
            .filter_map(|state| state.latest.as_ref())
            .filter(|note| !WindowLabel::is_retired(note.window()))
            .collect();
        notes.sort_by(|left, right| {
            WindowLabel::of(left.window())
                .rank()
                .cmp(&WindowLabel::of(right.window()).rank())
                .then_with(|| left.window().cmp(right.window()))
        });
        notes
    }

    #[must_use]
    pub fn spread_percent(&self, window: &str, reference: &str) -> Option<f64> {
        let price = self.latest(window)?.price();
        let base = self.latest(reference)?.price();
        (base > 0.0).then(|| (price - base) / base * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::{Ingest, PriceBoard};
    use crate::{NoteKind, SeriesPoint};

    struct BoardFixture;

    impl BoardFixture {
        fn mempool(price: f64, created_at: i64) -> NostrNote {
            let content = format!(
                r#"{{"source":"mempool","price":{price},"samples":300,"transactions":100,"band_low":{},"band_high":{}}}"#,
                price - 50.0,
                price + 50.0
            );
            let mut note = NostrNoteBuilder::new()
                .kind(NoteKind::LIVE)
                .content(content)
                .tag("w", "mempool")
                .build();
            note.created_at = created_at;
            note
        }

        fn audit(window: &str, price: f64, height: u32) -> NostrNote {
            Self::audit_with_id(window, price, height, &format!("{window}-{height}"))
        }

        fn audit_with_id(window: &str, price: f64, height: u32, id: &str) -> NostrNote {
            let content = format!(
                r#"{{"price":{price},"samples":900,"deviation":0.1,"start_height":1,"stop_height":{height},"start_time":1,"stop_time":{}}}"#,
                1_000 + height
            );
            let mut note = NostrNoteBuilder::new()
                .kind(NoteKind::AUDIT)
                .content(content)
                .tag("w", window)
                .build();
            note.created_at = i64::from(1_000 + height);
            note.id = Some(id.to_string());
            note
        }
    }

    #[test]
    fn the_highest_block_of_a_window_is_the_latest() {
        let mut board = PriceBoard::default();

        assert_eq!(board.accept(&BoardFixture::audit("chain-1h", 100.0, 10)), Ingest::Newer);
        assert_eq!(board.accept(&BoardFixture::audit("chain-1h", 110.0, 20)), Ingest::Newer);
        assert_eq!(board.accept(&BoardFixture::audit("chain-1h", 90.0, 15)), Ingest::Older);

        assert!((board.latest("chain-1h").expect("latest").price() - 110.0).abs() < 1e-9);
        let heights: Vec<Option<u32>> =
            board.history("chain-1h").iter().map(SeriesPoint::stop_height).collect();
        assert_eq!(heights, vec![Some(10), Some(15), Some(20)]);
    }

    #[test]
    fn a_note_delivered_twice_is_recorded_once() {
        let mut board = PriceBoard::default();
        let note = BoardFixture::mempool(100.0, 10);

        board.accept(&note);
        board.accept(&note);

        assert_eq!(board.history("mempool").len(), 1);
    }

    #[test]
    fn history_keeps_only_the_newest_points() {
        let mut board = PriceBoard::new(3);
        for second in 1..=6 {
            board.accept(&BoardFixture::mempool(100.0 + f64::from(second), i64::from(second)));
        }

        let times: Vec<i64> = board.history("mempool").iter().map(SeriesPoint::stop_time).collect();
        assert_eq!(times, vec![4, 5, 6]);
    }

    #[test]
    fn a_reorg_replacement_wins_in_either_order() {
        for replacement_first in [false, true] {
            let mut board = PriceBoard::default();
            let orphan = BoardFixture::audit_with_id("chain-1h", 100.0, 10, "orphan");
            let mut replacement = BoardFixture::audit_with_id("chain-1h", 105.0, 10, "fresh");
            replacement.tags.add_custom_tag("e", "orphan");
            if replacement_first {
                board.accept(&replacement);
                assert_eq!(board.accept(&orphan), Ingest::Replaced);
            } else {
                board.accept(&orphan);
                board.accept(&replacement);
            }

            let history = board.history("chain-1h");
            assert_eq!(history.len(), 1);
            assert!((history[0].price() - 105.0).abs() < 1e-9);
            assert!((board.latest("chain-1h").expect("latest").price() - 105.0).abs() < 1e-9);
        }
    }

    #[test]
    fn unreadable_notes_are_counted_and_change_nothing() {
        let mut board = PriceBoard::default();
        let mut bad = BoardFixture::audit("chain-1h", 100.0, 10);
        bad.content = "not json".to_string();

        assert_eq!(board.accept(&bad), Ingest::Rejected);

        assert!(board.is_empty());
        assert_eq!(board.rejected(), 1);
    }

    #[test]
    fn notes_are_ordered_live_first_then_by_window_length() {
        let mut board = PriceBoard::default();
        for window in ["chain-4h", "chain-1h", "chain-1d", "zzz"] {
            board.accept(&BoardFixture::audit(window, 100.0, 10));
        }
        board.accept(&BoardFixture::mempool(100.0, 10));

        let order: Vec<&str> = board.ordered().iter().map(|note| note.window()).collect();
        assert_eq!(order, vec!["mempool", "chain-1h", "chain-1d", "chain-4h", "zzz"]);
    }

    #[test]
    fn a_retired_window_is_not_listed() {
        let mut board = PriceBoard::default();
        board.accept(&BoardFixture::audit("chain-1w", 100.0, 10));
        board.accept(&BoardFixture::audit("chain-1h", 100.0, 10));

        let order: Vec<&str> = board.ordered().iter().map(|note| note.window()).collect();
        assert_eq!(order, vec!["chain-1h"]);
    }

    #[test]
    fn the_spread_is_measured_against_a_reference_window() {
        let mut board = PriceBoard::default();
        board.accept(&BoardFixture::audit("chain-1h", 100.0, 10));
        board.accept(&BoardFixture::mempool(101.0, 10));

        let spread = board.spread_percent("mempool", "chain-1h").expect("spread");
        assert!((spread - 1.0).abs() < 1e-9);
        assert_eq!(board.spread_percent("mempool", "chain-1d"), None);
    }
}
