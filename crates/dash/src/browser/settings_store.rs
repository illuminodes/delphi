use super::dash_db::DashDb;
use crate::DashSettings;

pub struct SettingsStore;

impl SettingsStore {
    const KEY: &'static str = "settings";

    pub async fn load(db: &DashDb) -> DashSettings {
        db.read(Self::KEY).await.map(|text| DashSettings::from_json(&text)).unwrap_or_default()
    }

    /// # Errors
    /// When the browser could not write the settings.
    pub async fn save(db: &DashDb, settings: &DashSettings) -> Result<(), idb::Error> {
        db.write(Self::KEY, &settings.to_json()).await
    }
}
