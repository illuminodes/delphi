use std::rc::Rc;

use nostr_minions::NostrNote;

use super::arrangement_store::ArrangementStore;
use super::dash_db::DashDb;
use super::history_store::HistoryStore;
use super::kept_store::KeptStore;
use super::settings_store::SettingsStore;
use crate::{DashSettings, WorkspaceArrangement};

/// The notes an earlier visit saved, unchecked; the board passes them through the note gate.
#[derive(Debug, Default, PartialEq)]
pub struct SavedNotes {
    pub kept: Vec<NostrNote>,
    pub history: Vec<NostrNote>,
}

/// Everything read from the browser's database before the first draw, so the page opens as it
/// was left. Without a database the page still runs, with nothing saved.
#[derive(Debug, Clone, PartialEq)]
pub struct Boot {
    pub db: Option<DashDb>,
    pub settings: DashSettings,
    pub arrangement: Option<WorkspaceArrangement>,
    pub saved: Rc<SavedNotes>,
}

impl Boot {
    pub async fn load() -> Self {
        let Ok(db) = DashDb::open().await else {
            return Self {
                db: None,
                settings: DashSettings::default(),
                arrangement: None,
                saved: Rc::default(),
            };
        };
        let settings = SettingsStore::load(&db).await;
        let arrangement = ArrangementStore::load(&db).await;
        let saved = match settings.publisher_key() {
            Ok(key) => SavedNotes {
                kept: KeptStore::new(&db, key.hex()).load().await,
                history: HistoryStore::new(&db, key.hex()).load().await,
            },
            Err(_) => SavedNotes::default(),
        };
        Self {
            db: Some(db),
            settings,
            arrangement,
            saved: Rc::new(saved),
        }
    }
}
