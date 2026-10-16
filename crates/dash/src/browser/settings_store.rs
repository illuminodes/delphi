use web_sys::Storage;

use crate::DashSettings;

pub struct SettingsStore;

impl SettingsStore {
    const KEY: &'static str = "delphi-dash.settings";

    fn storage() -> Option<Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    #[must_use]
    pub fn load() -> DashSettings {
        Self::storage()
            .and_then(|storage| storage.get_item(Self::KEY).ok().flatten())
            .map(|text| DashSettings::from_json(&text))
            .unwrap_or_default()
    }

    pub fn save(settings: &DashSettings) {
        if let Some(storage) = Self::storage() {
            let _ = storage.set_item(Self::KEY, &settings.to_json());
        }
    }
}
