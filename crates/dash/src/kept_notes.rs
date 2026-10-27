use std::collections::BTreeMap;

use nostr_minions::NostrNote;

use crate::NoteKind;

/// The newest note per window the daemon only repeats now and then (status, blocks, fits), whether it
/// came live or as its stored copy. The browser saves these so a reload can show them at once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeptNotes {
    notes: BTreeMap<String, NostrNote>,
    revision: u64,
}

impl KeptNotes {
    /// Keeps `note` when it is newer than the one held for its window, and says whether it was.
    pub fn keep(&mut self, note: &NostrNote) -> bool {
        let Some(window) = NoteKind::window(note) else {
            return false;
        };
        let newer = self
            .notes
            .get(window)
            .is_none_or(|held| note.created_at > held.created_at);
        if newer {
            self.notes.insert(window.to_string(), note.clone());
            self.revision += 1;
        }
        newer
    }

    /// Counts every change, so a saver can tell whether anything new arrived.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        let notes: Vec<&NostrNote> = self.notes.values().collect();
        serde_json::to_string(&notes).unwrap_or_default()
    }

    /// The notes in saved `text`; unreadable text yields none. Signatures are not checked here.
    #[must_use]
    pub fn read(text: &str) -> Vec<NostrNote> {
        serde_json::from_str(text).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::KeptNotes;
    use crate::NoteKind;

    struct KeptFixture;

    impl KeptFixture {
        fn note(window: &str, at: i64) -> NostrNote {
            let mut note = NostrNoteBuilder::new().kind(NoteKind::LIVE).content("{}").tag("w", window).build();
            note.created_at = at;
            note
        }
    }

    #[test]
    fn only_a_newer_note_replaces_the_one_held_for_its_window() {
        let mut kept = KeptNotes::default();

        assert!(kept.keep(&KeptFixture::note("status", 10)));
        assert!(!kept.keep(&KeptFixture::note("status", 10)));
        assert!(!kept.keep(&KeptFixture::note("status", 5)));
        assert!(kept.keep(&KeptFixture::note("blocks", 1)));
        assert_eq!(kept.revision(), 2);
    }

    #[test]
    fn saved_notes_read_back_unchanged() {
        let mut kept = KeptFixture::note("status", 10);
        kept.id = Some("ab".to_string());
        kept.sig = Some("cd".to_string());
        let mut notes = KeptNotes::default();
        notes.keep(&kept);

        assert_eq!(KeptNotes::read(&notes.to_json()), vec![kept]);
        assert!(KeptNotes::read("not json").is_empty());
    }
}
