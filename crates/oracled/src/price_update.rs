use mantle::difficulty::{BlockHeight, BlockTime};
use mantle_oracle::OracleReport;

use crate::{JsonObject, NotePayload, SeriesPoint};

/// What a published price was measured from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PriceSource {
    Chain {
        deviation: f64,
        start_height: BlockHeight,
        stop_height: BlockHeight,
        start_time: BlockTime,
        stop_time: BlockTime,
    },
    Mempool {
        transactions: usize,
        band_low: f64,
        band_high: f64,
    },
}

impl PriceSource {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Chain { .. } => "chain",
            Self::Mempool { .. } => "mempool",
        }
    }
}

/// One price for one named window, ready to publish. A chain window also
/// carries its timeline, oldest point first, ending at the headline price.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceUpdate {
    window: String,
    price: f64,
    samples: usize,
    source: PriceSource,
    series: Vec<SeriesPoint>,
}

impl PriceUpdate {
    /// Relays refuse notes near 20 KB without saying so, so the timeline is
    /// trimmed to fit well inside that.
    pub const SERIES_BUDGET_BYTES: usize = 16_000;

    #[must_use]
    pub fn new(window: impl Into<String>, price: f64, samples: usize, source: PriceSource) -> Self {
        Self {
            window: window.into(),
            price,
            samples,
            source,
            series: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_series(mut self, series: Vec<SeriesPoint>) -> Self {
        self.series = series;
        self
    }

    #[must_use]
    pub fn series(&self) -> &[SeriesPoint] {
        &self.series
    }

    #[must_use]
    pub fn chain(window: impl Into<String>, report: &OracleReport) -> Self {
        Self::new(
            window,
            report.central_price,
            report.sample_count,
            PriceSource::Chain {
                deviation: report.deviation,
                start_height: report.start_height,
                stop_height: report.stop_height,
                start_time: report.start_time,
                stop_time: report.stop_time,
            },
        )
    }

    #[must_use]
    pub fn window(&self) -> &str {
        &self.window
    }

    #[must_use]
    pub const fn price(&self) -> f64 {
        self.price
    }

    #[must_use]
    pub const fn samples(&self) -> usize {
        self.samples
    }

    #[must_use]
    pub const fn source(&self) -> &PriceSource {
        &self.source
    }

    /// Whether every number in the update is finite and the price positive,
    /// which is the precondition for encoding it as JSON.
    #[must_use]
    pub fn is_publishable(&self) -> bool {
        let source_finite = match self.source {
            PriceSource::Chain { deviation, .. } => deviation.is_finite(),
            PriceSource::Mempool {
                band_low,
                band_high,
                ..
            } => band_low.is_finite() && band_high.is_finite() && band_low <= band_high,
        };
        self.price.is_finite()
            && self.price > 0.0
            && source_finite
            && self.series.iter().all(SeriesPoint::is_publishable)
    }

    /// The JSON object carried as the note content.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut object = JsonObject::new();
        object.text("window", &self.window);
        object.text("source", self.source.label());
        object.float("price", self.price);
        object.count("samples", self.samples);
        match self.source {
            PriceSource::Chain {
                deviation,
                start_height,
                stop_height,
                start_time,
                stop_time,
            } => {
                object.float("deviation", deviation);
                object.count("start_height", start_height.to_u32() as usize);
                object.count("stop_height", stop_height.to_u32() as usize);
                object.count("start_time", start_time.to_u32() as usize);
                object.count("stop_time", stop_time.to_u32() as usize);
            }
            PriceSource::Mempool {
                transactions,
                band_low,
                band_high,
            } => {
                object.count("transactions", transactions);
                object.float("band_low", band_low);
                object.float("band_high", band_high);
            }
        }
        if !self.series.is_empty() {
            object.raw(
                "points",
                &SeriesPoint::array_json_within(&self.series, Self::SERIES_BUDGET_BYTES),
            );
        }
        object.finish()
    }
}

impl NotePayload for PriceUpdate {
    fn window(&self) -> &str {
        &self.window
    }

    fn label(&self) -> &str {
        self.source.label()
    }

    fn price(&self) -> f64 {
        self.price
    }

    fn is_publishable(&self) -> bool {
        Self::is_publishable(self)
    }

    fn to_json(&self) -> String {
        Self::to_json(self)
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle_oracle::OracleReport;

    use super::{PriceSource, PriceUpdate};

    struct UpdateFixture;

    impl UpdateFixture {
        fn chain() -> PriceUpdate {
            PriceUpdate::chain(
                "chain-1d",
                &OracleReport {
                    central_price: 77_160.5,
                    deviation: 0.0123,
                    sample_count: 96_006,
                    start_height: BlockHeight::new(965_075),
                    stop_height: BlockHeight::new(965_218),
                    start_time: BlockTime::new(1_780_000_000),
                    stop_time: BlockTime::new(1_780_086_000),
                },
            )
        }

        fn mempool(low: f64, high: f64) -> PriceUpdate {
            PriceUpdate::new(
                "mempool",
                77_000.0,
                412,
                PriceSource::Mempool {
                    transactions: 206,
                    band_low: low,
                    band_high: high,
                },
            )
        }

        fn parse(update: &PriceUpdate) -> serde_json::Value {
            serde_json::from_str(&update.to_json()).expect("valid json")
        }
    }

    #[test]
    fn a_chain_update_carries_the_report_fields() {
        let json = UpdateFixture::parse(&UpdateFixture::chain());

        assert_eq!(json["window"], "chain-1d");
        assert_eq!(json["source"], "chain");
        assert_eq!(json["price"], 77_160.5);
        assert_eq!(json["samples"], 96_006);
        assert_eq!(json["start_height"], 965_075);
        assert_eq!(json["stop_height"], 965_218);
        assert_eq!(json["stop_time"], 1_780_086_000_u64);
        assert!((json["deviation"].as_f64().expect("number") - 0.0123).abs() < 1e-12);
    }

    #[test]
    fn a_mempool_update_carries_the_band_and_transaction_count() {
        let json = UpdateFixture::parse(&UpdateFixture::mempool(76_500.0, 77_400.0));

        assert_eq!(json["source"], "mempool");
        assert_eq!(json["transactions"], 206);
        assert_eq!(json["band_low"], 76_500.0);
        assert_eq!(json["band_high"], 77_400.0);
        assert!(json.get("start_height").is_none());
    }

    #[test]
    fn a_window_name_is_escaped() {
        let update = PriceUpdate::new(
            "we\"ird\\name\n",
            1.0,
            1,
            PriceSource::Mempool {
                transactions: 1,
                band_low: 1.0,
                band_high: 1.0,
            },
        );

        assert_eq!(UpdateFixture::parse(&update)["window"], "we\"ird\\name\n");
    }

    #[test]
    fn non_finite_and_inverted_updates_are_not_publishable() {
        assert!(UpdateFixture::chain().is_publishable());
        assert!(UpdateFixture::mempool(1.0, 2.0).is_publishable());
        assert!(!UpdateFixture::mempool(2.0, 1.0).is_publishable());
        assert!(!UpdateFixture::mempool(f64::NAN, 2.0).is_publishable());
        assert!(!PriceUpdate::new("w", f64::INFINITY, 1, *UpdateFixture::chain().source()).is_publishable());
        assert!(!PriceUpdate::new("w", 0.0, 1, *UpdateFixture::chain().source()).is_publishable());
    }

    #[test]
    fn a_non_finite_number_encodes_as_null_and_stays_parseable() {
        let update = UpdateFixture::mempool(f64::NAN, 1.0);

        assert!(UpdateFixture::parse(&update)["band_low"].is_null());
    }
}
