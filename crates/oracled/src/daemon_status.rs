use std::time::Duration;

use mantle::difficulty::BlockHeight;
use delphi_oracle::PriceBand;

use crate::{JsonObject, NoteClass, NotePayload, TipAge};

/// What the daemon is doing right now, streamed live so a subscriber can
/// tell a quiet market from a stalled oracle.
#[derive(Debug, Clone, PartialEq)]
pub struct DaemonStatus {
    pub tip: Option<BlockHeight>,
    /// How long ago the tip header was stamped.
    pub tip_age: Option<TipAge>,
    pub anchor: Option<f64>,
    /// Blocks between the anchor's block and the tip.
    pub anchor_age: Option<u32>,
    pub band: Option<PriceBand>,
    pub mempool_outputs: usize,
    pub mempool_transactions: u64,
    pub audits_published: u64,
    pub transaction_notes: u64,
    pub transaction_notes_dropped: u64,
    pub uptime: Duration,
}

impl DaemonStatus {
    pub const WINDOW: &'static str = "status";
}

impl NotePayload for DaemonStatus {
    fn window(&self) -> &str {
        Self::WINDOW
    }

    fn label(&self) -> &str {
        Self::WINDOW
    }

    fn price(&self) -> f64 {
        self.anchor.unwrap_or(f64::NAN)
    }

    fn class(&self) -> NoteClass {
        NoteClass::Live
    }

    fn is_publishable(&self) -> bool {
        true
    }

    fn to_json(&self) -> String {
        let mut object = JsonObject::new();
        object.text("window", Self::WINDOW);
        match self.tip {
            Some(tip) => object.count("tip", tip.to_u32() as usize),
            None => object.raw("tip", "null"),
        }
        object.raw("tip_age_secs", &self.tip_age.map_or_else(|| "null".to_string(), |age| age.seconds().to_string()));
        object.raw("tip_stale", self.tip_age.map_or("null", |age| if age.is_stale() { "true" } else { "false" }));
        object.fixed("anchor", self.anchor.unwrap_or(f64::NAN), 2);
        match self.anchor_age {
            Some(age) => object.count("anchor_age_blocks", age as usize),
            None => object.raw("anchor_age_blocks", "null"),
        }
        object.fixed("band_low", self.band.map_or(f64::NAN, PriceBand::low), 2);
        object.fixed("band_high", self.band.map_or(f64::NAN, PriceBand::high), 2);
        object.count("mempool_outputs", self.mempool_outputs);
        object.raw("mempool_transactions", &self.mempool_transactions.to_string());
        object.raw("audits_published", &self.audits_published.to_string());
        object.raw("transaction_notes", &self.transaction_notes.to_string());
        object.raw("transaction_notes_dropped", &self.transaction_notes_dropped.to_string());
        object.raw("uptime", &self.uptime.as_secs().to_string());
        object.finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mantle::difficulty::BlockHeight;
    use delphi_oracle::PriceBand;

    use mantle::difficulty::BlockTime;

    use super::DaemonStatus;
    use crate::{NoteClass, NotePayload, TipAge};

    #[test]
    fn a_status_encodes_every_field_and_is_live() {
        let status = DaemonStatus {
            tip: Some(BlockHeight::new(970_000)),
            tip_age: Some(TipAge::at(BlockTime::new(1_000), 1_600)),
            anchor: Some(80_000.0),
            anchor_age: Some(2),
            band: PriceBand::around(80_000.0, 0.15),
            mempool_outputs: 812,
            mempool_transactions: 400,
            audits_published: 12,
            transaction_notes: 30,
            transaction_notes_dropped: 4,
            uptime: Duration::from_secs(90),
        };
        let json: serde_json::Value = serde_json::from_str(&status.to_json()).expect("json");

        assert_eq!(json["tip"], 970_000);
        assert_eq!(json["tip_age_secs"], 600);
        assert_eq!(json["tip_stale"], false);
        assert_eq!(json["anchor"], 80_000.0);
        assert_eq!(json["anchor_age_blocks"], 2);
        assert_eq!(json["mempool_outputs"], 812);
        assert_eq!(json["audits_published"], 12);
        assert_eq!(json["transaction_notes"], 30);
        assert_eq!(json["transaction_notes_dropped"], 4);
        assert_eq!(json["uptime"], 90);
        assert!(json["band_low"].as_f64().expect("low") < 80_000.0);
        assert_eq!(status.class(), NoteClass::Live);
    }

    #[test]
    fn a_status_before_the_first_block_encodes_nulls() {
        let status = DaemonStatus {
            tip: None,
            tip_age: None,
            anchor: None,
            anchor_age: None,
            band: None,
            mempool_outputs: 0,
            mempool_transactions: 0,
            audits_published: 0,
            transaction_notes: 0,
            transaction_notes_dropped: 0,
            uptime: Duration::ZERO,
        };
        let json: serde_json::Value = serde_json::from_str(&status.to_json()).expect("json");

        assert!(json["tip"].is_null());
        assert!(json["tip_age_secs"].is_null());
        assert!(json["tip_stale"].is_null());
        assert!(json["anchor"].is_null());
        assert!(json["anchor_age_blocks"].is_null());
        assert!(status.is_publishable());
    }

    #[test]
    fn a_stale_tip_is_flagged() {
        let status = DaemonStatus {
            tip: Some(BlockHeight::new(970_000)),
            tip_age: Some(TipAge::at(BlockTime::new(1_000), 1_000 + 3 * 60 * 60)),
            anchor: None,
            anchor_age: None,
            band: None,
            mempool_outputs: 0,
            mempool_transactions: 0,
            audits_published: 0,
            transaction_notes: 0,
            transaction_notes_dropped: 0,
            uptime: Duration::ZERO,
        };
        let json: serde_json::Value = serde_json::from_str(&status.to_json()).expect("json");

        assert_eq!(json["tip_age_secs"], 3 * 60 * 60);
        assert_eq!(json["tip_stale"], true);
    }
}
