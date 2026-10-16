use mantle::hash::Hash256;
use delphi_oracle::OracleReport;

use crate::{AlgorithmParams, ChainWindow, JsonObject, NoteClass, NotePayload};

/// The oracle's final price for one window ending at one block.
///
/// The content and timestamp depend only on the blocks and the algorithm, so
/// publishing the same record twice yields the same note id.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRecord {
    window: ChainWindow,
    report: OracleReport,
    stop_hash: Hash256,
    params: AlgorithmParams,
    replaces: Option<String>,
}

impl AuditRecord {
    pub const LABEL: &'static str = "audit";

    #[must_use]
    pub const fn new(window: ChainWindow, report: OracleReport, stop_hash: Hash256) -> Self {
        Self {
            window,
            report,
            stop_hash,
            params: AlgorithmParams::current(),
            replaces: None,
        }
    }

    /// Mark this record as the replacement of the note `id`, after a reorg.
    #[must_use]
    pub fn replacing(mut self, id: impl Into<String>) -> Self {
        self.replaces = Some(id.into());
        self
    }

    #[must_use]
    pub const fn chain_window(&self) -> ChainWindow {
        self.window
    }

    #[must_use]
    pub const fn report(&self) -> &OracleReport {
        &self.report
    }

    #[must_use]
    pub const fn stop_hash(&self) -> Hash256 {
        self.stop_hash
    }

    #[must_use]
    pub fn replaces(&self) -> Option<&str> {
        self.replaces.as_deref()
    }
}

impl NotePayload for AuditRecord {
    fn window(&self) -> &str {
        self.window.tag()
    }

    fn label(&self) -> &str {
        Self::LABEL
    }

    fn price(&self) -> f64 {
        self.report.central_price
    }

    fn class(&self) -> NoteClass {
        NoteClass::Audit
    }

    fn created_at(&self) -> Option<i64> {
        Some(i64::from(self.report.stop_time.to_u32()))
    }

    fn extra_tags(&self) -> Vec<(&'static str, String)> {
        let mut tags = vec![("h", self.report.stop_height.to_u32().to_string())];
        if let Some(id) = &self.replaces {
            tags.push(("e", id.clone()));
        }
        tags
    }

    fn is_publishable(&self) -> bool {
        self.report.central_price.is_finite()
            && self.report.central_price > 0.0
            && self.report.deviation.is_finite()
            && !self.stop_hash.eq(&Hash256::ZERO)
    }

    fn to_json(&self) -> String {
        let report = &self.report;
        let mut object = JsonObject::new();
        object.text("window", self.window.tag());
        object.count("blocks", self.window.blocks());
        object.fixed("price", report.central_price, 2);
        object.fixed("deviation", report.deviation, 6);
        object.fixed("mad_usd", report.median_absolute_deviation(), 2);
        object.count("samples", report.sample_count);
        object.count("start_height", report.start_height.to_u32() as usize);
        object.count("stop_height", report.stop_height.to_u32() as usize);
        object.count("start_time", report.start_time.to_u32() as usize);
        object.count("stop_time", report.stop_time.to_u32() as usize);
        object.text("stop_hash", &self.stop_hash.to_display_hex());
        object.raw("algorithm", &self.params.to_json());
        object.finish()
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle::hash::Hash256;
    use delphi_oracle::OracleReport;

    use super::AuditRecord;
    use crate::{ChainWindow, NoteClass, NotePayload};

    struct AuditFixture;

    impl AuditFixture {
        fn record(hash: Hash256) -> AuditRecord {
            AuditRecord::new(
                ChainWindow::Daily,
                OracleReport {
                    central_price: 77_160.456,
                    deviation: 0.012_345_678,
                    sample_count: 96_006,
                    start_height: BlockHeight::new(965_075),
                    stop_height: BlockHeight::new(965_218),
                    start_time: BlockTime::new(1_780_000_000),
                    stop_time: BlockTime::new(1_780_086_000),
                },
                hash,
            )
        }

        fn hash() -> Hash256 {
            Hash256::hash(b"block")
        }
    }

    #[test]
    fn the_note_carries_the_window_price_block_and_algorithm() {
        let record = AuditFixture::record(AuditFixture::hash());
        let json: serde_json::Value = serde_json::from_str(&record.to_json()).expect("json");

        assert_eq!(json["window"], "chain-1d");
        assert_eq!(json["blocks"], 144);
        assert_eq!(json["price"], 77_160.46);
        let mad = json["mad_usd"].as_f64().expect("mad");
        assert!((mad - 0.012_345_678 * 0.2 * 77_160.456).abs() < 0.01, "{mad}");
        assert_eq!(json["stop_height"], 965_218);
        assert_eq!(json["stop_hash"], AuditFixture::hash().to_display_hex());
        assert_eq!(json["algorithm"]["name"], "delphi-oracle");
        assert!(record.to_json().len() < 700, "{} bytes", record.to_json().len());
    }

    #[test]
    fn the_note_is_dated_at_its_stop_block_and_stored() {
        let record = AuditFixture::record(AuditFixture::hash());

        assert_eq!(record.created_at(), Some(1_780_086_000));
        assert_eq!(record.class(), NoteClass::Audit);
        assert_eq!(record.extra_tags(), vec![("h", "965218".to_string())]);
    }

    #[test]
    fn a_replacement_points_at_the_note_it_replaces() {
        let record = AuditFixture::record(AuditFixture::hash()).replacing("abc");

        assert_eq!(record.replaces(), Some("abc"));
        assert!(record.extra_tags().contains(&("e", "abc".to_string())));
    }

    #[test]
    fn a_record_without_a_block_hash_is_not_publishable() {
        assert!(AuditFixture::record(AuditFixture::hash()).is_publishable());
        assert!(!AuditFixture::record(Hash256::ZERO).is_publishable());
    }
}
