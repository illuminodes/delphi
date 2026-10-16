use crate::SplitMix;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CourseLine {
    pub index: usize,
    pub course_height: f64,
    pub seed: u64,
}

impl CourseLine {
    const KNOT_SPACING: f64 = 1.3;
    const KNOT_JITTER: f64 = 0.4;
    pub const KINK: f64 = 0.12;

    #[must_use]
    pub const fn new(index: usize, course_height: f64, seed: u64) -> Self {
        Self { index, course_height, seed }
    }

    #[must_use]
    pub fn level(&self) -> f64 {
        -(self.index as f64) * self.course_height
    }

    #[must_use]
    pub fn y(&self, x: f64) -> f64 {
        let knot = self.segment(x);
        let ((left_x, left_y), (right_x, right_y)) = (self.knot(knot), self.knot(knot + 1));
        let along = ((x - left_x) / (right_x - left_x).max(f64::EPSILON)).clamp(0.0, 1.0);
        left_y + (right_y - left_y) * along
    }

    #[must_use]
    pub fn between(&self, from: f64, to: f64) -> Vec<(f64, f64)> {
        let (low, high) = (from.min(to), from.max(to));
        let mut points = vec![(low, self.y(low))];
        let mut knot = self.segment(low) + 1;
        while self.knot(knot).0 < high {
            points.push(self.knot(knot));
            knot += 1;
        }
        points.push((high, self.y(high)));
        if from > to {
            points.reverse();
        }
        points
    }

    fn segment(&self, x: f64) -> i64 {
        let mut knot = (x / self.spacing()).floor() as i64;
        while self.knot(knot).0 > x {
            knot -= 1;
        }
        while self.knot(knot + 1).0 <= x {
            knot += 1;
        }
        knot
    }

    fn knot(&self, knot: i64) -> (f64, f64) {
        let mut random = SplitMix::new(self.seed ^ (self.index as u64).wrapping_mul(0x9E37_79B9) ^ (knot as u64).wrapping_mul(0xC2B2_AE35));
        let jitter = (random.unit() * 2.0 - 1.0) * Self::KNOT_JITTER;
        let kink = (random.unit() * 2.0 - 1.0) * Self::KINK * self.course_height;
        ((knot as f64 + jitter) * self.spacing(), self.level() + kink)
    }

    fn spacing(&self) -> f64 {
        self.course_height * Self::KNOT_SPACING
    }
}

#[cfg(test)]
mod tests {
    use super::CourseLine;

    #[test]
    fn every_line_kinks_a_little_around_its_course() {
        let third = CourseLine::new(3, 100.0, 3);

        assert!((0..50).all(|step| (third.y(f64::from(step) * 13.0) + 300.0).abs() <= 12.0 + 1e-9));
        assert!((0..50).any(|step| (third.y(f64::from(step) * 13.0) + 300.0).abs() > 2.0));
    }

    #[test]
    fn the_line_is_continuous_and_the_same_every_time() {
        let line = CourseLine::new(2, 80.0, 9);

        assert!((0..2000).all(|step| (line.y(f64::from(step) * 0.5) - line.y(f64::from(step) * 0.5 + 0.01)).abs() < 0.1));
        assert!((line.y(123.4) - CourseLine::new(2, 80.0, 9).y(123.4)).abs() < 1e-12);
    }

    #[test]
    fn the_knots_are_spaced_unevenly() {
        let line = CourseLine::new(1, 100.0, 4);
        let knots: Vec<f64> = line.between(0.0, 2000.0).iter().map(|point| point.0).collect();
        let gaps: Vec<f64> = knots[1..knots.len() - 1].windows(2).map(|pair| pair[1] - pair[0]).collect();
        let (short, long) = gaps.iter().fold((f64::INFINITY, 0.0_f64), |(low, high), gap| (low.min(*gap), high.max(*gap)));

        assert!(long - short > 30.0, "{short} {long}");
    }

    #[test]
    fn a_stretch_runs_from_end_to_end_through_every_knot_in_between() {
        let line = CourseLine::new(1, 100.0, 4);
        let forward = line.between(10.0, 600.0);
        let backward = line.between(600.0, 10.0);

        assert_eq!(forward.first().map(|point| point.0), Some(10.0));
        assert_eq!(forward.last().map(|point| point.0), Some(600.0));
        assert!(forward.len() >= 5);
        assert!(forward.windows(2).all(|pair| pair[1].0 > pair[0].0));
        assert!(forward.iter().all(|(x, y)| (line.y(*x) - y).abs() < 1e-9));
        assert_eq!(backward.first(), forward.last());
    }
}
