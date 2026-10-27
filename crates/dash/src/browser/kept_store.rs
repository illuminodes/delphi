use nostr_minions::NostrNote;
use web_sys::Storage;

use crate::KeptNotes;

/// The status, blocks and fit notes saved in this browser per publisher, shown at once on the next
/// visit until the relay sends fresh ones.
pub struct KeptStore {
    key: String,
}

impl KeptStore {
    const PREFIX: &'static str = "delphi-dash.kept.";

    #[must_use]
    pub fn new(publisher_hex: &str) -> Self {
        Self {
            key: format!("{}{publisher_hex}", Self::PREFIX),
        }
    }

    fn storage() -> Option<Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    /// The saved notes, unchecked; the caller passes them through the note gate.
    #[must_use]
    pub fn load(&self) -> Vec<NostrNote> {
        Self::storage()
            .and_then(|storage| storage.get_item(&self.key).ok().flatten())
            .map(|text| KeptNotes::read(&text))
            .unwrap_or_default()
    }

    pub fn save(&self, kept: &KeptNotes) {
        if let Some(storage) = Self::storage() {
            let _ = storage.set_item(&self.key, &kept.to_json());
        }
    }
}
