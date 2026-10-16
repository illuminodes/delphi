use crate::{CarveBox, FumePolygon, WallJoint};

#[derive(Debug, Clone, PartialEq)]
pub struct WallStone {
    pub block: usize,
    pub course: usize,
    pub bottom: Vec<(f64, f64)>,
    pub top: Vec<(f64, f64)>,
    pub left: WallJoint,
    pub right: WallJoint,
}

impl WallStone {
    #[must_use]
    pub fn shifted(&self, dx: f64, dy: f64) -> Self {
        let move_all = |points: &[(f64, f64)]| points.iter().map(|(x, y)| (x + dx, y + dy)).collect();
        Self { block: self.block, course: self.course, bottom: move_all(&self.bottom), top: move_all(&self.top), left: self.left.shifted(dx, dy), right: self.right.shifted(dx, dy) }
    }

    #[must_use]
    pub fn outline(&self) -> FumePolygon {
        let bottom = self.bottom.iter().copied().chain([self.right.knee]);
        FumePolygon::new(bottom.chain(self.top.iter().rev().copied()).chain([self.left.knee]).collect())
    }

    #[must_use]
    pub fn bounds(&self) -> CarveBox {
        let points = self.bottom.iter().chain(&self.top).chain([&self.left.knee, &self.right.knee]);
        let start = CarveBox::new(f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        points.fold(start, |bounds, (x, y)| CarveBox::new(bounds.left.min(*x), bounds.top.min(*y), bounds.right.max(*x), bounds.bottom.max(*y)))
    }

    #[must_use]
    pub fn face(&self, margin: f64) -> CarveBox {
        let floor = self.bottom.iter().map(|point| point.1).fold(f64::INFINITY, f64::min);
        let ceiling = self.top.iter().map(|point| point.1).fold(f64::NEG_INFINITY, f64::max);
        let left = self.left.rightmost();
        let right = self.right.leftmost();
        CarveBox::new(left + margin, ceiling + margin, right - margin, floor - margin)
    }

    #[must_use]
    pub fn centre(&self) -> (f64, f64) {
        let face = self.face(0.0);
        ((face.left + face.right) * 0.5, (face.top + face.bottom) * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::WallStone;
    use crate::WallJoint;

    fn stone() -> WallStone {
        WallStone {
            block: 0,
            course: 0,
            bottom: vec![(0.0, 100.0), (50.0, 104.0), (100.0, 100.0)],
            top: vec![(10.0, 0.0), (60.0, -4.0), (90.0, 0.0)],
            left: WallJoint::straight((0.0, 100.0), (10.0, 0.0)),
            right: WallJoint::new((100.0, 100.0), (104.0, 50.0), (90.0, 0.0)),
        }
    }

    #[test]
    fn the_outline_runs_along_the_bottom_and_back_along_the_top() {
        let outline = stone().outline();

        assert_eq!(outline.points().len(), 8);
        assert!(outline.contains(50.0, 50.0) && outline.contains(101.0, 50.0));
        assert!(!outline.contains(2.0, 5.0));
        assert!((outline.top() + 4.0).abs() < 1e-9 && (outline.bottom() - 104.0).abs() < 1e-9);
    }

    #[test]
    fn the_face_is_the_square_area_inside_every_edge() {
        let face = stone().face(5.0);

        assert!((face.left - 15.0).abs() < 1e-9 && (face.right - 85.0).abs() < 1e-9, "{face:?}");
        assert!((face.top - 5.0).abs() < 1e-9 && (face.bottom - 95.0).abs() < 1e-9);
        assert_eq!(stone().centre(), (50.0, 50.0));
    }

    #[test]
    fn a_shifted_stone_moves_every_corner_and_joint() {
        let moved = stone().shifted(10.0, -20.0);

        assert_eq!(moved.centre(), (60.0, 30.0));
        assert_eq!(moved.left.bottom, (10.0, 80.0));
        assert_eq!(moved.top[1], (70.0, -24.0));
    }
}
