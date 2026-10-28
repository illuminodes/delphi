use std::rc::Rc;

use idb::{DatabaseEvent, Factory, KeyRange, ObjectStoreParams, TransactionMode};
use wasm_bindgen::JsValue;

/// The dashboard's IndexedDB database: one store of named JSON values (settings, layout, the
/// repeated notes) and one of audit notes keyed by publisher and note id.
#[derive(Debug, Clone)]
pub struct DashDb {
    db: Rc<idb::Database>,
}

impl PartialEq for DashDb {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.db, &other.db)
    }
}

impl DashDb {
    const NAME: &'static str = "delphi-dash";
    const VERSION: u32 = 1;
    const VALUES: &'static str = "values";
    const AUDITS: &'static str = "audits";

    /// Opens the database, creating its stores the first time.
    ///
    /// # Errors
    /// When the browser has no `IndexedDB` or refuses to open it (private windows, full quota).
    pub async fn open() -> Result<Self, idb::Error> {
        let mut request = Factory::new()?.open(Self::NAME, Some(Self::VERSION))?;
        request.on_upgrade_needed(|event| {
            let Ok(database) = event.database() else { return };
            let existing = database.store_names();
            for name in [Self::VALUES, Self::AUDITS] {
                if !existing.iter().any(|held| held == name) {
                    let _ = database.create_object_store(name, ObjectStoreParams::new());
                }
            }
        });
        Ok(Self { db: Rc::new(request.await?) })
    }

    pub async fn read(&self, key: &str) -> Option<String> {
        let transaction = self.db.transaction(&[Self::VALUES], TransactionMode::ReadOnly).ok()?;
        let store = transaction.object_store(Self::VALUES).ok()?;
        let value = store.get(JsValue::from_str(key)).ok()?.await.ok().flatten()?;
        value.as_string()
    }

    /// # Errors
    /// When the write transaction fails.
    pub async fn write(&self, key: &str, value: &str) -> Result<(), idb::Error> {
        let transaction = self.db.transaction(&[Self::VALUES], TransactionMode::ReadWrite)?;
        transaction
            .object_store(Self::VALUES)?
            .put(&JsValue::from_str(value), Some(&JsValue::from_str(key)))?;
        transaction.commit()?.await?;
        Ok(())
    }

    /// Writes in the background; a failed write only costs the saved copy.
    pub fn write_later(&self, key: String, value: String) {
        let db = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = db.write(&key, &value).await;
        });
    }

    /// Every audit note saved for `publisher`, as JSON.
    pub async fn audits(&self, publisher: &str) -> Vec<String> {
        self.read_audits(publisher).await.unwrap_or_default()
    }

    async fn read_audits(&self, publisher: &str) -> Result<Vec<String>, idb::Error> {
        let range = KeyRange::bound(&Self::audit_key(publisher, ""), &Self::audit_key(publisher, "\u{ffff}"), None, None)?;
        let transaction = self.db.transaction(&[Self::AUDITS], TransactionMode::ReadOnly)?;
        let values = transaction.object_store(Self::AUDITS)?.get_all(Some(range.into()), None)?.await?;
        Ok(values.into_iter().filter_map(|value| value.as_string()).collect())
    }

    /// Saves `added` (id, JSON) audit notes and deletes the `dropped` ids, in the background.
    pub fn change_audits_later(&self, publisher: String, added: Vec<(String, String)>, dropped: Vec<String>) {
        let db = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = db.change_audits(&publisher, &added, &dropped).await;
        });
    }

    async fn change_audits(&self, publisher: &str, added: &[(String, String)], dropped: &[String]) -> Result<(), idb::Error> {
        let transaction = self.db.transaction(&[Self::AUDITS], TransactionMode::ReadWrite)?;
        let store = transaction.object_store(Self::AUDITS)?;
        for (id, json) in added {
            store.put(&JsValue::from_str(json), Some(&Self::audit_key(publisher, id)))?;
        }
        for id in dropped {
            store.delete(Self::audit_key(publisher, id))?;
        }
        transaction.commit()?.await?;
        Ok(())
    }

    fn audit_key(publisher: &str, id: &str) -> JsValue {
        JsValue::from_str(&format!("{publisher}|{id}"))
    }
}
