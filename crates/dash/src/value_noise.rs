#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueNoise {
    seed: u64,
}

impl ValueNoise {
    const OCTAVE_SHIFT: f64 = 17.13;

    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    #[must_use]
    pub fn sample(&self, x: f64, y: f64) -> f64 {
        let (left, top) = (x.floor(), y.floor());
        let (across, down) = (Self::fade(x - left), Self::fade(y - top));
        let (column, row) = (left as i64, top as i64);
        let upper = Self::mix(self.lattice(column, row), self.lattice(column + 1, row), across);
        let lower = Self::mix(self.lattice(column, row + 1), self.lattice(column + 1, row + 1), across);
        Self::mix(upper, lower, down)
    }

    #[must_use]
    pub fn fractal(&self, x: f64, y: f64, octaves: u32) -> f64 {
        let (mut total, mut weight, mut amplitude, mut frequency) = (0.0, 0.0, 1.0, 1.0);
        for octave in 0..octaves {
            let shift = f64::from(octave) * Self::OCTAVE_SHIFT;
            total += amplitude * self.sample(x * frequency + shift, y * frequency - shift);
            weight += amplitude;
            amplitude *= 0.5;
            frequency *= 2.0;
        }
        if weight > 0.0 { total / weight } else { 0.0 }
    }

    fn lattice(&self, column: i64, row: i64) -> f64 {
        let key = self.seed
            ^ (column as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (row as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        let mut mixed = key;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        (mixed >> 11) as f64 / (1_u64 << 52) as f64 - 1.0
    }

    fn fade(t: f64) -> f64 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn mix(from: f64, to: f64, amount: f64) -> f64 {
        from + (to - from) * amount
    }
}

#[cfg(test)]
mod tests {
    use super::ValueNoise;

    #[test]
    fn the_same_seed_gives_the_same_noise_and_another_seed_differs() {
        let (one, two, other) = (ValueNoise::new(3), ValueNoise::new(3), ValueNoise::new(4));

        assert!((one.fractal(1.3, 7.9, 4) - two.fractal(1.3, 7.9, 4)).abs() < f64::EPSILON);
        assert!((0..20).any(|step| (one.sample(f64::from(step) * 0.7, 0.3) - other.sample(f64::from(step) * 0.7, 0.3)).abs() > 0.1));
    }

    #[test]
    fn noise_stays_between_minus_one_and_one() {
        let noise = ValueNoise::new(11);
        let draws = (0..4_000).map(|step| f64::from(step) * 0.173).flat_map(|x| [noise.sample(x, -x * 0.5), noise.fractal(x, x * 0.3, 5)]);

        assert!(draws.into_iter().all(|value| (-1.0..=1.0).contains(&value)));
    }

    #[test]
    fn noise_changes_smoothly() {
        let noise = ValueNoise::new(5);
        let steepest = (0..2_000)
            .map(|step| f64::from(step) * 0.01 - 7.0)
            .map(|x| (noise.fractal(x + 0.01, 2.5, 4) - noise.fractal(x, 2.5, 4)).abs())
            .fold(0.0, f64::max);

        assert!(steepest < 0.1, "{steepest}");
    }

    #[test]
    fn noise_is_not_flat() {
        let noise = ValueNoise::new(8);
        let values: Vec<f64> = (0..200).map(|step| noise.fractal(f64::from(step) * 0.37, 1.1, 3)).collect();
        let (low, high) = values.iter().fold((1.0_f64, -1.0_f64), |(low, high), value| (low.min(*value), high.max(*value)));

        assert!(high - low > 0.8, "{low} {high}");
    }
}
