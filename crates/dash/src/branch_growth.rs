use crate::{BranchNode, SplitMix, ValueNoise, VeinBranch};

#[derive(Debug, Clone)]
pub struct BranchGrowth {
    random: SplitMix,
    noise: ValueNoise,
    branches: Vec<VeinBranch>,
}

#[derive(Debug, Clone, Copy)]
struct Sprout {
    x: f64,
    y: f64,
    angle: f64,
    side: f64,
    heading: f64,
    length: f64,
    width: f64,
    depth: u32,
}

impl BranchGrowth {
    const STEP: f64 = 2.5;
    const MIN_ANGLE: f64 = 0.35;
    const MAX_ANGLE: f64 = 1.35;
    const BEND: f64 = 0.8;
    const WANDER_PACE: f64 = 0.05;
    const SWELL_PACE: f64 = 0.07;
    const JITTER: f64 = 2.5;
    const JITTER_PACE: f64 = 0.06;
    const FORK_CHANCE: f64 = 0.8;
    const FORK_SPREAD: f64 = 0.6;
    const MAX_DEPTH: u32 = 3;
    const TIP: f64 = 0.12;
    const SHORTEST: f64 = 60.0;
    const LONGEST: f64 = 260.0;

    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            random: SplitMix::new(seed),
            noise: ValueNoise::new(seed),
            branches: Vec::new(),
        }
    }

    pub fn sprout_from(&mut self, course: &[(f64, f64)], width: f64) {
        if course.len() < 2 {
            return;
        }
        let low = course.len() / 12;
        let high = course.len() - 1 - low;
        let index = low + (self.random.unit() * (high - low) as f64) as usize;
        let (x, y) = course[index.min(course.len() - 1)];
        let sprout = Sprout {
            x,
            y,
            angle: Self::MIN_ANGLE + self.random.unit() * 0.6,
            side: self.sign(),
            heading: self.sign(),
            length: Self::SHORTEST + self.random.unit() * (Self::LONGEST - Self::SHORTEST),
            width: width * (0.6 + 0.4 * self.random.unit()),
            depth: 0,
        };
        self.grow(sprout);
    }

    #[must_use]
    pub fn finish(self) -> Vec<VeinBranch> {
        self.branches
    }

    fn grow(&mut self, sprout: Sprout) {
        let steps = ((sprout.length / Self::STEP).ceil() as usize).max(2);
        let fork_step = (sprout.depth < Self::MAX_DEPTH && self.random.unit() < Self::FORK_CHANCE)
            .then(|| ((0.25 + 0.4 * self.random.unit()) * steps as f64) as usize);
        let wander = self.random.unit() * 1000.0;
        let (mut x, mut y) = (sprout.x, sprout.y);
        let mut angle = sprout.angle;
        let mut nodes = Vec::with_capacity(steps + 1);
        let mut fork = None;
        for step in 0..=steps {
            let remaining = 1.0 - step as f64 / steps as f64;
            let swell = 0.75 + 0.45 * self.noise.sample(wander * 0.5 + step as f64 * Self::SWELL_PACE, 77.0 + f64::from(sprout.depth));
            let half_width = (sprout.width * remaining.powf(0.8) * swell).max(Self::TIP);
            let jitter = Self::JITTER * self.noise.sample(wander - step as f64 * Self::JITTER_PACE, 41.0 + f64::from(sprout.depth));
            nodes.push(BranchNode { x: x - sprout.heading * angle.sin() * jitter * (1.0 - remaining).min(0.5) * 2.0, y: y + sprout.side * angle.cos() * jitter * (1.0 - remaining).min(0.5) * 2.0, half_width });
            if Some(step) == fork_step {
                fork = Some(Sprout {
                    x,
                    y,
                    angle: Self::soft_clamp(angle + self.sign() * Self::FORK_SPREAD),
                    length: sprout.length * remaining * (0.5 + 0.4 * self.random.unit()),
                    width: half_width * 0.7,
                    depth: sprout.depth + 1,
                    ..sprout
                });
            }
            let bend = self.noise.sample(wander + step as f64 * Self::WANDER_PACE, f64::from(sprout.depth) * 9.0);
            angle = Self::soft_clamp(sprout.angle + Self::BEND * bend);
            x += sprout.heading * angle.cos() * Self::STEP;
            y += sprout.side * angle.sin() * Self::STEP;
        }
        self.branches.push(VeinBranch::new(nodes));
        if let Some(child) = fork {
            self.grow(child);
        }
    }

    fn soft_clamp(angle: f64) -> f64 {
        let middle = (Self::MIN_ANGLE + Self::MAX_ANGLE) / 2.0;
        let half = (Self::MAX_ANGLE - Self::MIN_ANGLE) / 2.0;
        middle + half * ((angle - middle) / half).tanh()
    }

    fn sign(&mut self) -> f64 {
        if self.random.unit() < 0.5 { -1.0 } else { 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::BranchGrowth;

    struct GrowthFixture;

    impl GrowthFixture {
        fn flat() -> Vec<(f64, f64)> {
            (0..=100).map(|step| (f64::from(step) * 8.0, 200.0)).collect()
        }

        fn grown(seed: u64, count: usize) -> Vec<crate::VeinBranch> {
            let mut growth = BranchGrowth::new(seed);
            for _ in 0..count {
                growth.sprout_from(&Self::flat(), 1.2);
            }
            growth.finish()
        }
    }

    #[test]
    fn every_branch_leaves_the_line_on_one_side_and_stays_there() {
        for branch in GrowthFixture::grown(4, 12) {
            let nodes = branch.nodes();
            let side = (nodes[nodes.len() - 1].y - 200.0).signum();
            assert!(nodes.iter().all(|node| (node.y - 200.0) * side >= -1e-9));
        }
    }

    #[test]
    fn branches_start_on_the_line() {
        let roots: Vec<_> = GrowthFixture::grown(9, 8).into_iter().map(|branch| branch.nodes()[0]).collect();

        assert!(roots.iter().filter(|root| (root.y - 200.0).abs() < 1e-9).count() >= 8);
    }

    #[test]
    fn branches_end_thinner_than_they_start() {
        for branch in GrowthFixture::grown(2, 10) {
            let nodes = branch.nodes();
            let (start, end) = (nodes[0].half_width, nodes[nodes.len() - 1].half_width);
            assert!(end <= (start * 0.3).max(BranchGrowth::TIP) + 1e-9, "{start} {end}");
        }
    }

    #[test]
    fn branches_pinch_and_swell_along_their_length() {
        let swelling = GrowthFixture::grown(5, 10)
            .iter()
            .filter(|branch| branch.nodes().windows(2).any(|pair| pair[1].half_width > pair[0].half_width + 1e-6))
            .count();

        assert!(swelling > 0);
    }

    #[test]
    fn some_branches_fork() {
        assert!(GrowthFixture::grown(6, 10).len() > 10);
    }

    #[test]
    fn the_same_seed_grows_the_same_branches() {
        assert_eq!(GrowthFixture::grown(3, 5), GrowthFixture::grown(3, 5));
    }

    #[test]
    fn a_course_too_short_grows_nothing() {
        let mut growth = BranchGrowth::new(1);
        growth.sprout_from(&[(0.0, 0.0)], 1.0);

        assert!(growth.finish().is_empty());
    }
}
