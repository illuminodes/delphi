use nostr_minions::NostrNote;
use serde::Deserialize;

use crate::NoteKind;

/// What the daemon last said about itself on the live stream.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OracleStatus {
    pub tip: Option<u32>,
    #[serde(default)]
    pub tip_age_secs: Option<u64>,
    #[serde(default)]
    pub tip_stale: Option<bool>,
    #[serde(default)]
    pub settled: Option<u32>,
    pub anchor: Option<f64>,
    pub band_low: Option<f64>,
    pub band_high: Option<f64>,
    #[serde(default)]
    pub mempool_outputs: usize,
    #[serde(default)]
    pub mempool_transactions: u64,
    #[serde(default)]
    pub audits_published: u64,
    #[serde(default)]
    pub transaction_notes: u64,
    #[serde(default)]
    pub transaction_notes_dropped: u64,
    #[serde(default)]
    pub uptime: u64,
    #[serde(skip)]
    pub received_at: i64,
}

impl OracleStatus {
    pub const WINDOW: &'static str = "status";

    #[must_use]
    pub fn from_note(note: &NostrNote) -> Option<Self> {
        if !NoteKind::carries_live(note.kind) || NoteKind::window(note) != Some(Self::WINDOW) {
            return None;
        }
        let mut status: Self = serde_json::from_str(&note.content).ok()?;
        status.received_at = note.created_at;
        Some(status)
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::NostrNoteBuilder;

    use super::OracleStatus;
    use crate::NoteKind;

    #[test]
    fn a_status_note_is_read_with_its_time() {
        let mut note = NostrNoteBuilder::new()
            .kind(NoteKind::LIVE)
            .content(r#"{"window":"status","tip":970000,"anchor":80000.0,"band_low":null,"band_high":null,"mempool_outputs":12,"mempool_transactions":6,"audits_published":3,"uptime":90}"#)
            .tag("w", "status")
            .build();
        note.created_at = 55;

        let status = OracleStatus::from_note(&note).expect("status");

        assert_eq!(status.tip, Some(970_000));
        assert_eq!(status.tip_age_secs, None);
        assert_eq!(status.band_low, None);
        assert_eq!(status.mempool_outputs, 12);
        assert_eq!(status.received_at, 55);
    }

    #[test]
    fn the_tip_age_and_staleness_are_read_when_sent() {
        let note = NostrNoteBuilder::new()
            .kind(NoteKind::LIVE)
            .content(r#"{"window":"status","tip":970000,"tip_age_secs":9000,"tip_stale":true,"anchor":null,"band_low":null,"band_high":null}"#)
            .tag("w", "status")
            .build();

        let status = OracleStatus::from_note(&note).expect("status");

        assert_eq!(status.tip_age_secs, Some(9_000));
        assert_eq!(status.tip_stale, Some(true));
    }

    #[test]
    fn other_live_notes_are_not_status() {
        let note = NostrNoteBuilder::new()
            .kind(NoteKind::LIVE)
            .content("{}")
            .tag("w", "mempool")
            .build();

        assert!(OracleStatus::from_note(&note).is_none());
    }
}
