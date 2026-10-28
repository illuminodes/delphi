use web_sys::Storage;

use crate::WorkspaceArrangement;

/// The reader's card order and sizes, saved in this browser.
pub struct ArrangementStore;

impl ArrangementStore {
    const KEY: &'static str = "delphi-dash.arrangement";

    fn storage() -> Option<Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    #[must_use]
    pub fn load() -> Option<WorkspaceArrangement> {
        Self::storage()
            .and_then(|storage| storage.get_item(Self::KEY).ok().flatten())
            .and_then(|text| WorkspaceArrangement::from_json(&text))
    }

    pub fn save(arrangement: &WorkspaceArrangement) {
        if let Some(storage) = Self::storage() {
            let _ = storage.set_item(Self::KEY, &arrangement.to_json());
        }
    }
}
