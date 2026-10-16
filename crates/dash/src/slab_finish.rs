use crate::{CarveMask, CarveRelief, EtchRecipe, StoneRgb};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlabFinish {
    recipe: EtchRecipe,
}

impl SlabFinish {
    const POLISHED: f64 = 0.14;
    const TOOLED: f64 = 0.7;
    const GILDED: f64 = 0.3;
    const TOOL_DEPTH: f64 = 2.0;

    #[must_use]
    pub const fn new(recipe: EtchRecipe) -> Self {
        Self { recipe }
    }

    #[must_use]
    fn tooled(relief: &CarveRelief, column: usize, row: usize) -> f64 {
        (relief.at(column, row) / Self::TOOL_DEPTH).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn albedo(&self, photo: &[u8], mask: &CarveMask, relief: &CarveRelief) -> Vec<u8> {
        let (frost, gold) = (StoneRgb::from(self.recipe.frost), StoneRgb::from(self.recipe.gilt));
        photo
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .flat_map(|(index, pixel)| {
                let (column, row) = (index % mask.width(), index / mask.width());
                let cut = Self::tooled(relief, column, row);
                let gilt = mask.gilt_at(column, row).min(1.0) * cut;
                let stone = StoneRgb::new([f64::from(pixel[0]), f64::from(pixel[1]), f64::from(pixel[2])]);
                let tooled = stone.dulled(self.recipe.dullness).lit(self.recipe.depth_tone).frosted(frost);
                stone.mix(tooled, cut).mix(gold, gilt * self.recipe.gilt_amount).rgba()
            })
            .collect()
    }

    #[must_use]
    pub fn surface(&self, mask: &CarveMask, relief: &CarveRelief) -> Vec<u8> {
        (0..mask.width() * mask.height())
            .flat_map(|index| {
                let (column, row) = (index % mask.width(), index / mask.width());
                let cut = Self::tooled(relief, column, row);
                let gilt = mask.gilt_at(column, row).min(1.0) * cut * self.recipe.gilt_amount;
                let roughness = Self::POLISHED + (Self::TOOLED - Self::POLISHED) * cut;
                let roughness = roughness + (Self::GILDED - roughness) * gilt;
                [1.0 - cut, roughness, gilt].map(|level| (level * 255.0).round().clamp(0.0, 255.0) as u8).into_iter().chain([u8::MAX])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::SlabFinish;
    use crate::{CarveCut, CarveFill, CarveGroove, CarveMask, CarveRelief, MarbleKind};

    struct FinishFixture;

    impl FinishFixture {
        const SIDE: usize = 40;
        const STONE: usize = 5 * 40 + 5;
        const PLAIN: usize = 10 * 40 + 20;
        const GILT: usize = 30 * 40 + 20;

        fn mask() -> CarveMask {
            let mut mask = CarveMask::new(Self::SIDE, Self::SIDE, 1.0);
            for row in 6..15 {
                for column in 10..30 {
                    mask.press(column, row, 1.0, 1.0, CarveFill::Plain, CarveCut::Relief);
                }
            }
            for row in 26..35 {
                for column in 10..30 {
                    mask.press(column, row, 1.0, 1.0, CarveFill::Gilt, CarveCut::Relief);
                }
            }
            mask
        }

        fn relief(mask: &CarveMask) -> CarveRelief {
            CarveRelief::new(mask, 1.8, 14.0)
        }

        fn photo(tone: [u8; 4]) -> Vec<u8> {
            tone.repeat(Self::SIDE * Self::SIDE)
        }

        fn maps(finish: SlabFinish, tone: [u8; 4]) -> (Vec<u8>, Vec<u8>) {
            let mask = Self::mask();
            let relief = Self::relief(&mask);
            (finish.albedo(&Self::photo(tone), &mask, &relief), finish.surface(&mask, &relief))
        }

        fn pixel(pixels: &[u8], index: usize) -> [u8; 4] {
            [pixels[index * 4], pixels[index * 4 + 1], pixels[index * 4 + 2], pixels[index * 4 + 3]]
        }
    }

    #[test]
    fn uncut_stone_keeps_its_photo_and_its_polish() {
        let (albedo, surface) = FinishFixture::maps(SlabFinish::new(MarbleKind::Nero.etch()), [30, 30, 32, 255]);

        assert_eq!(FinishFixture::pixel(&albedo, FinishFixture::STONE), [30, 30, 32, 255]);
        let [clearcoat, roughness, metal, _] = FinishFixture::pixel(&surface, FinishFixture::STONE);
        assert_eq!(clearcoat, 255);
        assert!(roughness < 50, "{roughness}");
        assert_eq!(metal, 0);
    }

    #[test]
    fn a_cut_loses_its_polish_and_shows_bruised_lighter_stone_on_black_marble() {
        let (albedo, surface) = FinishFixture::maps(SlabFinish::new(MarbleKind::Nero.etch()), [30, 30, 32, 255]);
        let [red, ..] = FinishFixture::pixel(&albedo, FinishFixture::PLAIN);
        let [clearcoat, roughness, metal, _] = FinishFixture::pixel(&surface, FinishFixture::PLAIN);

        assert!(red > 40, "{red}");
        assert_eq!(clearcoat, 0);
        assert!(roughness > 150, "{roughness}");
        assert_eq!(metal, 0);
    }

    #[test]
    fn a_gilt_cut_is_gold_metal() {
        let (albedo, surface) = FinishFixture::maps(SlabFinish::new(MarbleKind::Parian.etch()), [240, 238, 235, 255]);
        let [red, _, blue, _] = FinishFixture::pixel(&albedo, FinishFixture::GILT);
        let [_, _, metal, _] = FinishFixture::pixel(&surface, FinishFixture::GILT);

        assert!(red > blue + 60, "{red} {blue}");
        assert_eq!(metal, 255);
    }

    #[test]
    fn the_lip_of_a_cut_keeps_its_polish_until_the_wall_has_sunk() {
        let mask = FinishFixture::mask();
        let relief = FinishFixture::relief(&mask);
        let surface = SlabFinish::new(MarbleKind::Nero.etch()).surface(&mask, &relief);
        let polish = |column: usize| FinishFixture::pixel(&surface, 10 * FinishFixture::SIDE + column)[0];

        assert!(mask.cut_at(10, 10) > 0.9);
        assert!(polish(10) > polish(20) + 60, "lip {} floor {} (depth {} vs {})", polish(10), polish(20), relief.at(10, 10), relief.at(20, 10));
    }

    #[test]
    fn a_gilt_line_is_never_painted_flat_on_the_surface() {
        let mask = CarveMask::new(40, 40, 1.0);
        let mut gilded = mask.clone();
        gilded.stamp_groove(&CarveGroove::line((20.0, 2.0), (20.0, 38.0), 1.0));
        let relief = CarveRelief::new(&gilded, 1.8, 14.0);
        let surface = SlabFinish::new(MarbleKind::Parian.etch()).surface(&gilded, &relief);
        let flat_metal = (0..40 * 40).filter(|&index| relief.depths()[index] < 0.05 && surface[index * 4 + 2] > 40).count();

        assert_eq!(flat_metal, 0);
    }
}
