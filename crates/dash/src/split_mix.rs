#[derive(Debug, Clone)]
pub struct SplitMix {
    state: u64,
}

impl SplitMix {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    pub fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 1.0) / (1_u64 << 53) as f64
    }

    pub fn gaussian(&mut self) -> f64 {
        let (radius, angle) = (self.unit(), self.unit());
        (-2.0 * radius.ln()).sqrt() * (std::f64::consts::TAU * angle).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix;

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let mut one = SplitMix::new(42);
        let mut two = SplitMix::new(42);

        assert!((0..100).all(|_| one.next_u64() == two.next_u64()));
    }

    #[test]
    fn units_stay_inside_the_open_low_closed_high_interval() {
        let mut random = SplitMix::new(1);

        assert!((0..10_000).map(|_| random.unit()).all(|unit| unit > 0.0 && unit <= 1.0));
    }

    #[test]
    fn gaussians_are_finite_and_centred() {
        let mut random = SplitMix::new(9);
        let draws: Vec<f64> = (0..20_000).map(|_| random.gaussian()).collect();
        let mean = draws.iter().sum::<f64>() / draws.len() as f64;

        assert!(draws.iter().all(|draw| draw.is_finite()));
        assert!(mean.abs() < 0.05, "mean {mean}");
    }
}
