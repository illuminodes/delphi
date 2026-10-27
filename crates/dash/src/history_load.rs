use std::collections::HashSet;

use nostr_minions::NostrNote;

/// Holds the stored audit history back while the relay is still sending it, so the charts are
/// drawn once with all of it instead of once per note. Loading ends when every history
/// subscription has reported the end of its stored notes, or when the caller gives up waiting.
#[derive(Debug, Default)]
pub struct HistoryLoad {
    waiting: HashSet<String>,
    held: Vec<NostrNote>,
    loading: bool,
}

impl HistoryLoad {
    /// Starts holding notes. Call before the subscriptions open, then [`Self::wait_for`] their ids.
    pub fn start(&mut self) {
        self.waiting.clear();
        self.held.clear();
        self.loading = true;
    }

    pub fn wait_for(&mut self, ids: impl IntoIterator<Item = String>) {
        if self.loading {
            self.waiting.extend(ids);
        }
    }

    /// The note back when loading is over, or `None` once it is held.
    pub fn hold(&mut self, note: NostrNote) -> Option<NostrNote> {
        if self.loading {
            self.held.push(note);
            return None;
        }
        Some(note)
    }

    /// Marks subscription `id` as done; returns every held note once the last one is.
    pub fn finish(&mut self, id: &str) -> Vec<NostrNote> {
        if !self.waiting.remove(id) || !self.waiting.is_empty() {
            return Vec::new();
        }
        self.release()
    }

    /// Stops waiting and returns every held note.
    pub fn release(&mut self) -> Vec<NostrNote> {
        self.loading = false;
        self.waiting.clear();
        std::mem::take(&mut self.held)
    }

    #[must_use]
    pub const fn is_loading(&self) -> bool {
        self.loading
    }

    #[must_use]
    pub fn held(&self) -> usize {
        self.held.len()
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::NostrNote;

    use super::HistoryLoad;

    #[test]
    fn notes_are_held_until_every_subscription_has_finished() {
        let mut load = HistoryLoad::default();
        load.start();
        load.wait_for(["a".to_string(), "b".to_string()]);

        assert!(load.hold(NostrNote::default()).is_none());
        assert!(load.hold(NostrNote::default()).is_none());
        assert!(load.finish("a").is_empty());
        assert!(load.finish("unknown").is_empty());
        assert_eq!(load.held(), 2);
        assert_eq!(load.finish("b").len(), 2);
        assert!(!load.is_loading());
        assert!(load.hold(NostrNote::default()).is_some());
    }

    #[test]
    fn releasing_early_hands_back_what_was_held() {
        let mut load = HistoryLoad::default();
        load.start();
        load.wait_for(["a".to_string()]);
        load.hold(NostrNote::default());

        assert_eq!(load.release().len(), 1);
        assert!(load.finish("a").is_empty());
        assert!(load.hold(NostrNote::default()).is_some());
    }
}
