use crate::{BoxBlur, CarveMask};

#[derive(Debug, Clone, PartialEq)]
pub struct CutShadow {
    width: usize,
    height: usize,
    offset: isize,
    surface: Vec<f32>,
}

impl CutShadow {
    const PASSES: usize = 2;

    #[must_use]
    pub fn new(mask: &CarveMask, softness: usize, offset: isize) -> Self {
        let (width, height) = (mask.width(), mask.height());
        let uncut: Vec<f32> = mask.cuts().iter().map(|cut| 1.0 - cut).collect();
        let surface = BoxBlur::new(width, height, softness, Self::PASSES).blur(&uncut);
        Self { width, height, offset, surface }
    }

    #[must_use]
    pub fn at(&self, column: usize, row: usize) -> f64 {
        let (column, row) = (column as isize - self.offset, row as isize - self.offset);
        if column < 0 || row < 0 || column as usize >= self.width || row as usize >= self.height {
            return 1.0;
        }
        f64::from(self.surface[row as usize * self.width + column as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::CutShadow;
    use crate::{CarveGroove, CarveMask};

    struct ShadowFixture;

    impl ShadowFixture {
        fn groove() -> CarveMask {
            let mut mask = CarveMask::new(60, 60, 1.0);
            mask.stamp_groove(&CarveGroove::line((5.0, 30.0), (55.0, 30.0), 12.0));
            mask
        }
    }

    #[test]
    fn the_surface_shades_the_upper_side_of_a_cut_more_than_the_lower_side() {
        let shadow = CutShadow::new(&ShadowFixture::groove(), 1, 3);

        assert!(shadow.at(30, 26) > shadow.at(30, 34) + 0.3, "{} {}", shadow.at(30, 26), shadow.at(30, 34));
    }

    #[test]
    fn uncut_stone_is_all_surface() {
        let shadow = CutShadow::new(&CarveMask::new(20, 20, 1.0), 2, 2);

        assert!((shadow.at(10, 10) - 1.0).abs() < 1e-6);
    }
}
