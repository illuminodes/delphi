use crate::ValueNoise;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoneMottle {
    tint: [f64; 3],
    cloud: ValueNoise,
    stain: ValueNoise,
}

impl StoneMottle {
    const CLOUD_SCALE: f64 = 70.0;
    const CLOUD: f64 = 0.13;
    const STAIN_SCALE: f64 = 45.0;
    const STAIN_FROM: f64 = 0.25;
    const STAIN: [f64; 3] = [1.05, 0.9, 0.74];
    const STAIN_MOST: f64 = 0.55;

    #[must_use]
    pub const fn new(tint: [f64; 3], seed: u64) -> Self {
        Self { tint, cloud: ValueNoise::new(seed ^ 0xC10D), stain: ValueNoise::new(seed ^ 0x57A1) }
    }

    #[must_use]
    pub fn at(&self, x: f64, y: f64) -> [f64; 3] {
        let cloud = 1.0 + self.cloud.fractal(x / Self::CLOUD_SCALE, y / Self::CLOUD_SCALE, 3) * Self::CLOUD;
        let stain = ((self.stain.fractal(x / Self::STAIN_SCALE, y / Self::STAIN_SCALE, 3) - Self::STAIN_FROM) / (1.0 - Self::STAIN_FROM)).clamp(0.0, 1.0) * Self::STAIN_MOST;
        [0, 1, 2].map(|channel| self.tint[channel] * cloud * (1.0 + (Self::STAIN[channel] - 1.0) * stain))
    }
}

#[cfg(test)]
mod tests {
    use super::StoneMottle;

    fn spread(mottle: &StoneMottle, channel: usize) -> f64 {
        let values: Vec<f64> = (0..400).map(|step| mottle.at(f64::from(step % 20) * 15.0, f64::from(step / 20) * 15.0)[channel]).collect();
        let (low, high) = values.iter().fold((f64::INFINITY, 0.0_f64), |(low, high), value| (low.min(*value), high.max(*value)));
        high - low
    }

    #[test]
    fn the_face_brightens_and_darkens_in_broad_patches() {
        let mottle = StoneMottle::new([1.0; 3], 4);

        assert!(spread(&mottle, 1) > 0.12, "{}", spread(&mottle, 1));
        assert!((mottle.at(10.0, 10.0)[1] - mottle.at(11.0, 10.0)[1]).abs() < 0.01);
    }

    #[test]
    fn stains_warm_the_stone_more_than_they_change_its_brightness() {
        let mottle = StoneMottle::new([1.0; 3], 4);
        let warmest = (0..400).map(|step| mottle.at(f64::from(step % 20) * 15.0, f64::from(step / 20) * 15.0)).map(|colour| colour[0] - colour[2]).fold(0.0, f64::max);

        assert!(warmest > 0.1, "{warmest}");
    }

    #[test]
    fn the_tint_colours_the_whole_stone() {
        let warm = StoneMottle::new([1.1, 1.0, 0.9], 4).at(5.0, 5.0);
        let plain = StoneMottle::new([1.0; 3], 4).at(5.0, 5.0);

        assert!((warm[0] / plain[0] - 1.1).abs() < 1e-9 && (warm[2] / plain[2] - 0.9).abs() < 1e-9);
    }
}
