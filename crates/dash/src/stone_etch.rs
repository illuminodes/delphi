use crate::{CarveMask, ChiselMark, CutFloor, CutLip, CutShadow, CutSlope, EtchRecipe, GrainTint, StoneRgb, VeeCut};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoneEtch {
    recipe: EtchRecipe,
    seed: u64,
}

impl StoneEtch {
    const UNTOUCHED: f64 = 1e-4;
    const WALL_GAIN: f64 = 2.5;
    const SHEEN: f64 = 0.3;
    const TINT_REACH: f64 = 40.0;
    const KNEE: f64 = 210.0;
    const LEAF_SCALE: f64 = 0.2;

    #[must_use]
    pub const fn new(recipe: EtchRecipe, seed: u64) -> Self {
        Self { recipe, seed }
    }

    pub fn apply(&self, pixels: &mut [u8], mask: &CarveMask) {
        let scale = mask.scale();
        let softness = (self.recipe.shadow_softness * scale).round().max(1.0) as usize;
        let offset = (self.recipe.shadow_offset * scale).round().max(1.0) as isize;
        let shadow = CutShadow::new(mask, softness, offset);
        let chisel = ChiselMark::new(self.seed, self.recipe.ragged);
        let slope = CutSlope::new(mask, scale.round().max(1.0) as usize);
        let vee = VeeCut::new(mask, self.recipe.vee_slope, self.recipe.vee_floor, self.recipe.ambient);
        let lip = CutLip::new(mask);
        let floor_stone = CutFloor::new(mask.width(), mask.height(), pixels, self.recipe.parallax * scale);
        let tint = GrainTint::new(mask.width(), mask.height(), pixels, (Self::TINT_REACH * scale).round().max(1.0) as usize, self.recipe.grain_keep);
        let (frost, light_tint, gilt) = (StoneRgb::from(self.recipe.frost), StoneRgb::from(self.recipe.light_tint), StoneRgb::from(self.recipe.gilt));
        let relief_frost = StoneRgb::from(self.recipe.relief_frost);
        for row in 0..mask.height() {
            for column in 0..mask.width() {
                let (x, y) = (column as f64 / scale, row as f64 / scale);
                let cut = chisel.bite(mask.cut_at(column, row), x, y);
                let rim = lip.at(mask, column, row) * self.recipe.rim;
                if cut < Self::UNTOUCHED && rim < Self::UNTOUCHED {
                    continue;
                }
                let index = (row * mask.width() + column) * 4;
                let Some(pixel) = pixels.get_mut(index..index + 4) else { continue };
                let surface = StoneRgb::new([f64::from(pixel[0]), f64::from(pixel[1]), f64::from(pixel[2])]);
                let surface = surface.mix(light_tint, rim);
                let gilded = (mask.gilt_at(column, row) / mask.cut_at(column, row).max(Self::UNTOUCHED)).clamp(0.0, 1.0) * self.recipe.gilt_amount;
                let grain = 1.0 + self.recipe.grain * chisel.grain(x, y);
                let sample = floor_stone.at(column, row, vee.sink(column, row));
                let ground = tint.mean(column, row).mix(light_tint, rim);
                let gold = gilt.lit(1.0 + self.recipe.grain * chisel.grain(x * Self::LEAF_SCALE, y * Self::LEAF_SCALE));
                let tooled = tint.carry(ground.dulled(self.recipe.dullness).lit(self.recipe.depth_tone).frosted(frost), sample, column, row).lit(grain);
                let floor = tooled.mix(gold, gilded);
                let inked = tint.carry(ground.dulled(self.recipe.dullness).lit(self.recipe.incised_tone).frosted(frost), sample, column, row).lit(grain).mix(gold, gilded);
                let incision = (mask.incised_at(column, row) / mask.cut_at(column, row).max(Self::UNTOUCHED)).clamp(0.0, 1.0);
                let grooved = (mask.grooved_at(column, row) / mask.cut_at(column, row).max(Self::UNTOUCHED)).clamp(0.0, 1.0);
                let shaded = floor.lit(1.0 - self.recipe.shadow * shadow.at(column, row));
                let facing = (Self::WALL_GAIN * slope.facing(column, row)).clamp(-1.0, 1.0);
                let groove = if facing < 0.0 {
                    shaded.lit(1.0 + self.recipe.shadow * facing)
                } else {
                    shaded.lit(1.0 + facing * self.recipe.highlight).mix(light_tint, facing * self.recipe.highlight * Self::SHEEN)
                };
                let chiselled = tint.carry(ground.dulled(self.recipe.dullness).lit(self.recipe.depth_tone).frosted(relief_frost), sample, column, row).lit(grain);
                let relief = chiselled.lit(self.recipe.relief_tone).mix(gold, gilded).lit(vee.light(column, row));
                let wall = relief.mix(groove, grooved * (1.0 - self.recipe.line_vee));
                pixel.copy_from_slice(&surface.mix(wall.mix(inked, incision).shouldered(Self::KNEE), cut).rgba());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StoneEtch;
    use crate::{CarveGroove, CarveMask, EtchRecipe, HexColor};

    struct EtchFixture;

    impl EtchFixture {
        const SIZE: usize = 80;
        const GREY: u8 = 128;

        fn recipe() -> EtchRecipe {
            EtchRecipe {
                depth_tone: 0.8,
                frost: HexColor::rgb([30, 30, 30]),
                relief_frost: HexColor::rgb([30, 30, 30]),
                dullness: 0.5,
                incised_tone: 0.5,
                grain: 0.0,
                ragged: 0.0,
                shadow: 0.7,
                shadow_softness: 1.0,
                shadow_offset: 3.0,
                highlight: 0.8,
                vee_slope: 1.0,
                vee_floor: 3.0,
                ambient: 0.3,
                relief_tone: 0.9,
                line_vee: 0.0,
                parallax: 0.0,
                grain_keep: 1.0,
                thicken: 0.0,
                rim: 0.0,
                light_tint: HexColor::rgb([255, 255, 255]),
                gilt: HexColor::rgb([200, 160, 70]),
                gilt_amount: 0.9,
            }
        }

        fn etched_on(groove: CarveGroove, stone: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
            let mut mask = CarveMask::new(Self::SIZE, Self::SIZE, 1.0);
            mask.stamp_groove(&groove);
            let mut pixels: Vec<u8> = (0..Self::SIZE * Self::SIZE).flat_map(|index| stone(index % Self::SIZE, index / Self::SIZE)).collect();
            StoneEtch::new(Self::recipe(), 1).apply(&mut pixels, &mask);
            pixels
        }

        fn letter_stroke() -> Vec<u8> {
            (0..Self::SIZE * Self::SIZE).flat_map(|index| if (30..42).contains(&(index % Self::SIZE)) { [255, 0, 0, 255] } else { [0, 0, 0, 255] }).collect()
        }

        fn etched(groove: CarveGroove) -> Vec<u8> {
            Self::etched_on(groove, |_, _| [Self::GREY; 4])
        }

        fn pixel(pixels: &[u8], column: usize, row: usize) -> [u8; 4] {
            let index = (row * Self::SIZE + column) * 4;
            [pixels[index], pixels[index + 1], pixels[index + 2], pixels[index + 3]]
        }

        fn brightness(pixels: &[u8], column: usize, row: usize) -> u32 {
            Self::pixel(pixels, column, row)[..3].iter().map(|channel| u32::from(*channel)).sum()
        }
    }

    #[test]
    fn stone_away_from_the_carving_is_untouched() {
        let pixels = EtchFixture::etched(CarveGroove::line((10.0, 40.0), (70.0, 40.0), 6.0));

        assert_eq!(EtchFixture::pixel(&pixels, 40, 5), [128, 128, 128, 128]);
    }

    #[test]
    fn the_polished_surface_casts_its_shadow_into_the_upper_part_of_the_cut() {
        let pixels = EtchFixture::etched(CarveGroove::line((10.0, 40.0), (70.0, 40.0), 12.0));
        let upper = EtchFixture::brightness(&pixels, 40, 36);
        let lower = EtchFixture::brightness(&pixels, 40, 42);

        assert!(upper + 60 < lower, "upper {upper}, lower {lower}");
        assert!(upper < 3 * 128, "upper {upper}");
    }

    #[test]
    fn the_lower_wall_of_a_cut_catches_the_light() {
        let pixels = EtchFixture::etched(CarveGroove::line((10.0, 40.0), (70.0, 40.0), 12.0));
        let wall = (42..47).map(|row| EtchFixture::brightness(&pixels, 40, row)).max().unwrap();

        assert!(wall > 3 * 128 + 20, "wall {wall}");
    }

    #[test]
    fn the_left_wall_of_a_cut_is_darker_than_the_right() {
        let pixels = EtchFixture::etched(CarveGroove::line((40.0, 10.0), (40.0, 70.0), 12.0));
        let left = EtchFixture::brightness(&pixels, 36, 40);
        let right = EtchFixture::brightness(&pixels, 44, 40);

        assert!(left < right, "left {left}, right {right}");
    }

    #[test]
    fn stone_just_outside_a_cut_keeps_its_polish() {
        let pixels = EtchFixture::etched(CarveGroove::line((10.0, 40.0), (70.0, 40.0), 8.0));

        assert_eq!(EtchFixture::pixel(&pixels, 40, 34), [128, 128, 128, 128]);
        assert_eq!(EtchFixture::pixel(&pixels, 40, 46), [128, 128, 128, 128]);
    }

    #[test]
    fn a_vein_running_under_a_cut_still_shows_on_its_floor() {
        let vein = |column: usize, _row: usize| if (38..=41).contains(&column) { [230, 230, 230, 255] } else { [40, 40, 40, 255] };
        let pixels = EtchFixture::etched_on(CarveGroove::line((10.0, 40.0), (70.0, 40.0), 12.0), vein);
        let on_vein = EtchFixture::brightness(&pixels, 40, 41);
        let off_vein = EtchFixture::brightness(&pixels, 30, 41);

        assert!(on_vein > off_vein + 150, "on {on_vein}, off {off_vein}");
    }

    #[test]
    fn incised_lettering_is_an_even_tone_without_walls() {
        let mut mask = CarveMask::new(EtchFixture::SIZE, EtchFixture::SIZE, 1.0);
        let blue = (0..EtchFixture::SIZE * EtchFixture::SIZE).flat_map(|index| if (30..50).contains(&(index / EtchFixture::SIZE)) { [255, 0, 255, 255] } else { [0, 0, 0, 255] });
        mask = CarveMask::from_rgba(mask.width(), mask.height(), 1.0, &blue.collect::<Vec<u8>>());
        let mut pixels = vec![EtchFixture::GREY; EtchFixture::SIZE * EtchFixture::SIZE * 4];
        StoneEtch::new(EtchFixture::recipe(), 1).apply(&mut pixels, &mask);
        let (upper, lower) = (EtchFixture::brightness(&pixels, 40, 31), EtchFixture::brightness(&pixels, 40, 48));

        assert!(upper.abs_diff(lower) < 6, "upper {upper}, lower {lower}");
        assert!(upper < 3 * 128 - 60, "upper {upper}");
    }

    #[test]
    fn a_carved_letter_stroke_has_a_dark_facet_and_a_lit_facet_either_side_of_its_crease() {
        let mask = CarveMask::from_rgba(EtchFixture::SIZE, EtchFixture::SIZE, 1.0, &EtchFixture::letter_stroke());
        let mut pixels = vec![EtchFixture::GREY; EtchFixture::SIZE * EtchFixture::SIZE * 4];
        StoneEtch::new(EtchFixture::recipe(), 1).apply(&mut pixels, &mask);
        let (shaded, lit) = (EtchFixture::brightness(&pixels, 32, 60), EtchFixture::brightness(&pixels, 39, 60));

        assert!(shaded + 100 < lit, "shaded {shaded}, lit {lit}");
    }

    #[test]
    fn a_thin_parian_stem_at_double_density_still_splits_into_a_shaded_and_a_lit_face() {
        let stem: Vec<u8> = (0..EtchFixture::SIZE * EtchFixture::SIZE).flat_map(|index| if (36..46).contains(&(index % EtchFixture::SIZE)) { [255, 0, 0, 255] } else { [0, 0, 0, 255] }).collect();
        let mask = CarveMask::from_rgba(EtchFixture::SIZE, EtchFixture::SIZE, 2.0, &stem);
        let mut pixels = vec![240; EtchFixture::SIZE * EtchFixture::SIZE * 4];
        StoneEtch::new(crate::MarbleKind::Parian.etch(), 1).apply(&mut pixels, &mask);
        let row: Vec<u32> = (34..48).map(|column| EtchFixture::brightness(&pixels, column, 40) / 3).collect();

        assert!(row[3] + 80 < row[9], "{row:?}");
    }

    #[test]
    fn letters_cut_into_black_stone_show_a_lit_face_and_a_dark_one_without_glaring() {
        let stem: Vec<u8> = (0..EtchFixture::SIZE * EtchFixture::SIZE).flat_map(|index| if (36..46).contains(&(index % EtchFixture::SIZE)) { [255, 0, 0, 255] } else { [0, 0, 0, 255] }).collect();
        let mask = CarveMask::from_rgba(EtchFixture::SIZE, EtchFixture::SIZE, 2.0, &stem);
        let mut pixels = vec![25; EtchFixture::SIZE * EtchFixture::SIZE * 4];
        StoneEtch::new(crate::MarbleKind::Nero.etch(), 1).apply(&mut pixels, &mask);
        let row: Vec<u32> = (34..48).map(|column| EtchFixture::brightness(&pixels, column, 40) / 3).collect();

        assert!((70..130).contains(&row[9]), "{row:?}");
        assert!(row[3] + 40 < row[9], "{row:?}");
    }

    #[test]
    fn gilt_letters_are_lit_like_the_stone_letters_around_them() {
        let gilt_stroke: Vec<u8> = (0..EtchFixture::SIZE * EtchFixture::SIZE).flat_map(|index| if (30..42).contains(&(index % EtchFixture::SIZE)) { [255, 255, 0, 255] } else { [0, 0, 0, 255] }).collect();
        let mask = CarveMask::from_rgba(EtchFixture::SIZE, EtchFixture::SIZE, 1.0, &gilt_stroke);
        let mut pixels = vec![EtchFixture::GREY; EtchFixture::SIZE * EtchFixture::SIZE * 4];
        StoneEtch::new(EtchFixture::recipe(), 1).apply(&mut pixels, &mask);
        let (shaded, lit) = (EtchFixture::brightness(&pixels, 32, 60), EtchFixture::brightness(&pixels, 39, 60));

        assert!(shaded + 100 < lit, "shaded {shaded}, lit {lit}");
    }

    #[test]
    fn a_vein_crossing_a_letter_stays_visible_on_both_facets() {
        let mask = CarveMask::from_rgba(EtchFixture::SIZE, EtchFixture::SIZE, 1.0, &EtchFixture::letter_stroke());
        let vein = |_column: usize, row: usize| if (50..=53).contains(&row) { [60, 60, 60, 255] } else { [200, 200, 200, 255] };
        let mut pixels: Vec<u8> = (0..EtchFixture::SIZE * EtchFixture::SIZE).flat_map(|index| vein(index % EtchFixture::SIZE, index / EtchFixture::SIZE)).collect();
        StoneEtch::new(EtchFixture::recipe(), 1).apply(&mut pixels, &mask);

        for column in [32, 39] {
            let (on, off) = (EtchFixture::brightness(&pixels, column, 51), EtchFixture::brightness(&pixels, column, 30));
            assert!(on * 2 < off, "column {column}: on {on}, off {off}");
        }
    }

    #[test]
    fn a_vein_under_a_deep_cut_shows_on_its_floor_shifted_away_from_the_light() {
        let mask = CarveMask::from_rgba(EtchFixture::SIZE, EtchFixture::SIZE, 1.0, &EtchFixture::letter_stroke());
        let vein = |_column: usize, row: usize| if (50..=52).contains(&row) { [200, 200, 200, 255] } else { [40, 40, 40, 255] };
        let stone: Vec<u8> = (0..EtchFixture::SIZE * EtchFixture::SIZE).flat_map(|index| vein(index % EtchFixture::SIZE, index / EtchFixture::SIZE)).collect();
        let carve = |parallax: f64| {
            let mut pixels = stone.clone();
            StoneEtch::new(EtchRecipe { parallax, ..EtchFixture::recipe() }, 1).apply(&mut pixels, &mask);
            pixels
        };
        let (flat, deep) = (carve(0.0), carve(3.0));
        let brightest = |pixels: &[u8]| (40..60).max_by_key(|row| EtchFixture::brightness(pixels, 36, *row)).unwrap();

        assert!((50..=52).contains(&brightest(&flat)), "flat vein at row {}", brightest(&flat));
        assert!(brightest(&deep) + 2 <= brightest(&flat), "deep {} flat {}", brightest(&deep), brightest(&flat));
    }

    #[test]
    fn black_stone_grain_keeps_its_contrast_inside_a_lightened_letter() {
        let mask = CarveMask::from_rgba(EtchFixture::SIZE, EtchFixture::SIZE, 2.0, &EtchFixture::letter_stroke());
        let speckle = |column: usize, row: usize| if (column + row).is_multiple_of(5) { [10, 10, 10, 255] } else { [30, 30, 30, 255] };
        let mut pixels: Vec<u8> = (0..EtchFixture::SIZE * EtchFixture::SIZE).flat_map(|index| speckle(index % EtchFixture::SIZE, index / EtchFixture::SIZE)).collect();
        StoneEtch::new(EtchRecipe { grain: 0.0, ragged: 0.0, parallax: 0.0, ..crate::MarbleKind::Nero.etch() }, 1).apply(&mut pixels, &mask);
        let (fleck, ground) = (EtchFixture::brightness(&pixels, 39, 61), EtchFixture::brightness(&pixels, 39, 62));

        assert!(ground > 3 * 60, "ground {ground}");
        assert!(ground > fleck + 3 * 40, "fleck {fleck}, ground {ground}");
    }

    #[test]
    fn a_gilt_cut_shows_gold_in_its_floor() {
        let plain = EtchFixture::etched(CarveGroove::line((10.0, 40.0), (70.0, 40.0), 14.0));
        let gilt = EtchFixture::etched(CarveGroove::line((10.0, 40.0), (70.0, 40.0), 14.0).gilt());
        let [red, _, blue, _] = EtchFixture::pixel(&gilt, 40, 41);
        let [plain_red, _, plain_blue, _] = EtchFixture::pixel(&plain, 40, 41);

        assert!(red > blue + 40, "gilt {red} {blue}");
        assert!(plain_red.abs_diff(plain_blue) < 4, "plain {plain_red} {plain_blue}");
    }
}
