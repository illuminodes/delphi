use mantle::difficulty::{BlockHeight, BlockTime};
use mantle_oracle::OracleReport;

use crate::JsonObject;

/// One price in a timeline: the estimate over the window ending at `stop_height`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesPoint {
    pub price: f64,
    pub deviation: f64,
    pub samples: usize,
    pub stop_height: BlockHeight,
    pub stop_time: BlockTime,
}

impl SeriesPoint {
    #[must_use]
    pub const fn from_report(report: &OracleReport) -> Self {
        Self {
            price: report.central_price,
            deviation: report.deviation,
            samples: report.sample_count,
            stop_height: report.stop_height,
            stop_time: report.stop_time,
        }
    }

    #[must_use]
    pub fn is_publishable(&self) -> bool {
        self.price.is_finite() && self.price > 0.0 && self.deviation.is_finite()
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        let mut object = JsonObject::new();
        object.float("price", self.price);
        object.float("deviation", self.deviation);
        object.count("samples", self.samples);
        object.count("stop_height", self.stop_height.to_u32() as usize);
        object.count("stop_time", self.stop_time.to_u32() as usize);
        object.finish()
    }

    /// A JSON array of the points, oldest first as given.
    #[must_use]
    pub fn array_json(points: &[Self]) -> String {
        let encoded: Vec<String> = points.iter().map(Self::to_json).collect();
        format!("[{}]", encoded.join(","))
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};

    use super::SeriesPoint;

    fn point(price: f64, deviation: f64) -> SeriesPoint {
        SeriesPoint {
            price,
            deviation,
            samples: 4_000,
            stop_height: BlockHeight::new(100),
            stop_time: BlockTime::new(1_780_000_000),
        }
    }

    #[test]
    fn a_point_encodes_its_price_and_end() {
        let json: serde_json::Value = serde_json::from_str(&point(80_000.5, 0.02).to_json()).expect("json");

        assert_eq!(json["price"], 80_000.5);
        assert_eq!(json["samples"], 4_000);
        assert_eq!(json["stop_height"], 100);
        assert_eq!(json["stop_time"], 1_780_000_000_u64);
    }

    #[test]
    fn a_point_with_a_bad_number_is_not_publishable() {
        assert!(point(80_000.0, 0.02).is_publishable());
        assert!(!point(f64::NAN, 0.02).is_publishable());
        assert!(!point(0.0, 0.02).is_publishable());
        assert!(!point(80_000.0, f64::INFINITY).is_publishable());
    }

    #[test]
    fn points_encode_as_an_array_in_the_order_given() {
        let json: serde_json::Value =
            serde_json::from_str(&SeriesPoint::array_json(&[point(1.0, 0.0), point(2.0, 0.0)])).expect("json");

        assert_eq!(json[0]["price"], 1.0);
        assert_eq!(json[1]["price"], 2.0);
        assert_eq!(SeriesPoint::array_json(&[]), "[]");
    }
}
