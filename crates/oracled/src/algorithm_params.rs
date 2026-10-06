use mantle_oracle::{CentralPriceFinder, OutputFilter, PriceStencil, RoughPriceEstimator};

use crate::JsonObject;

/// The algorithm and settings behind a chain price, carried in every audit
/// note so a reader can reproduce it from the same blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlgorithmParams {
    pub version: usize,
    pub smooth_weight: f64,
    pub smooth_below: i32,
    pub tight_range: f64,
    pub deviation_range: f64,
    pub min_output_sats: u64,
    pub max_output_sats: u64,
    pub max_inputs: usize,
    pub required_outputs: usize,
}

impl AlgorithmParams {
    pub const NAME: &'static str = "mantle-oracle";
    pub const VERSION: usize = 1;

    #[must_use]
    pub const fn current() -> Self {
        Self {
            version: Self::VERSION,
            smooth_weight: PriceStencil::DEFAULT_SMOOTH_WEIGHT,
            smooth_below: RoughPriceEstimator::DEFAULT_SMOOTH_BELOW,
            tight_range: CentralPriceFinder::TIGHT_RANGE,
            deviation_range: CentralPriceFinder::DEVIATION_RANGE,
            min_output_sats: OutputFilter::MIN_OUTPUT.to_u64(),
            max_output_sats: OutputFilter::MAX_OUTPUT.to_u64(),
            max_inputs: OutputFilter::MAX_INPUTS,
            required_outputs: OutputFilter::REQUIRED_OUTPUTS,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        let mut object = JsonObject::new();
        object.text("name", Self::NAME);
        object.count("version", self.version);
        object.float("smooth_weight", self.smooth_weight);
        object.raw("smooth_below", &self.smooth_below.to_string());
        object.float("tight_range", self.tight_range);
        object.float("deviation_range", self.deviation_range);
        object.raw("min_output_sats", &self.min_output_sats.to_string());
        object.raw("max_output_sats", &self.max_output_sats.to_string());
        object.count("max_inputs", self.max_inputs);
        object.count("required_outputs", self.required_outputs);
        object.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::AlgorithmParams;

    #[test]
    fn the_current_parameters_encode_as_json() {
        let json: serde_json::Value =
            serde_json::from_str(&AlgorithmParams::current().to_json()).expect("json");

        assert_eq!(json["name"], "mantle-oracle");
        assert_eq!(json["version"], AlgorithmParams::VERSION);
        assert_eq!(json["smooth_weight"], 0.65);
        assert_eq!(json["smooth_below"], 150);
        assert_eq!(json["required_outputs"], 2);
    }
}
