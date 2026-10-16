use crate::BranchNode;

#[derive(Debug, Clone, PartialEq)]
pub struct BranchMask {
    width: usize,
    height: usize,
    scale: f64,
    cover: Vec<f32>,
}

impl BranchMask {
    const SOFT_EDGE: f64 = 0.7;

    #[must_use]
    pub fn new(width: usize, height: usize, scale: f64) -> Self {
        Self { width, height, scale, cover: vec![0.0; width * height] }
    }

    #[must_use]
    pub fn at(&self, column: usize, row: usize) -> f64 {
        self.cover.get(row * self.width + column).map_or(0.0, |cover| f64::from(*cover))
    }

    pub fn stamp(&mut self, from: BranchNode, to: BranchNode) {
        let (ax, ay, bx, by) = (from.x * self.scale, from.y * self.scale, to.x * self.scale, to.y * self.scale);
        let (wa, wb) = (from.half_width * self.scale, to.half_width * self.scale);
        let reach = wa.max(wb) + Self::SOFT_EDGE + 1.0;
        let columns = Self::span(ax.min(bx) - reach, ax.max(bx) + reach, self.width);
        let rows = Self::span(ay.min(by) - reach, ay.max(by) + reach, self.height);
        let (dx, dy) = (bx - ax, by - ay);
        let length_squared = (dx * dx + dy * dy).max(f64::EPSILON);
        for row in rows {
            let py = row as f64 + 0.5;
            for column in columns.clone() {
                let px = column as f64 + 0.5;
                let t = (((px - ax) * dx + (py - ay) * dy) / length_squared).clamp(0.0, 1.0);
                let distance = (px - ax - dx * t).hypot(py - ay - dy * t);
                let half = wa + (wb - wa) * t;
                let cover = Self::smoothstep(half + Self::SOFT_EDGE, half - Self::SOFT_EDGE, distance) * half.min(1.0);
                let cell = &mut self.cover[row * self.width + column];
                *cell = cell.max(cover as f32);
            }
        }
    }

    fn span(low: f64, high: f64, limit: usize) -> std::ops::Range<usize> {
        let start = low.floor().max(0.0) as usize;
        let end = (high.ceil().max(0.0) as usize).min(limit);
        start.min(end)..end
    }

    fn smoothstep(low: f64, high: f64, value: f64) -> f64 {
        let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::BranchMask;
    use crate::BranchNode;

    struct MaskFixture;

    impl MaskFixture {
        fn node(x: f64, y: f64) -> BranchNode {
            BranchNode { x, y, half_width: 1.5 }
        }
    }

    #[test]
    fn a_stamped_segment_covers_its_centre_and_nothing_far_away() {
        let mut mask = BranchMask::new(60, 40, 1.0);
        mask.stamp(MaskFixture::node(10.0, 20.0), MaskFixture::node(50.0, 20.0));

        assert!(mask.at(30, 19) > 0.9 || mask.at(30, 20) > 0.9);
        assert!(mask.at(30, 30) < 1e-6);
        assert!(mask.at(5, 20) < 1e-6);
    }

    #[test]
    fn a_denser_screen_stamps_the_same_place() {
        let mut mask = BranchMask::new(120, 80, 2.0);
        mask.stamp(MaskFixture::node(10.0, 20.0), MaskFixture::node(50.0, 20.0));

        assert!(mask.at(60, 39) > 0.9 || mask.at(60, 40) > 0.9);
        assert!(mask.at(60, 50) < 1e-6);
    }

    #[test]
    fn segments_off_the_canvas_are_ignored() {
        let mut mask = BranchMask::new(20, 20, 1.0);
        mask.stamp(MaskFixture::node(-50.0, -50.0), MaskFixture::node(-30.0, -40.0));

        assert!((0..20).all(|row| (0..20).all(|column| mask.at(column, row) == 0.0)));
    }

    #[test]
    fn a_hair_thin_tip_is_faint() {
        let mut mask = BranchMask::new(40, 20, 1.0);
        let thin = |x| BranchNode { x, y: 10.0, half_width: 0.15 };
        mask.stamp(thin(5.0), thin(35.0));

        assert!((0..20).all(|row| mask.at(20, row) < 0.2));
    }
}
