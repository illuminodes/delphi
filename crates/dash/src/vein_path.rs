use crate::VeinPoint;

#[derive(Debug, Clone, PartialEq)]
pub struct VeinPath {
    points: Vec<(f64, f64)>,
    along: Vec<f64>,
    first: f64,
    last: f64,
}

impl VeinPath {
    pub const REACH: f64 = 16.0;
    const TAPER: f64 = 36.0;

    #[must_use]
    pub fn new(points: &[(f64, f64)]) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let (first, last) = (points[0], points[points.len() - 1]);
        let mut extended = Vec::with_capacity(points.len() + 2);
        extended.push((first.0 - 2.0 * Self::TAPER, first.1));
        extended.extend_from_slice(points);
        extended.push((last.0 + 2.0 * Self::TAPER, last.1));
        let mut along = Vec::with_capacity(extended.len());
        let mut total = 0.0;
        along.push(total);
        for pair in extended.windows(2) {
            total += (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
            along.push(total);
        }
        Some(Self {
            points: extended,
            along,
            first: first.0,
            last: last.0,
        })
    }

    #[must_use]
    pub fn nearest(&self, x: f64, y: f64) -> Option<VeinPoint> {
        let start = self.points.partition_point(|point| point.0 < x - Self::REACH).saturating_sub(1);
        let mut best: Option<(f64, VeinPoint)> = None;
        for index in start..self.points.len() - 1 {
            let (a, b) = (self.points[index], self.points[index + 1]);
            if a.0 > x + Self::REACH {
                break;
            }
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let length_squared = dx * dx + dy * dy;
            let t = if length_squared < f64::EPSILON {
                0.0
            } else {
                (((x - a.0) * dx + (y - a.1) * dy) / length_squared).clamp(0.0, 1.0)
            };
            let (qx, qy) = (a.0 + dx * t, a.1 + dy * t);
            let distance = (x - qx).hypot(y - qy);
            if distance > Self::REACH || best.is_some_and(|(kept, _)| kept <= distance) {
                continue;
            }
            let side = if dx * (y - a.1) - dy * (x - a.0) < 0.0 { -1.0 } else { 1.0 };
            best = Some((
                distance,
                VeinPoint {
                    offset: side * distance,
                    along: self.along[index] + t * length_squared.sqrt(),
                    strength: self.strength(qx),
                },
            ));
        }
        best.map(|(_, point)| point)
    }

    fn strength(&self, x: f64) -> f64 {
        let beyond = (self.first - x).max(x - self.last).max(0.0) / Self::TAPER;
        (-beyond * beyond).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::VeinPath;

    struct PathFixture;

    impl PathFixture {
        fn zigzag() -> VeinPath {
            VeinPath::new(&[(100.0, 100.0), (200.0, 100.0), (202.0, 20.0), (300.0, 20.0)]).unwrap()
        }
    }

    #[test]
    fn fewer_than_two_points_make_no_path() {
        assert!(VeinPath::new(&[(1.0, 2.0)]).is_none());
    }

    #[test]
    fn a_point_on_the_line_has_no_offset_and_full_strength() {
        let point = PathFixture::zigzag().nearest(150.0, 100.0).unwrap();

        assert!(point.offset.abs() < 1e-9);
        assert!((point.strength - 1.0).abs() < 1e-9);
    }

    #[test]
    fn below_a_rightward_line_is_positive_and_above_is_negative() {
        let path = PathFixture::zigzag();

        assert!((path.nearest(150.0, 106.0).unwrap().offset - 6.0).abs() < 1e-9);
        assert!((path.nearest(150.0, 94.0).unwrap().offset + 6.0).abs() < 1e-9);
    }

    #[test]
    fn beside_a_steep_segment_the_distance_is_measured_across_it() {
        let point = PathFixture::zigzag().nearest(196.0, 60.0).unwrap();

        assert!((point.offset.abs() - 5.0).abs() < 0.05, "{}", point.offset);
    }

    #[test]
    fn the_position_along_the_line_grows_with_the_line() {
        let path = PathFixture::zigzag();
        let alongs: Vec<f64> = [(120.0, 100.0), (199.0, 100.0), (201.0, 60.0), (250.0, 20.0)]
            .iter()
            .map(|(x, y)| path.nearest(*x, *y).unwrap().along)
            .collect();

        assert!(alongs.windows(2).all(|pair| pair[1] > pair[0]), "{alongs:?}");
    }

    #[test]
    fn stone_out_of_reach_of_the_line_is_not_vein() {
        assert!(PathFixture::zigzag().nearest(150.0, 140.0).is_none());
    }

    #[test]
    fn the_vein_tapers_away_past_the_first_and_last_prices() {
        let path = PathFixture::zigzag();
        let inside = path.nearest(100.0, 100.0).unwrap().strength;
        let past = path.nearest(64.0, 100.0).unwrap().strength;
        let far = path.nearest(30.0, 100.0).unwrap().strength;

        assert!((inside - 1.0).abs() < 1e-9);
        assert!(past < 0.5 && far < past && far < 0.05, "{past} {far}");
    }
}
