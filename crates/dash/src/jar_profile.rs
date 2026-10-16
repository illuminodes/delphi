#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JarProfile {
    knots: &'static [(f64, f64)],
}

impl JarProfile {
    pub const FLASK: Self = Self {
        knots: &[
            (0.0, 0.26),
            (0.021, 0.298),
            (0.042, 0.342),
            (0.062, 0.368),
            (0.083, 0.5),
            (0.104, 0.599),
            (0.125, 0.675),
            (0.146, 0.74),
            (0.167, 0.795),
            (0.188, 0.839),
            (0.208, 0.88),
            (0.229, 0.912),
            (0.25, 0.936),
            (0.271, 0.959),
            (0.292, 0.977),
            (0.312, 0.988),
            (0.333, 0.997),
            (0.375, 1.0),
            (0.396, 0.994),
            (0.417, 0.985),
            (0.438, 0.971),
            (0.458, 0.95),
            (0.479, 0.927),
            (0.5, 0.901),
            (0.521, 0.868),
            (0.542, 0.83),
            (0.562, 0.798),
            (0.583, 0.775),
            (0.604, 0.632),
            (0.625, 0.456),
            (0.646, 0.427),
            (0.667, 0.327),
            (0.688, 0.243),
            (0.708, 0.208),
            (0.729, 0.213),
            (0.75, 0.222),
            (0.771, 0.2),
            (0.792, 0.175),
            (0.812, 0.149),
            (0.833, 0.102),
            (0.896, 0.105),
            (0.917, 0.129),
            (0.938, 0.175),
            (0.958, 0.31),
            (0.979, 0.3),
            (1.0, 0.24),
        ],
    };

    #[must_use]
    pub fn reach(&self, rise: f64) -> f64 {
        let knots = self.knots;
        let rise = rise.clamp(0.0, 1.0);
        let after = knots.partition_point(|knot| knot.0 < rise).clamp(1, knots.len() - 1);
        let (from, to) = (knots[after - 1], knots[after]);
        let span = to.0 - from.0;
        let t = if span > 0.0 { (rise - from.0) / span } else { 0.0 };
        let (leave, arrive) = (self.slope(after - 1) * span, self.slope(after) * span);
        let (square, cube) = (t * t, t * t * t);
        (2.0 * cube - 3.0 * square + 1.0) * from.1 + (cube - 2.0 * square + t) * leave + (3.0 * square - 2.0 * cube) * to.1 + (cube - square) * arrive
    }

    fn slope(&self, at: usize) -> f64 {
        let knots = self.knots;
        let (before, here, after) = (knots[at.saturating_sub(1)], knots[at], knots[(at + 1).min(knots.len() - 1)]);
        let (left, right) = (Self::rate(before, here), Self::rate(here, after));
        if left * right <= 0.0 { 0.0 } else { 2.0 / (1.0 / left + 1.0 / right) }
    }

    fn rate(from: (f64, f64), to: (f64, f64)) -> f64 {
        if to.0 > from.0 { (to.1 - from.1) / (to.0 - from.0) } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::JarProfile;

    const JAR: JarProfile = JarProfile::FLASK;

    fn sweep() -> Vec<f64> {
        (0..=1000).map(|step| JAR.reach(f64::from(step) / 1000.0)).collect()
    }

    #[test]
    fn the_profile_passes_through_its_knots() {
        assert!((JAR.reach(0.0) - 0.26).abs() < 1e-9);
        assert!((JAR.reach(0.375) - 1.0).abs() < 1e-9);
        assert!((JAR.reach(1.0) - 0.24).abs() < 1e-9);
    }

    #[test]
    fn the_profile_never_overshoots_its_knots() {
        let widest = sweep().into_iter().fold(0.0, f64::max);
        let narrowest = sweep().into_iter().fold(f64::INFINITY, f64::min);

        assert!(widest <= 1.0 + 1e-9, "{widest}");
        assert!(narrowest >= 0.102 - 1e-9, "{narrowest}");
    }

    #[test]
    fn the_profile_has_no_jumps() {
        assert!(sweep().windows(2).all(|pair| (pair[1] - pair[0]).abs() < 0.015));
    }

    #[test]
    fn a_round_belly_tapers_to_a_thin_neck_that_flares_at_the_lip() {
        let belly = JAR.reach(0.37);
        let neck = JAR.reach(0.86);

        assert!(belly > 0.98);
        assert!(neck < belly * 0.15);
        assert!(JAR.reach(0.97) > neck * 2.5);
        assert!(JAR.reach(0.02) < belly * 0.35);
    }
}
