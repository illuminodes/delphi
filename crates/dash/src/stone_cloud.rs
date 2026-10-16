use crate::{StoneRecipe, StoneRgb, ValueNoise};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoneCloud {
    recipe: StoneRecipe,
    noise: ValueNoise,
}

impl StoneCloud {
    const SCALE: f64 = 140.0;
    const WARP: f64 = 3.5;
    const SPREAD: f64 = 1.5;
    const HUG: f64 = 28.0;
    const PATCH_SCALE: f64 = 210.0;

    #[must_use]
    pub const fn new(recipe: StoneRecipe, noise: ValueNoise) -> Self {
        Self { recipe, noise }
    }

    #[must_use]
    pub fn over(&self, banded: StoneRgb, x: f64, y: f64, across: f64) -> StoneRgb {
        let weight = self.weight(x, y, across);
        if weight <= 0.0 {
            return banded;
        }
        banded.mix(self.colour(x, y), weight)
    }

    #[must_use]
    pub fn weight(&self, x: f64, y: f64, across: f64) -> f64 {
        if self.recipe.cloudiness <= 0.0 || self.recipe.bands.is_empty() {
            return 0.0;
        }
        let calm = 1.0 - (-(across / Self::HUG).powi(2)).exp();
        let patch = 0.5 + 0.5 * self.noise.fractal(x / Self::PATCH_SCALE + 300.0, y / Self::PATCH_SCALE - 150.0, 3);
        (self.recipe.cloudiness * calm * (0.65 + 0.35 * patch)).clamp(0.0, 1.0)
    }

    fn colour(&self, x: f64, y: f64) -> StoneRgb {
        let bands = self.recipe.bands;
        let (u, v) = (x / Self::SCALE, y / Self::SCALE);
        let warp_x = self.noise.fractal(u + 7.1, v - 3.3, 4);
        let warp_y = self.noise.fractal(u - 5.2, v + 9.4, 4);
        let value = self.noise.fractal(u + Self::WARP * warp_x, v + Self::WARP * warp_y, 5);
        let (darkest, lightest) = bands.iter().map(|band| StoneRgb::from(*band)).fold((StoneRgb::from(bands[0]), StoneRgb::from(bands[0])), |(dark, light), band| {
            (if band.lightness() < dark.lightness() { band } else { dark }, if band.lightness() > light.lightness() { band } else { light })
        });
        let middle = darkest.mix(lightest, 0.5);
        let t = (value * Self::SPREAD).clamp(-1.0, 1.0);
        if t < 0.0 { middle.mix(darkest, -t) } else { middle.mix(lightest, t) }
    }
}

#[cfg(test)]
mod tests {
    use super::StoneCloud;
    use crate::{HexColor, StoneRecipe, StoneRgb, ValueNoise};

    struct CloudFixture;

    impl CloudFixture {
        const BANDS: [HexColor; 2] = [HexColor::rgb([220, 200, 160]), HexColor::rgb([90, 60, 30])];

        fn recipe(cloudiness: f64) -> StoneRecipe {
            StoneRecipe {
                base: Self::BANDS[0],
                bands: &Self::BANDS,
                band_spacing: 10.0,
                band_softness: 1.0,
                clouding: 0.0,
                cloudiness,
                turbulence: 0.0,
                branch_colour: HexColor::rgb([255, 255, 255]),
                branch_count: 0,
                branch_width: 1.0,
                vein_core: HexColor::rgb([250, 245, 235]),
                vein_rim: HexColor::rgb([40, 20, 10]),
                vein_width: 4.0,
                rim_strength: 0.8,
            }
        }

        fn cloud(cloudiness: f64) -> StoneCloud {
            StoneCloud::new(Self::recipe(cloudiness), ValueNoise::new(3))
        }
    }

    #[test]
    fn beside_the_line_the_bands_stay_clear() {
        let cloud = CloudFixture::cloud(0.9);

        assert!((0..100).all(|x| cloud.weight(f64::from(x) * 9.0, 50.0, 0.0) < 1e-9));
        assert!((0..100).all(|x| cloud.weight(f64::from(x) * 9.0, 50.0, 4.0) < 0.05));
    }

    #[test]
    fn far_from_the_line_the_cloud_covers_much_of_the_stone() {
        let cloud = CloudFixture::cloud(0.9);
        let weights: Vec<f64> = (0..400).map(|x| cloud.weight(f64::from(x) * 3.0, 200.0, 150.0)).collect();
        let mean = weights.iter().sum::<f64>() / weights.len() as f64;

        assert!(mean > 0.4, "{mean}");
    }

    #[test]
    fn the_cloud_varies_across_the_stone() {
        let cloud = CloudFixture::cloud(1.0);
        let banded = StoneRgb::new([0.0; 3]);
        let lightness: Vec<f64> = (0..600)
            .map(|step| cloud.over(banded, f64::from(step) * 2.0, f64::from(step % 30) * 10.0, 200.0).lightness())
            .collect();
        let (low, high) = lightness.iter().fold((f64::MAX, f64::MIN), |(low, high), value| (low.min(*value), high.max(*value)));

        assert!(high - low > 0.25, "{low} {high}");
    }

    #[test]
    fn a_stone_without_cloudiness_keeps_its_bands() {
        let banded = StoneRgb::new([10.0, 20.0, 30.0]);

        assert_eq!(CloudFixture::cloud(0.0).over(banded, 40.0, 40.0, 300.0), banded);
    }
}
