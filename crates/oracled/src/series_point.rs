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
    pub const PRICE_DECIMALS: usize = 2;
    pub const DEVIATION_DECIMALS: usize = 4;

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
        object.fixed("price", self.price, Self::PRICE_DECIMALS);
        object.fixed("deviation", self.deviation, Self::DEVIATION_DECIMALS);
        object.count("samples", self.samples);
        object.count("stop_height", self.stop_height.to_u32() as usize);
        object.count("stop_time", self.stop_time.to_u32() as usize);
        object.finish()
    }

    /// A JSON array of the points, oldest first as given.
    #[must_use]
    pub fn array_json(points: &[Self]) -> String {
        Self::array_json_within(points, usize::MAX)
    }

    /// A JSON array of at most `budget` bytes, dropping the oldest points first.
    #[must_use]
    pub fn array_json_within(points: &[Self], budget: usize) -> String {
        let mut kept: Vec<String> = Vec::new();
        let mut used: usize = 2;
        for point in points.iter().rev() {
            let encoded = point.to_json();
            let cost = encoded.len() + usize::from(!kept.is_empty());
            if used.saturating_add(cost) > budget {
                break;
            }
            used += cost;
            kept.push(encoded);
        }
        kept.reverse();
        format!("[{}]", kept.join(","))
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
    fn prices_are_rounded_to_cents_and_deviations_to_four_places() {
        let text = point(84_459.459_459_459, 0.171_580_237_428).to_json();

        assert!(text.contains("\"price\":84459.46,"), "{text}");
        assert!(text.contains("\"deviation\":0.1716,"), "{text}");
    }

    #[test]
    fn a_budget_drops_the_oldest_points_first() {
        let points: Vec<SeriesPoint> = (11..=20).map(|index| point(f64::from(index), 0.0)).collect();
        let one = points[0].to_json().len();
        let json: serde_json::Value =
            serde_json::from_str(&SeriesPoint::array_json_within(&points, 3 * one + 4)).expect("json");
        let kept = json.as_array().expect("array");

        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0]["price"], 18.0);
        assert_eq!(kept[2]["price"], 20.0);
    }

    #[test]
    fn a_zero_budget_keeps_nothing() {
        assert_eq!(SeriesPoint::array_json_within(&[point(1.0, 0.0)], 0), "[]");
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
