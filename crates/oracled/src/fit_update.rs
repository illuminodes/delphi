use std::fmt::Write;

use mantle_oracle::{DenominationFit, PriceBand, PriceFit};

use crate::{JsonObject, NoteClass, NotePayload};

/// The evidence behind one window's price, streamed live under the window
/// name `fit-<window>`.
#[derive(Debug, Clone, PartialEq)]
pub struct FitUpdate {
    window: String,
    source: &'static str,
    fit: PriceFit,
}

impl FitUpdate {
    pub const PREFIX: &'static str = "fit-";
    pub const LABEL: &'static str = "fit";
    pub const SCORE_DECIMALS: usize = 3;

    #[must_use]
    pub fn new(price_window: &str, source: &'static str, fit: PriceFit) -> Self {
        Self {
            window: format!("{}{price_window}", Self::PREFIX),
            source,
            fit,
        }
    }

    #[must_use]
    pub const fn fit(&self) -> &PriceFit {
        &self.fit
    }

    fn curve_json(&self) -> String {
        let curve = &self.fit.curve;
        let mut object = JsonObject::new();
        let prices = curve.prices();
        object.fixed("first_price", prices.first().copied().unwrap_or(f64::NAN), 2);
        let step = match prices {
            [first, second, ..] => second / first,
            _ => f64::NAN,
        };
        object.fixed("step", step, 8);
        object.raw("scores", &Self::number_array(curve.scores(), Self::SCORE_DECIMALS));
        object.finish()
    }

    fn implied_json(&self) -> String {
        let implied = &self.fit.implied;
        let mut object = JsonObject::new();
        object.fixed("low", implied.low(), 2);
        object.fixed("high", implied.high(), 2);
        let counts: Vec<String> = implied.counts().iter().map(u32::to_string).collect();
        object.raw("counts", &format!("[{}]", counts.join(",")));
        object.finish()
    }

    fn denominations_json(&self) -> String {
        let rows: Vec<String> = self
            .fit
            .denominations
            .iter()
            .map(Self::denomination_json)
            .collect();
        format!("[{}]", rows.join(","))
    }

    fn denomination_json(row: &DenominationFit) -> String {
        let mut object = JsonObject::new();
        object.count("usd", row.usd as usize);
        object.count("points", row.points);
        object.count("inside", row.inside);
        object.fixed("median", row.median.unwrap_or(f64::NAN), 2);
        object.finish()
    }

    fn number_array(values: &[f64], decimals: usize) -> String {
        let mut text = String::from("[");
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                text.push(',');
            }
            if value.is_finite() {
                let _ = write!(text, "{value:.decimals$}");
            } else {
                text.push_str("null");
            }
        }
        text.push(']');
        text
    }
}

impl NotePayload for FitUpdate {
    fn window(&self) -> &str {
        &self.window
    }

    fn label(&self) -> &str {
        Self::LABEL
    }

    fn price(&self) -> f64 {
        self.fit.report.central_price
    }

    fn class(&self) -> NoteClass {
        NoteClass::Live
    }

    fn is_publishable(&self) -> bool {
        let report = &self.fit.report;
        report.central_price.is_finite()
            && report.central_price > 0.0
            && self.fit.rough_price.is_finite()
            && !self.fit.curve.is_empty()
    }

    fn to_json(&self) -> String {
        let fit = &self.fit;
        let report = &fit.report;
        let mut object = JsonObject::new();
        object.text("window", &self.window);
        object.text("source", self.source);
        object.fixed("price", report.central_price, 2);
        object.fixed("rough_price", fit.rough_price, 2);
        object.fixed("deviation", report.deviation, 4);
        object.count("samples", report.sample_count);
        object.count("stop_height", report.stop_height.to_u32() as usize);
        object.count("stop_time", report.stop_time.to_u32() as usize);
        object.fixed("band_low", fit.band.map_or(f64::NAN, PriceBand::low), 2);
        object.fixed("band_high", fit.band.map_or(f64::NAN, PriceBand::high), 2);
        object.fixed("octave_ratio", fit.octave_ratio.unwrap_or(f64::NAN), 4);
        object.raw("curve", &self.curve_json());
        object.raw("implied", &self.implied_json());
        object.raw("denominations", &self.denominations_json());
        object.finish()
    }
}

#[cfg(test)]
mod tests {
    use mantle_oracle::{PriceBand, RoughPriceEstimator};

    use super::FitUpdate;
    use crate::NotePayload;
    use crate::synthetic_market::SyntheticMarket;

    struct FitFixture;

    impl FitFixture {
        fn update(band: bool) -> FitUpdate {
            let sample = SyntheticMarket::new(80_000.0, 3).sample(600);
            let band = band.then(|| PriceBand::around(80_000.0, 0.15).expect("band"));
            let fit = sample.fit(&RoughPriceEstimator::new(), band).expect("fit");
            FitUpdate::new("mempool", "mempool", fit)
        }

        fn parse(update: &FitUpdate) -> serde_json::Value {
            serde_json::from_str(&update.to_json()).expect("valid json")
        }
    }

    #[test]
    fn the_window_is_prefixed_and_labelled() {
        let update = FitFixture::update(true);

        assert_eq!(update.window(), "fit-mempool");
        assert_eq!(update.label(), "fit");
        assert!(update.is_publishable());
    }

    #[test]
    fn the_curve_rebuilds_from_its_first_price_and_step() {
        let update = FitFixture::update(true);
        let json = FitFixture::parse(&update);
        let first = json["curve"]["first_price"].as_f64().expect("first");
        let step = json["curve"]["step"].as_f64().expect("step");
        let prices = update.fit().curve.prices();
        let last = prices.len() - 1;
        let rebuilt = first * step.powi(i32::try_from(last).expect("fits"));

        assert_eq!(json["curve"]["scores"].as_array().map(Vec::len), Some(prices.len()));
        assert!((rebuilt - prices[last]).abs() / prices[last] < 1e-4, "{rebuilt} vs {}", prices[last]);
    }

    #[test]
    fn the_note_carries_the_band_and_every_denomination() {
        let json = FitFixture::parse(&FitFixture::update(true));

        assert!(json["band_low"].as_f64().expect("low") < json["price"].as_f64().expect("price"));
        assert_eq!(json["denominations"].as_array().map(Vec::len), Some(14));
        assert_eq!(json["implied"]["counts"].as_array().map(Vec::len), Some(60));
    }

    #[test]
    fn an_unbanded_fit_encodes_a_null_band() {
        let json = FitFixture::parse(&FitFixture::update(false));

        assert!(json["band_low"].is_null());
        assert!(json["band_high"].is_null());
    }

    #[test]
    fn the_note_stays_well_under_the_relay_limit() {
        let size = FitFixture::update(true).to_json().len();

        assert!(size < 8_000, "{size} bytes");
    }
}
