use crate::{JsonObject, NoteClass, NotePayload};

/// One mempool price, streamed live at every check.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceUpdate {
    window: String,
    price: f64,
    samples: usize,
    transactions: usize,
    band_low: f64,
    band_high: f64,
}

impl PriceUpdate {
    pub const LABEL: &'static str = "mempool";

    #[must_use]
    pub fn new(
        window: impl Into<String>,
        price: f64,
        samples: usize,
        transactions: usize,
        band_low: f64,
        band_high: f64,
    ) -> Self {
        Self {
            window: window.into(),
            price,
            samples,
            transactions,
            band_low,
            band_high,
        }
    }

    #[must_use]
    pub const fn samples(&self) -> usize {
        self.samples
    }

    #[must_use]
    pub const fn transactions(&self) -> usize {
        self.transactions
    }

    #[must_use]
    pub const fn band(&self) -> (f64, f64) {
        (self.band_low, self.band_high)
    }
}

impl NotePayload for PriceUpdate {
    fn window(&self) -> &str {
        &self.window
    }

    fn label(&self) -> &str {
        Self::LABEL
    }

    fn price(&self) -> f64 {
        self.price
    }

    fn class(&self) -> NoteClass {
        NoteClass::Live
    }

    fn is_publishable(&self) -> bool {
        self.price.is_finite()
            && self.price > 0.0
            && self.band_low.is_finite()
            && self.band_high.is_finite()
            && self.band_low <= self.band_high
    }

    fn to_json(&self) -> String {
        let mut object = JsonObject::new();
        object.text("window", &self.window);
        object.text("source", Self::LABEL);
        object.fixed("price", self.price, 2);
        object.count("samples", self.samples);
        object.count("transactions", self.transactions);
        object.fixed("band_low", self.band_low, 2);
        object.fixed("band_high", self.band_high, 2);
        object.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::PriceUpdate;
    use crate::{NoteClass, NotePayload};

    struct UpdateFixture;

    impl UpdateFixture {
        fn mempool(low: f64, high: f64) -> PriceUpdate {
            PriceUpdate::new("mempool", 77_000.0, 412, 206, low, high)
        }

        fn parse(update: &PriceUpdate) -> serde_json::Value {
            serde_json::from_str(&update.to_json()).expect("valid json")
        }
    }

    #[test]
    fn a_mempool_update_carries_the_band_and_transaction_count() {
        let json = UpdateFixture::parse(&UpdateFixture::mempool(76_500.0, 77_400.0));

        assert_eq!(json["source"], "mempool");
        assert_eq!(json["price"], 77_000.0);
        assert_eq!(json["transactions"], 206);
        assert_eq!(json["band_low"], 76_500.0);
        assert_eq!(json["band_high"], 77_400.0);
    }

    #[test]
    fn a_mempool_update_is_live() {
        assert_eq!(UpdateFixture::mempool(1.0, 2.0).class(), NoteClass::Live);
    }

    #[test]
    fn a_window_name_is_escaped() {
        let update = PriceUpdate::new("we\"ird\\name\n", 1.0, 1, 1, 1.0, 1.0);

        assert_eq!(UpdateFixture::parse(&update)["window"], "we\"ird\\name\n");
    }

    #[test]
    fn non_finite_and_inverted_updates_are_not_publishable() {
        assert!(UpdateFixture::mempool(1.0, 2.0).is_publishable());
        assert!(!UpdateFixture::mempool(2.0, 1.0).is_publishable());
        assert!(!UpdateFixture::mempool(f64::NAN, 2.0).is_publishable());
        assert!(!PriceUpdate::new("w", f64::INFINITY, 1, 1, 1.0, 2.0).is_publishable());
        assert!(!PriceUpdate::new("w", 0.0, 1, 1, 1.0, 2.0).is_publishable());
    }

    #[test]
    fn a_non_finite_number_encodes_as_null_and_stays_parseable() {
        let update = UpdateFixture::mempool(f64::NAN, 1.0);

        assert!(UpdateFixture::parse(&update)["band_low"].is_null());
    }
}
