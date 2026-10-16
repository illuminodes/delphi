#[derive(Debug, Clone, PartialEq)]
pub struct VeinCourse {
    trend: Vec<f64>,
}

impl VeinCourse {
    const PASSES: usize = 3;

    #[must_use]
    pub fn new(points: &[(f64, f64)], width: f64, radius: usize) -> Option<Self> {
        if points.len() < 2 || width.is_nan() || width <= 0.0 {
            return None;
        }
        let columns = width.ceil() as usize + 1;
        let mut segment = 0;
        let line: Vec<f64> = (0..columns)
            .map(|column| {
                let x = column as f64;
                while segment + 2 < points.len() && points[segment + 1].0 < x {
                    segment += 1;
                }
                let ((x0, y0), (x1, y1)) = (points[segment], points[segment + 1]);
                if x <= x0 {
                    return y0;
                }
                if x >= x1 || (x1 - x0).abs() < f64::EPSILON {
                    return y1;
                }
                y0 + (y1 - y0) * (x - x0) / (x1 - x0)
            })
            .collect();
        Some(Self {
            trend: (0..Self::PASSES).fold(line, |values, _| Self::blur(&values, radius)),
        })
    }

    #[must_use]
    pub fn trend(&self, x: f64) -> f64 {
        let last = self.trend.len() - 1;
        let at = x.clamp(0.0, last as f64);
        let left = at.floor() as usize;
        let right = (left + 1).min(last);
        let blend = at - left as f64;
        self.trend[left] * (1.0 - blend) + self.trend[right] * blend
    }

    fn blur(values: &[f64], radius: usize) -> Vec<f64> {
        let mut sums = Vec::with_capacity(values.len() + 1);
        sums.push(0.0);
        for value in values {
            sums.push(sums.last().copied().unwrap_or(0.0) + value);
        }
        (0..values.len())
            .map(|index| {
                let low = index.saturating_sub(radius);
                let high = (index + radius).min(values.len() - 1);
                (sums[high + 1] - sums[low]) / (high - low + 1) as f64
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::VeinCourse;

    #[test]
    fn fewer_than_two_points_or_no_width_make_no_course() {
        assert!(VeinCourse::new(&[(0.0, 1.0)], 10.0, 4).is_none());
        assert!(VeinCourse::new(&[(0.0, 1.0), (10.0, 2.0)], 0.0, 4).is_none());
        assert!(VeinCourse::new(&[(0.0, 1.0), (10.0, 2.0)], f64::NAN, 4).is_none());
    }

    #[test]
    fn a_flat_line_has_a_flat_trend() {
        let course = VeinCourse::new(&[(0.0, 40.0), (300.0, 40.0)], 300.0, 40).unwrap();

        assert!([0.0, 150.0, 300.0].iter().all(|x| (course.trend(*x) - 40.0).abs() < 1e-9));
    }

    #[test]
    fn a_spike_barely_moves_a_wide_trend() {
        let course = VeinCourse::new(&[(0.0, 100.0), (149.0, 100.0), (150.0, 20.0), (151.0, 100.0), (300.0, 100.0)], 300.0, 40).unwrap();

        assert!(course.trend(150.0) > 97.0, "{}", course.trend(150.0));
    }

    #[test]
    fn a_narrow_course_keeps_close_to_the_line() {
        let points = [(0.0, 100.0), (100.0, 100.0), (150.0, 40.0), (200.0, 100.0), (300.0, 100.0)];
        let narrow = VeinCourse::new(&points, 300.0, 1).unwrap();
        let wide = VeinCourse::new(&points, 300.0, 40).unwrap();

        assert!((narrow.trend(150.0) - 40.0).abs() < 3.0, "{}", narrow.trend(150.0));
        assert!(wide.trend(150.0) > narrow.trend(150.0) + 20.0);
    }

    #[test]
    fn a_jagged_line_gives_a_gentle_trend() {
        let points: Vec<(f64, f64)> = (0..60).map(|step| (f64::from(step) * 10.0, if step % 2 == 0 { 40.0 } else { 160.0 })).collect();
        let course = VeinCourse::new(&points, 590.0, 40).unwrap();
        let steepest = (0..589).map(|x| (course.trend(f64::from(x) + 1.0) - course.trend(f64::from(x))).abs()).fold(0.0, f64::max);

        assert!(steepest < 0.5, "{steepest}");
    }
}
