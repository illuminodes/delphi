use std::fmt::Write;

pub struct SparklinePath {
    width: f64,
    height: f64,
    padding: f64,
}

impl SparklinePath {
    #[must_use]
    pub const fn new(width: f64, height: f64, padding: f64) -> Self {
        Self {
            width,
            height,
            padding,
        }
    }

    #[must_use]
    pub fn points(&self, history: &[(i64, f64)]) -> Option<String> {
        if history.len() < 2 {
            return None;
        }
        let first_time = history.first()?.0;
        let last_time = history.last()?.0;
        let time_span = (last_time - first_time).max(1) as f64;
        let low = history.iter().map(|point| point.1).fold(f64::INFINITY, f64::min);
        let high = history.iter().map(|point| point.1).fold(f64::NEG_INFINITY, f64::max);
        let value_span = high - low;
        let inner_width = 2.0f64.mul_add(-self.padding, self.width);
        let inner_height = 2.0f64.mul_add(-self.padding, self.height);

        let mut points = String::new();
        for (time, value) in history {
            let x = self.padding + (*time - first_time) as f64 / time_span * inner_width;
            let unit = if value_span > 0.0 {
                (value - low) / value_span
            } else {
                0.5
            };
            let y = self.padding + (1.0 - unit) * inner_height;
            if !points.is_empty() {
                points.push(' ');
            }
            let _ = write!(points, "{x:.1},{y:.1}");
        }
        Some(points)
    }
}

#[cfg(test)]
mod tests {
    use super::SparklinePath;

    struct PointsFixture;

    impl PointsFixture {
        fn coordinates(points: &str) -> Vec<(f64, f64)> {
            points
                .split(' ')
                .map(|pair| {
                    let (x, y) = pair.split_once(',').expect("pair");
                    (x.parse().expect("x"), y.parse().expect("y"))
                })
                .collect()
        }
    }

    #[test]
    fn fewer_than_two_points_draw_nothing() {
        let path = SparklinePath::new(100.0, 40.0, 2.0);

        assert_eq!(path.points(&[]), None);
        assert_eq!(path.points(&[(1, 5.0)]), None);
    }

    #[test]
    fn the_series_spans_the_padded_box_with_high_values_at_the_top() {
        let path = SparklinePath::new(100.0, 40.0, 2.0);
        let points = PointsFixture::coordinates(
            &path.points(&[(0, 10.0), (50, 30.0), (100, 20.0)]).expect("points"),
        );

        assert_eq!(points[0], (2.0, 38.0));
        assert_eq!(points[1], (50.0, 2.0));
        assert_eq!(points[2], (98.0, 20.0));
    }

    #[test]
    fn a_flat_series_is_drawn_on_the_middle_line() {
        let path = SparklinePath::new(100.0, 40.0, 0.0);
        let points = PointsFixture::coordinates(&path.points(&[(0, 7.0), (10, 7.0)]).expect("points"));

        assert!(points.iter().all(|point| (point.1 - 20.0).abs() < 1e-9));
    }

    #[test]
    fn identical_timestamps_do_not_divide_by_zero() {
        let path = SparklinePath::new(100.0, 40.0, 0.0);
        let points = path.points(&[(5, 1.0), (5, 2.0)]).expect("points");

        assert!(!points.contains("NaN") && !points.contains("inf"), "{points}");
    }
}
