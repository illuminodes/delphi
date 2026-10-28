use super::dash_db::DashDb;
use crate::WorkspaceArrangement;

/// The reader's card order and sizes, saved in this browser.
pub struct ArrangementStore;

impl ArrangementStore {
    const KEY: &'static str = "arrangement";

    pub async fn load(db: &DashDb) -> Option<WorkspaceArrangement> {
        WorkspaceArrangement::from_json(&db.read(Self::KEY).await?)
    }

    pub fn save(db: &DashDb, arrangement: &WorkspaceArrangement) {
        db.write_later(Self::KEY.to_string(), arrangement.to_json());
    }
}
