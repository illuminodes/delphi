use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SeriesPoint {
    price: f64,
    deviation: Option<f64>,
    samples: Option<usize>,
    stop_height: Option<u32>,
    stop_time: i64,
}

impl SeriesPoint {
    #[must_use]
    pub const fn new(
        price: f64,
        deviation: Option<f64>,
        samples: usize,
        stop_height: Option<u32>,
        stop_time: i64,
    ) -> Self {
        Self {
            price,
            deviation,
            samples: Some(samples),
            stop_height,
            stop_time,
        }
    }

    #[must_use]
    pub const fn price(&self) -> f64 {
        self.price
    }

    #[must_use]
    pub const fn deviation(&self) -> Option<f64> {
        self.deviation
    }

    #[must_use]
    pub const fn samples(&self) -> usize {
        match self.samples {
            Some(samples) => samples,
            None => 0,
        }
    }

    #[must_use]
    pub const fn stop_height(&self) -> Option<u32> {
        self.stop_height
    }

    #[must_use]
    pub const fn stop_time(&self) -> i64 {
        self.stop_time
    }

    #[must_use]
    pub fn is_plottable(&self) -> bool {
        self.price.is_finite() && self.price > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::SeriesPoint;

    #[test]
    fn a_full_point_is_read_from_the_daemon_shape() {
        let point: SeriesPoint = serde_json::from_str(
            r#"{"price":83514.28,"deviation":0.1726,"samples":92746,"stop_height":969218,"stop_time":1790722852}"#,
        )
        .expect("point");

        assert!((point.price() - 83_514.28).abs() < 1e-9);
        assert_eq!(point.samples(), 92_746);
        assert_eq!(point.stop_height(), Some(969_218));
        assert_eq!(point.stop_time(), 1_790_722_852);
        assert!(point.is_plottable());
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let point: SeriesPoint =
            serde_json::from_str(r#"{"price":1.5,"stop_time":10}"#).expect("point");

        assert_eq!(point.samples(), 0);
        assert_eq!(point.deviation(), None);
        assert_eq!(point.stop_height(), None);
    }

    #[test]
    fn a_non_positive_price_is_not_plottable() {
        let point: SeriesPoint =
            serde_json::from_str(r#"{"price":0,"stop_time":10}"#).expect("point");

        assert!(!point.is_plottable());
    }
}
