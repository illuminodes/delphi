use nostr_minions::NostrNote;

/// The note kinds the oracle publishes, and the window tag each carries.
pub struct NoteKind;

impl NoteKind {
    pub const AUDIT: u32 = 8_078;
    pub const LIVE: u32 = 20_078;
    pub const TRANSACTION: u32 = 20_079;
    /// The relay keeps the newest of these per `d` tag: a stored copy of a live note, so a page
    /// opened between repeats still finds it.
    pub const STORED: u32 = 30_078;
    /// The `d` tag of a stored copy is this prefix and the live window it copies.
    pub const STORED_PREFIX: &'static str = "latest-";

    #[must_use]
    pub fn window(note: &NostrNote) -> Option<&str> {
        note.tags
            .find_tags_ref("w")
            .first()
            .copied()
            .filter(|window| !window.is_empty())
    }

    /// Whether a note of `kind` carries a live payload: the live note itself or its stored copy.
    #[must_use]
    pub const fn carries_live(kind: u32) -> bool {
        kind == Self::LIVE || kind == Self::STORED
    }
}
