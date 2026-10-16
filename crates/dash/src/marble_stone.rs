use crate::{BandFlow, StoneGround, StoneRecipe, StoneRgb, ValueNoise, VeinBranches, VeinInlay, VeinPath};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarbleStone {
    recipe: StoneRecipe,
    noise: ValueNoise,
    ground: StoneGround,
    inlay: VeinInlay,
}

impl MarbleStone {
    const BRANCH_STRENGTH: f64 = 0.85;
    const BRANCH_FADE_SCALE: f64 = 45.0;

    #[must_use]
    pub const fn new(recipe: StoneRecipe, seed: u64) -> Self {
        let noise = ValueNoise::new(seed);
        Self {
            recipe,
            noise,
            ground: StoneGround::new(recipe, noise),
            inlay: VeinInlay::new(recipe, noise),
        }
    }

    #[must_use]
    pub const fn recipe(&self) -> StoneRecipe {
        self.recipe
    }

    #[must_use]
    pub fn render(&self, path: &VeinPath, flow: &BandFlow, branches: &VeinBranches, pixel_width: usize, pixel_height: usize, scale: f64) -> Vec<u8> {
        let mask = branches.mask(pixel_width, pixel_height, scale);
        let branch_colour = StoneRgb::from(self.recipe.branch_colour);
        let mut pixels = Vec::with_capacity(pixel_width * pixel_height * 4);
        for row in 0..pixel_height {
            let y = (row as f64 + 0.5) / scale;
            for column in 0..pixel_width {
                let x = (column as f64 + 0.5) / scale;
                let stone = self.ground.colour(x, y, flow.offset(x, y));
                let ground = stone.mix(branch_colour, self.branch_strength(x, y) * mask.at(column, row));
                let pixel = path.nearest(x, y).map_or(ground, |point| self.inlay.over(ground, point));
                pixels.extend(pixel.rgba());
            }
        }
        pixels
    }

    fn branch_strength(&self, x: f64, y: f64) -> f64 {
        let fade = self.noise.fractal(x / Self::BRANCH_FADE_SCALE - 520.0, y / Self::BRANCH_FADE_SCALE + 260.0, 2);
        Self::BRANCH_STRENGTH * (0.65 + 0.35 * fade)
    }
}

#[cfg(test)]
mod tests {
    use super::MarbleStone;
    use crate::{BandFlow, HexColor, StoneRecipe, VeinBranches, VeinPath};

    struct StoneFixture;

    impl StoneFixture {
        const BANDS: [HexColor; 2] = [HexColor::rgb([110, 80, 50]), HexColor::rgb([80, 55, 30])];

        fn recipe(branch_count: usize) -> StoneRecipe {
            StoneRecipe {
                base: HexColor::rgb([100, 70, 40]),
                bands: &Self::BANDS,
                band_spacing: 8.0,
                band_softness: 0.4,
                clouding: 0.1,
                cloudiness: 0.4,
                turbulence: 4.0,
                branch_colour: HexColor::rgb([240, 230, 215]),
                branch_count,
                branch_width: 1.2,
                vein_core: HexColor::rgb([250, 245, 235]),
                vein_rim: HexColor::rgb([30, 15, 5]),
                vein_width: 3.0,
                rim_strength: 0.8,
            }
        }

        fn render_with(points: &[(f64, f64)], branch_count: usize, width: usize, height: usize, scale: f64) -> Vec<u8> {
            let logical = width as f64 / scale;
            let recipe = Self::recipe(branch_count);
            let path = VeinPath::new(points).unwrap();
            let flow = BandFlow::new(points, logical).unwrap();
            let branches = VeinBranches::grow(points, recipe, 5);
            MarbleStone::new(recipe, 5).render(&path, &flow, &branches, width, height, scale)
        }

        fn render(points: &[(f64, f64)], width: usize, height: usize, scale: f64) -> Vec<u8> {
            Self::render_with(points, 0, width, height, scale)
        }

        fn red(pixels: &[u8], width: usize, column: usize, row: usize) -> u8 {
            pixels[(row * width + column) * 4]
        }

        fn brightest_row(pixels: &[u8], width: usize, height: usize, column: usize) -> f64 {
            let peak = (0..height).map(|row| Self::red(pixels, width, column, row)).max().unwrap();
            let rows: Vec<f64> = (0..height).filter(|row| Self::red(pixels, width, column, *row) + 10 >= peak).map(|row| row as f64 + 0.5).collect();
            rows.iter().sum::<f64>() / rows.len() as f64
        }
    }

    #[test]
    fn the_render_fills_every_pixel_opaquely() {
        let pixels = StoneFixture::render(&[(0.0, 30.0), (100.0, 30.0)], 100, 60, 1.0);

        assert_eq!(pixels.len(), 100 * 60 * 4);
        assert!(pixels.chunks(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn the_brightest_stone_in_each_column_is_the_price_line() {
        let (width, height) = (160, 100);
        let pixels = StoneFixture::render(&[(0.0, 20.0), (80.0, 80.0), (160.0, 35.0)], width, height, 1.0);

        for (column, expected) in [(2_usize, 21.0), (80, 79.0), (157, 36.0)] {
            let row = StoneFixture::brightest_row(&pixels, width, height, column);
            assert!((row - expected).abs() <= 3.0, "column {column}: row {row}, expected {expected}");
        }
    }

    #[test]
    fn branches_lighten_stone_away_from_the_price_line() {
        let points: Vec<(f64, f64)> = (0..=20).map(|step| (f64::from(step) * 15.0, 100.0)).collect();
        let (width, height) = (300, 200);
        let bare = StoneFixture::render_with(&points, 0, width, height, 1.0);
        let branched = StoneFixture::render_with(&points, 6, width, height, 1.0);
        let lightened = (0..height)
            .filter(|row| row.abs_diff(100) > 12)
            .flat_map(|row| (0..width).map(move |column| (column, row)))
            .filter(|(column, row)| StoneFixture::red(&branched, width, *column, *row) > StoneFixture::red(&bare, width, *column, *row) + 40)
            .count();

        assert!(lightened > 100, "{lightened}");
    }

    #[test]
    fn a_denser_screen_draws_the_vein_at_the_same_place() {
        let pixels = StoneFixture::render(&[(0.0, 50.0), (100.0, 50.0)], 200, 160, 2.0);

        assert!((StoneFixture::brightest_row(&pixels, 200, 160, 100) - 100.0).abs() <= 3.0);
    }

    #[test]
    fn the_same_stone_renders_the_same_pixels() {
        let points = [(0.0, 40.0), (60.0, 10.0), (120.0, 50.0)];

        assert_eq!(StoneFixture::render_with(&points, 4, 120, 60, 1.0), StoneFixture::render_with(&points, 4, 120, 60, 1.0));
    }
}
