use crate::{BoxBlur, CarveMask};

#[derive(Debug, Clone, PartialEq)]
pub struct CutSlope {
    width: usize,
    height: usize,
    depth: Vec<f32>,
}

impl CutSlope {
    const PASSES: usize = 1;
    const LIGHT: (f64, f64) = (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2);

    #[must_use]
    pub fn new(mask: &CarveMask, radius: usize) -> Self {
        let (width, height) = (mask.width(), mask.height());
        Self { width, height, depth: BoxBlur::new(width, height, radius, Self::PASSES).blur(mask.cuts()) }
    }

    #[must_use]
    pub fn facing(&self, column: usize, row: usize) -> f64 {
        let (left, right) = (column.saturating_sub(1), (column + 1).min(self.width - 1));
        let (up, down) = (row.saturating_sub(1), (row + 1).min(self.height - 1));
        let across = self.at(right, row) - self.at(left, row);
        let downward = self.at(column, down) - self.at(column, up);
        -(across * Self::LIGHT.0 + downward * Self::LIGHT.1)
    }

    fn at(&self, column: usize, row: usize) -> f64 {
        f64::from(self.depth[row * self.width + column])
    }
}

#[cfg(test)]
mod tests {
    use super::CutSlope;
    use crate::{CarveGroove, CarveMask};

    struct SlopeFixture;

    impl SlopeFixture {
        fn thin_groove() -> CutSlope {
            let mut mask = CarveMask::new(40, 40, 1.0);
            mask.stamp_groove(&CarveGroove::line((2.0, 20.0), (38.0, 20.0), 3.0));
            CutSlope::new(&mask, 1)
        }
    }

    #[test]
    fn even_a_thin_groove_has_an_upper_wall_turned_away_from_the_light_and_a_lower_wall_turned_towards_it() {
        let slope = SlopeFixture::thin_groove();
        let upper = (17..20).map(|row| slope.facing(20, row)).fold(f64::INFINITY, f64::min);
        let lower = (20..23).map(|row| slope.facing(20, row)).fold(f64::NEG_INFINITY, f64::max);

        assert!(upper < -0.1, "{upper}");
        assert!(lower > 0.1, "{lower}");
    }

    #[test]
    fn flat_stone_faces_nowhere() {
        let slope = CutSlope::new(&CarveMask::new(10, 10, 1.0), 1);

        assert!(slope.facing(5, 5).abs() < 1e-9);
    }
}
