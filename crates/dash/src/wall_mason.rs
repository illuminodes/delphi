use crate::{CourseLine, SplitMix, WallJoint, WallStone};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallMason {
    pub width: f64,
    pub course_height: f64,
    pub seed: u64,
}

impl WallMason {
    const LEAN: f64 = 0.22;
    const KNEE: f64 = 0.16;
    const KNEE_RISE: (f64, f64) = (0.3, 0.7);
    const SHORTEST: f64 = 0.55;

    #[must_use]
    pub const fn new(width: f64, course_height: f64, seed: u64) -> Self {
        Self { width, course_height, seed }
    }

    #[must_use]
    pub fn lay(&self, first_course: usize, spans: &[f64]) -> Vec<WallStone> {
        let shortest = self.course_height * Self::SHORTEST;
        let mut stones: Vec<WallStone> = Vec::with_capacity(spans.len());
        let (mut course, mut x) = (first_course, 0.0);
        for (block, span) in spans.iter().enumerate() {
            let span = span.max(shortest);
            if self.width - x < shortest {
                (course, x) = (course + 1, 0.0);
            }
            let mut end = (x + span).min(self.width);
            if self.width - end < shortest {
                end = self.width;
            }
            stones.push(self.stone(block, course, x, end));
            x = end;
        }
        stones
    }

    fn stone(&self, block: usize, course: usize, from: f64, to: f64) -> WallStone {
        let below = CourseLine::new(course, self.course_height, self.seed);
        let above = CourseLine::new(course + 1, self.course_height, self.seed);
        let left = self.joint(course, from, &below, &above);
        let right = self.joint(course, to, &below, &above);
        WallStone { block, course, bottom: below.between(left.bottom.0, right.bottom.0), top: above.between(left.top.0, right.top.0), left, right }
    }

    fn joint(&self, course: usize, x: f64, below: &CourseLine, above: &CourseLine) -> WallJoint {
        let edge = x <= 0.0 || x >= self.width;
        if edge {
            let x = x.clamp(0.0, self.width);
            return WallJoint::straight((x, below.y(x)), (x, above.y(x)));
        }
        let mut random = SplitMix::new(self.seed ^ (course as u64).wrapping_mul(0x51_7CC1) ^ x.to_bits());
        let lean = (random.unit() * 2.0 - 1.0) * Self::LEAN * self.course_height;
        let knee = (random.unit() * 2.0 - 1.0) * Self::KNEE * self.course_height;
        let rise = Self::KNEE_RISE.0 + (Self::KNEE_RISE.1 - Self::KNEE_RISE.0) * random.unit();
        let (bottom, top) = (x - lean * 0.5, x + lean * 0.5);
        let (low, high) = ((bottom, below.y(bottom)), (top, above.y(top)));
        let bend = (low.0 + (high.0 - low.0) * rise + knee, low.1 + (high.1 - low.1) * rise);
        WallJoint::new(low, bend, high)
    }

    #[must_use]
    pub fn courses(stones: &[WallStone]) -> usize {
        let first = stones.iter().map(|stone| stone.course).min().unwrap_or(0);
        stones.iter().map(|stone| stone.course + 1 - first).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::WallMason;
    use crate::WallStone;

    fn mason() -> WallMason {
        WallMason::new(1000.0, 100.0, 5)
    }

    #[test]
    fn stones_fill_each_course_from_wall_edge_to_wall_edge_before_starting_the_next() {
        let stones = mason().lay(0, &[300.0; 12]);
        let first: Vec<_> = stones.iter().filter(|stone| stone.course == 0).collect();

        assert!((first[0].left.bottom.0).abs() < 1e-9);
        assert!((first.last().expect("a stone").right.bottom.0 - 1000.0).abs() < 1e-9);
        assert!(stones.windows(2).all(|pair| pair[1].course >= pair[0].course));
        assert_eq!(first.len(), 4);
        assert_eq!(WallMason::courses(&stones), 3);
    }

    #[test]
    fn neighbours_share_the_joint_between_them() {
        let stones = mason().lay(0, &[180.0, 260.0, 140.0, 300.0, 220.0, 200.0]);

        for pair in stones.windows(2).filter(|pair| pair[0].course == pair[1].course) {
            assert_eq!(pair[0].right, pair[1].left);
        }
    }

    #[test]
    fn a_stone_sits_on_the_line_below_and_carries_the_line_above() {
        let stones = mason().lay(0, &[250.0; 9]);

        for stone in &stones {
            let floor = -(stone.course as f64) * 100.0;
            assert!(stone.bottom.iter().all(|point| (point.1 - floor).abs() <= 12.0 + 1e-9));
            assert!(stone.top.iter().all(|point| (point.1 - floor + 100.0).abs() <= 12.0 + 1e-9));
            assert!(stone.outline().contains(stone.centre().0, stone.centre().1));
        }
    }

    #[test]
    fn a_stone_too_small_is_widened_and_no_sliver_is_left_at_the_end_of_a_course() {
        let stones = mason().lay(0, &[10.0, 950.0, 400.0]);

        assert!(stones[0].right.bottom.0 - stones[0].left.bottom.0 >= 55.0 - 12.0);
        assert!((stones[1].right.bottom.0 - 1000.0).abs() < 1e-9);
        assert_eq!(stones[2].course, 1);
    }

    #[test]
    fn the_same_spans_lay_the_same_wall() {
        assert_eq!(mason().lay(0, &[200.0, 300.0, 150.0]), mason().lay(0, &[200.0, 300.0, 150.0]));
    }

    #[test]
    fn dropping_the_bottom_course_leaves_every_other_stone_exactly_where_it_was() {
        let spans = [300.0, 280.0, 260.0, 240.0, 310.0, 290.0, 270.0, 330.0, 200.0];
        let whole = mason().lay(0, &spans);
        let first_course = whole.iter().filter(|stone| stone.course == 0).count();
        let rest = mason().lay(1, &spans[first_course..]);

        assert_eq!(whole[first_course..].iter().map(|stone| WallStone { block: 0, ..stone.clone() }).collect::<Vec<_>>(), rest.iter().map(|stone| WallStone { block: 0, ..stone.clone() }).collect::<Vec<_>>());
        assert_eq!(WallMason::courses(&rest), WallMason::courses(&whole) - 1);
    }

    #[test]
    fn inner_joints_bend_at_a_knee_so_stones_have_more_than_four_sides() {
        let stones = mason().lay(0, &[300.0; 4]);
        let bent = stones.iter().filter(|stone| {
            let joint = stone.right;
            let straight = joint.bottom.0 + (joint.top.0 - joint.bottom.0) * (joint.knee.1 - joint.bottom.1) / (joint.top.1 - joint.bottom.1);
            (joint.knee.0 - straight).abs() > 2.0
        });

        assert!(bent.count() >= 2);
        assert!(stones.iter().all(|stone| stone.outline().points().len() >= 6));
    }
}
