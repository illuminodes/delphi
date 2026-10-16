use crate::{StoneCloud, StoneRecipe, StoneRgb, ValueNoise};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoneGround {
    recipe: StoneRecipe,
    noise: ValueNoise,
    cloud: StoneCloud,
}

impl StoneGround {
    const WARP_SCALE: f64 = 70.0;
    const DRIFT_SCALE: f64 = 260.0;
    const DRIFT_BANDS: f64 = 0.9;
    const UNEVEN: f64 = 0.32;
    const THICKNESS_SCALE: f64 = 120.0;
    const THICKNESS_SWING: f64 = 0.55;
    const BLEED_SCALE: f64 = 16.0;
    const BLEED_BANDS: f64 = 0.28;
    const CLOUD_SCALE: f64 = 95.0;

    #[must_use]
    pub const fn new(recipe: StoneRecipe, noise: ValueNoise) -> Self {
        Self { recipe, noise, cloud: StoneCloud::new(recipe, noise) }
    }

    #[must_use]
    pub fn colour(&self, x: f64, y: f64, across: f64) -> StoneRgb {
        let banded = self.cloud.over(self.band(x, y, across), x, y, across);
        let cloud = self.noise.fractal(x / Self::CLOUD_SCALE + 31.0, y / Self::CLOUD_SCALE - 7.0, 4);
        banded.lit(1.0 + self.recipe.clouding * cloud)
    }

    fn band(&self, x: f64, y: f64, across: f64) -> StoneRgb {
        let bands = self.recipe.bands;
        if bands.is_empty() {
            return StoneRgb::from(self.recipe.base);
        }
        let warp = self.recipe.turbulence * self.noise.fractal(x / Self::WARP_SCALE, y / Self::WARP_SCALE, 5);
        let drift = Self::DRIFT_BANDS * self.noise.fractal(x / Self::DRIFT_SCALE - 13.0, y / Self::DRIFT_SCALE + 5.0, 2);
        let thickness = 1.0 + Self::THICKNESS_SWING * self.noise.fractal(x / Self::THICKNESS_SCALE + 401.0, y / Self::THICKNESS_SCALE - 23.0, 3);
        let raw = (across + warp) / (self.recipe.band_spacing * thickness) + drift;
        let bleed = Self::BLEED_BANDS * self.noise.fractal(x / Self::BLEED_SCALE - 211.0, y / Self::BLEED_SCALE + 67.0, 3);
        let place = raw + Self::UNEVEN * (raw * 2.3).sin() + bleed;
        let count = bands.len() as f64;
        let cycle = place.rem_euclid(count);
        let index = cycle.floor() as usize % bands.len();
        let next = (index + 1) % bands.len();
        let blend = Self::smoothstep(0.5 - self.recipe.band_softness / 2.0, 0.5 + self.recipe.band_softness / 2.0, cycle.fract());
        StoneRgb::from(bands[index]).mix(StoneRgb::from(bands[next]), blend)
    }

    fn smoothstep(low: f64, high: f64, value: f64) -> f64 {
        if high <= low {
            return if value < low { 0.0 } else { 1.0 };
        }
        let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::StoneGround;
    use crate::{HexColor, StoneRecipe, StoneRgb, ValueNoise};

    struct GroundFixture;

    impl GroundFixture {
        const LIGHT: HexColor = HexColor::rgb([220, 200, 160]);
        const DARK: HexColor = HexColor::rgb([90, 60, 30]);
        const BANDS: [HexColor; 2] = [Self::LIGHT, Self::DARK];

        fn recipe() -> StoneRecipe {
            StoneRecipe {
                base: Self::LIGHT,
                bands: &Self::BANDS,
                band_spacing: 10.0,
                band_softness: 0.3,
                clouding: 0.0,
                cloudiness: 0.0,
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

        fn ground(recipe: StoneRecipe) -> StoneGround {
            StoneGround::new(recipe, ValueNoise::new(1))
        }
    }

    #[test]
    fn colour_changes_much_faster_across_the_bands_than_along_them() {
        let ground = GroundFixture::ground(GroundFixture::recipe());
        let restless = |sample: &dyn Fn(f64) -> StoneRgb| {
            (0..400).map(|step| (sample(f64::from(step) + 1.0).lightness() - sample(f64::from(step)).lightness()).abs()).sum::<f64>()
        };
        let along = restless(&|x| ground.colour(x, 40.0, 3.0));
        let across = restless(&|offset| ground.colour(20.0, 40.0, offset));

        assert!(across > 4.0 * along, "across {across} along {along}");
    }

    #[test]
    fn every_band_colour_shows_up() {
        let ground = GroundFixture::ground(GroundFixture::recipe());
        let pixels: Vec<[u8; 4]> = (0..200).map(|step| ground.colour(0.0, 0.0, f64::from(step) * 0.5).rgba()).collect();

        for band in GroundFixture::BANDS {
            let [red, green, blue] = band.channels();
            assert!(pixels.iter().any(|pixel| pixel[0].abs_diff(red) < 12 && pixel[1].abs_diff(green) < 12 && pixel[2].abs_diff(blue) < 12), "{band:?}");
        }
    }

    #[test]
    fn a_stone_without_bands_is_its_base_colour() {
        let recipe = StoneRecipe { bands: &[], ..GroundFixture::recipe() };

        assert_eq!(GroundFixture::ground(recipe).colour(12.0, 30.0, 5.0), StoneRgb::from(recipe.base));
    }
}
