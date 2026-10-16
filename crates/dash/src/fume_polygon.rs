#[derive(Debug, Clone, PartialEq)]
pub struct FumePolygon {
    points: Vec<(f64, f64)>,
}

impl FumePolygon {
    #[must_use]
    pub const fn new(points: Vec<(f64, f64)>) -> Self {
        Self { points }
    }

    #[must_use]
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let mut inside = false;
        let count = self.points.len();
        for at in 0..count {
            let (one, two) = (self.points[at], self.points[(at + count - 1) % count]);
            if (one.1 > y) != (two.1 > y) && x < (two.0 - one.0) * (y - one.1) / (two.1 - one.1) + one.0 {
                inside = !inside;
            }
        }
        inside
    }

    #[must_use]
    pub fn spans(&self, y: f64) -> Vec<(f64, f64)> {
        let count = self.points.len();
        let mut crossings: Vec<f64> = (0..count)
            .filter_map(|at| {
                let (one, two) = (self.points[at], self.points[(at + count - 1) % count]);
                ((one.1 > y) != (two.1 > y)).then(|| (two.0 - one.0) * (y - one.1) / (two.1 - one.1) + one.0)
            })
            .collect();
        crossings.sort_by(f64::total_cmp);
        crossings.as_chunks::<2>().0.iter().map(|[from, to]| (*from, *to)).collect()
    }

    #[must_use]
    pub fn nearest_edge(&self, x: f64, y: f64) -> (f64, (f64, f64)) {
        let count = self.points.len();
        let turn = if self.signed_area() < 0.0 { -1.0 } else { 1.0 };
        let mut best = (f64::INFINITY, (0.0, 0.0));
        for at in 0..count {
            let (one, two) = (self.points[at], self.points[(at + 1) % count]);
            let (dx, dy) = (two.0 - one.0, two.1 - one.1);
            let length_squared = (dx * dx + dy * dy).max(f64::EPSILON);
            let along = (((x - one.0) * dx + (y - one.1) * dy) / length_squared).clamp(0.0, 1.0);
            let distance = (x - one.0 - dx * along).hypot(y - one.1 - dy * along);
            if distance < best.0 {
                let length = length_squared.sqrt();
                best = (distance, (turn * dy / length, -turn * dx / length));
            }
        }
        best
    }

    #[must_use]
    pub fn area(&self) -> f64 {
        self.signed_area().abs()
    }

    fn signed_area(&self) -> f64 {
        let count = self.points.len();
        (0..count)
            .map(|at| {
                let (one, two) = (self.points[at], self.points[(at + 1) % count]);
                one.0 * two.1 - two.0 * one.1
            })
            .sum::<f64>()
            * 0.5
    }

    #[must_use]
    pub fn top(&self) -> f64 {
        self.points.iter().map(|point| point.1).fold(f64::INFINITY, f64::min)
    }

    #[must_use]
    pub fn bottom(&self) -> f64 {
        self.points.iter().map(|point| point.1).fold(f64::NEG_INFINITY, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::FumePolygon;

    #[test]
    fn a_square_contains_its_middle_and_not_its_outside() {
        let square = FumePolygon::new(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);

        assert!(square.contains(5.0, 5.0));
        assert!(!square.contains(15.0, 5.0));
        assert!(!square.contains(5.0, -1.0));
        assert!((square.top()).abs() < 1e-9);
        assert!((square.bottom() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_row_through_a_notched_shape_splits_into_two_spans() {
        let notched = FumePolygon::new(vec![(0.0, 0.0), (4.0, 0.0), (4.0, 6.0), (6.0, 6.0), (6.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);

        assert_eq!(notched.spans(3.0), vec![(0.0, 4.0), (6.0, 10.0)]);
        assert_eq!(notched.spans(8.0), vec![(0.0, 10.0)]);
        assert!(notched.spans(12.0).is_empty());
    }

    #[test]
    fn the_nearest_edge_reports_its_distance_and_outward_normal_whichever_way_the_outline_runs() {
        let square = FumePolygon::new(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let reversed = FumePolygon::new(square.points().iter().rev().copied().collect());

        for shape in [square, reversed] {
            let (distance, normal) = shape.nearest_edge(5.0, 2.0);
            assert!((distance - 2.0).abs() < 1e-9);
            assert!((normal.0).abs() < 1e-9 && (normal.1 + 1.0).abs() < 1e-9, "{normal:?}");
            assert!((shape.area() - 100.0).abs() < 1e-9);
        }
    }

    #[test]
    fn a_notch_is_outside() {
        let notched = FumePolygon::new(vec![(0.0, 0.0), (4.0, 0.0), (4.0, 6.0), (6.0, 6.0), (6.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);

        assert!(!notched.contains(5.0, 3.0));
        assert!(notched.contains(5.0, 8.0));
    }
}
