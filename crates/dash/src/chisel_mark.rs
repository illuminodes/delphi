use crate::ValueNoise;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChiselMark {
    noise: ValueNoise,
    ragged: f64,
}

impl ChiselMark {
    const EDGE_SCALE: f64 = 1.6;
    const GRAIN_SCALE: f64 = 0.7;
    const FAINT: f64 = 0.02;

    #[must_use]
    pub const fn new(seed: u64, ragged: f64) -> Self {
        Self { noise: ValueNoise::new(seed), ragged }
    }

    #[must_use]
    pub fn bite(&self, cut: f64, x: f64, y: f64) -> f64 {
        if cut < Self::FAINT {
            return 0.0;
        }
        if cut >= 1.0 {
            return 1.0;
        }
        let wobble = self.noise.fractal(x / Self::EDGE_SCALE, y / Self::EDGE_SCALE, 2);
        (cut + self.ragged * wobble).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn grain(&self, x: f64, y: f64) -> f64 {
        self.noise.sample(x / Self::GRAIN_SCALE + 311.0, y / Self::GRAIN_SCALE - 97.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ChiselMark;

    #[test]
    fn untouched_stone_and_the_floor_of_a_cut_are_left_as_they_are() {
        let chisel = ChiselMark::new(4, 0.5);

        assert!((0..50).all(|step| chisel.bite(0.0, f64::from(step) * 0.37, 3.0) == 0.0));
        assert!((0..50).all(|step| (chisel.bite(1.0, f64::from(step) * 0.37, 3.0) - 1.0).abs() < 1e-12));
    }

    #[test]
    fn a_trace_of_coverage_left_by_the_rasteriser_does_not_become_a_cut() {
        let chisel = ChiselMark::new(4, 0.5);

        assert!((0..200).all(|step| chisel.bite(1.19e-7, f64::from(step) * 0.37, f64::from(step % 13)) == 0.0));
        assert!((0..200).all(|step| chisel.bite(1.0 / 255.0, f64::from(step) * 0.37, f64::from(step % 13)) == 0.0));
    }

    #[test]
    fn the_edge_of_a_cut_wanders_along_its_length() {
        let chisel = ChiselMark::new(4, 0.5);
        let edge: Vec<f64> = (0..80).map(|step| chisel.bite(0.5, f64::from(step) * 0.5, 7.0)).collect();
        let (low, high) = edge.iter().fold((1.0_f64, 0.0_f64), |(low, high), value| (low.min(*value), high.max(*value)));

        assert!(high - low > 0.2, "{low} {high}");
    }

    #[test]
    fn grain_stays_between_minus_one_and_one() {
        let chisel = ChiselMark::new(9, 0.3);

        assert!((0..400).all(|step| chisel.grain(f64::from(step) * 0.13, f64::from(step % 17)).abs() <= 1.0));
    }
}
