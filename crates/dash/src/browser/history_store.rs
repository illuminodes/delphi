use nostr_minions::NostrNote;

use super::dash_db::DashDb;

/// The audit notes saved in this browser per publisher, so a revisit draws the history at once
/// and asks the relay only for what came after.
pub struct HistoryStore {
    db: DashDb,
    publisher: String,
}

impl HistoryStore {
    #[must_use]
    pub fn new(db: &DashDb, publisher_hex: &str) -> Self {
        Self {
            db: db.clone(),
            publisher: publisher_hex.to_string(),
        }
    }

    pub async fn load(&self) -> Vec<NostrNote> {
        self.db
            .audits(&self.publisher)
            .await
            .iter()
            .filter_map(|text| serde_json::from_str(text).ok())
            .collect()
    }

    pub fn save(&self, added: &[NostrNote], dropped: Vec<String>) {
        let added = added
            .iter()
            .filter_map(|note| Some((note.id.clone()?, serde_json::to_string(note).ok()?)))
            .collect();
        self.db.change_audits_later(self.publisher.clone(), added, dropped);
    }
}
