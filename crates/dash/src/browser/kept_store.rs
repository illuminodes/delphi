use nostr_minions::NostrNote;

use super::dash_db::DashDb;
use crate::KeptNotes;

/// The status, blocks and fit notes saved in this browser per publisher, shown at once on the next
/// visit until the relay sends fresh ones.
pub struct KeptStore {
    db: DashDb,
    key: String,
}

impl KeptStore {
    const PREFIX: &'static str = "kept.";

    #[must_use]
    pub fn new(db: &DashDb, publisher_hex: &str) -> Self {
        Self {
            db: db.clone(),
            key: format!("{}{publisher_hex}", Self::PREFIX),
        }
    }

    /// The saved notes, unchecked; the caller passes them through the note gate.
    pub async fn load(&self) -> Vec<NostrNote> {
        self.db.read(&self.key).await.map(|text| KeptNotes::read(&text)).unwrap_or_default()
    }

    pub fn save(&self, kept: &KeptNotes) {
        self.db.write_later(self.key.clone(), kept.to_json());
    }
}
