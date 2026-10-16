use crate::{BoxBlur, StoneRgb};

#[derive(Debug, Clone, PartialEq)]
pub struct GrainTint {
    width: usize,
    mean: [Vec<f32>; 3],
    keep: f64,
}

impl GrainTint {
    const PASSES: usize = 2;
    const FLOOR: f64 = 6.0;
    const MAX_GAIN: f64 = 6.0;

    #[must_use]
    pub fn new(width: usize, height: usize, stone: &[u8], reach: usize, keep: f64) -> Self {
        let blur = BoxBlur::new(width, height, reach, Self::PASSES);
        let channel = |offset: usize| -> Vec<f32> { stone.as_chunks::<4>().0.iter().map(|pixel| f32::from(pixel[offset])).collect() };
        Self { width, mean: [blur.blur(&channel(0)), blur.blur(&channel(1)), blur.blur(&channel(2))], keep }
    }

    #[must_use]
    pub fn mean(&self, column: usize, row: usize) -> StoneRgb {
        let index = row * self.width + column;
        StoneRgb::new(self.mean.each_ref().map(|channel| channel.get(index).map_or(0.0, |value| f64::from(*value))))
    }

    #[must_use]
    pub fn carry(&self, tinted: StoneRgb, sample: StoneRgb, column: usize, row: usize) -> StoneRgb {
        let mean = self.mean(column, row);
        let light = (255.0 * sample.lightness() + Self::FLOOR) / (255.0 * mean.lightness() + Self::FLOOR);
        let gain = light.powf(self.keep).min(Self::MAX_GAIN);
        let (tinted, sample, mean) = (tinted.channels(), sample.channels(), mean.channels());
        StoneRgb::new([0, 1, 2].map(|channel| {
            let hue = (sample[channel] + Self::FLOOR) / (mean[channel] + Self::FLOOR) / light;
            tinted[channel] * gain * hue
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::GrainTint;
    use crate::StoneRgb;

    struct TintFixture;

    impl TintFixture {
        const SIZE: usize = 30;

        fn speckled(ground: u8, fleck: u8) -> Vec<u8> {
            (0..Self::SIZE * Self::SIZE).flat_map(|index| if index % 7 == 0 { [fleck; 4] } else { [ground; 4] }).collect()
        }

        fn grey(value: f64) -> StoneRgb {
            StoneRgb::new([value; 3])
        }
    }

    #[test]
    fn flat_stone_takes_the_tint_exactly() {
        let stone = vec![40; TintFixture::SIZE * TintFixture::SIZE * 4];
        let tint = GrainTint::new(TintFixture::SIZE, TintFixture::SIZE, &stone, 3, 1.0);

        assert_eq!(tint.carry(TintFixture::grey(150.0), TintFixture::grey(40.0), 15, 15).rgba(), [150, 150, 150, 255]);
    }

    #[test]
    fn a_dark_fleck_in_dark_stone_stays_as_strong_once_the_cut_is_lightened() {
        let stone = TintFixture::speckled(30, 15);
        let tint = GrainTint::new(TintFixture::SIZE, TintFixture::SIZE, &stone, 3, 1.0);
        let mean = tint.mean(15, 15).channels()[0];
        let (ground, fleck) = (tint.carry(TintFixture::grey(150.0), TintFixture::grey(30.0), 15, 15), tint.carry(TintFixture::grey(150.0), TintFixture::grey(15.0), 15, 15));
        let lifted = ground.channels()[0] - fleck.channels()[0];

        assert!(mean > 25.0 && mean < 30.0, "{mean}");
        assert!(lifted > 3.0 * 15.0, "lifted step {lifted}");
    }

    #[test]
    fn a_stronger_keep_deepens_the_grain_without_tinting_it() {
        let stone = vec![60; TintFixture::SIZE * TintFixture::SIZE * 4];
        let tint = GrainTint::new(TintFixture::SIZE, TintFixture::SIZE, &stone, 3, 3.0);
        let [red, green, blue] = tint.carry(TintFixture::grey(100.0), TintFixture::grey(40.0), 10, 10).channels();

        assert!((red - green).abs() < 1e-9 && (green - blue).abs() < 1e-9, "{red} {green} {blue}");
        assert!(red < 100.0 * 40.0 / 60.0, "{red}");
    }

    #[test]
    fn keeping_none_of_the_grain_gives_the_flat_tint() {
        let stone = TintFixture::speckled(30, 15);
        let tint = GrainTint::new(TintFixture::SIZE, TintFixture::SIZE, &stone, 3, 0.0);

        assert_eq!(tint.carry(TintFixture::grey(150.0), TintFixture::grey(15.0), 15, 15).rgba(), [150, 150, 150, 255]);
    }

    #[test]
    fn the_hue_of_a_tinted_vein_follows_the_stone() {
        let stone = vec![60; TintFixture::SIZE * TintFixture::SIZE * 4];
        let tint = GrainTint::new(TintFixture::SIZE, TintFixture::SIZE, &stone, 3, 1.0);
        let [red, green, blue] = tint.carry(TintFixture::grey(100.0), StoneRgb::new([90.0, 60.0, 40.0]), 10, 10).channels();

        assert!(red > green && green > blue, "{red} {green} {blue}");
    }
}
