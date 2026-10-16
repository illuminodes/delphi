use nostr_minions::NostrNote;

/// The note kinds the oracle publishes, and the window tag each carries.
pub struct NoteKind;

impl NoteKind {
    pub const AUDIT: u32 = 8_078;
    pub const LIVE: u32 = 20_078;
    pub const TRANSACTION: u32 = 20_079;

    #[must_use]
    pub fn window(note: &NostrNote) -> Option<&str> {
        note.tags
            .find_tags_ref("w")
            .first()
            .copied()
            .filter(|window| !window.is_empty())
    }
}
