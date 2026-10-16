use crate::{BoxBlur, CarveMask, DistanceField};

#[derive(Debug, Clone, PartialEq)]
pub struct VeeCut {
    width: usize,
    height: usize,
    depth: Vec<f32>,
    floor: f64,
    slope: f64,
    ambient: f64,
}

impl VeeCut {
    const INSIDE: f32 = 0.5;
    const LIGHT: [f64; 3] = [-0.5, -0.5, std::f64::consts::FRAC_1_SQRT_2];
    const PASSES: usize = 1;

    #[must_use]
    pub fn new(mask: &CarveMask, slope: f64, floor: f64, ambient: f64) -> Self {
        let (width, height) = (mask.width(), mask.height());
        let cuts = mask.cuts();
        let field = DistanceField::new(width, height, |index| cuts[index] >= Self::INSIDE);
        let floor = floor * mask.scale();
        let raw: Vec<f32> = (0..width * height).map(|index| field.at(index % width, index / width).min(floor) as f32).collect();
        let depth = BoxBlur::new(width, height, 1, Self::PASSES).blur(&raw);
        Self { width, height, depth, floor, slope, ambient }
    }

    #[must_use]
    pub fn sink(&self, column: usize, row: usize) -> f64 {
        if self.floor <= 0.0 {
            return 0.0;
        }
        (self.depth(column, row) / self.floor).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn light(&self, column: usize, row: usize) -> f64 {
        let (left, right) = (column.saturating_sub(1), (column + 1).min(self.width - 1));
        let (up, down) = (row.saturating_sub(1), (row + 1).min(self.height - 1));
        let across = (self.depth(right, row) - self.depth(left, row)) / (right - left).max(1) as f64;
        let downward = (self.depth(column, down) - self.depth(column, up)) / (down - up).max(1) as f64;
        let normal = [self.slope * across, self.slope * downward, 1.0];
        let length = normal.iter().map(|axis| axis * axis).sum::<f64>().sqrt();
        let lambert = (normal.iter().zip(Self::LIGHT).map(|(axis, light)| axis * light).sum::<f64>() / length).max(0.0);
        let flat = Self::LIGHT[2];
        (self.ambient + (1.0 - self.ambient) * lambert) / (self.ambient + (1.0 - self.ambient) * flat)
    }

    fn depth(&self, column: usize, row: usize) -> f64 {
        f64::from(self.depth[row * self.width + column])
    }
}

#[cfg(test)]
mod tests {
    use super::VeeCut;
    use crate::{CarveGroove, CarveMask};

    struct VeeFixture;

    impl VeeFixture {
        fn upright_stroke(floor: f64) -> VeeCut {
            let mut mask = CarveMask::new(40, 40, 1.0);
            mask.stamp_groove(&CarveGroove::line((20.0, 2.0), (20.0, 38.0), 12.0));
            VeeCut::new(&mask, 1.0, floor, 0.4)
        }
    }

    #[test]
    fn the_facet_facing_away_from_the_light_is_darker_than_the_polished_face_and_the_other_brighter() {
        let vee = VeeFixture::upright_stroke(20.0);

        assert!(vee.light(16, 20) < 0.7, "{}", vee.light(16, 20));
        assert!(vee.light(24, 20) > 1.03, "{}", vee.light(24, 20));
    }

    #[test]
    fn the_shaded_facet_keeps_some_light_from_the_sky() {
        let vee = VeeFixture::upright_stroke(20.0);

        assert!(vee.light(16, 20) > 0.3, "{}", vee.light(16, 20));
    }

    #[test]
    fn a_stroke_wider_than_the_floor_depth_has_a_flat_bottom() {
        let vee = VeeFixture::upright_stroke(2.0);

        assert!((vee.light(20, 20) - 1.0).abs() < 0.05, "{}", vee.light(20, 20));
        assert!(vee.light(14, 20) < 0.9, "{}", vee.light(14, 20));
    }

    #[test]
    fn the_walls_are_as_steep_at_double_density_as_at_single() {
        let stroke = |scale: f64| {
            let mut mask = CarveMask::new((40.0 * scale) as usize, (40.0 * scale) as usize, scale);
            mask.stamp_groove(&CarveGroove::line((20.0, 2.0), (20.0, 38.0), 12.0));
            VeeCut::new(&mask, 1.0, 20.0, 0.4).light((16.0 * scale) as usize, (20.0 * scale) as usize)
        };

        assert!((stroke(1.0) - stroke(2.0)).abs() < 0.12, "{} vs {}", stroke(1.0), stroke(2.0));
    }

    #[test]
    fn a_cut_sinks_from_nothing_at_its_edge_to_its_full_depth_at_its_floor() {
        let vee = VeeFixture::upright_stroke(3.0);

        assert!(vee.sink(4, 20) < 1e-6, "{}", vee.sink(4, 20));
        assert!(vee.sink(14, 20) < 0.6, "{}", vee.sink(14, 20));
        assert!((vee.sink(20, 20) - 1.0).abs() < 1e-6, "{}", vee.sink(20, 20));
    }

    #[test]
    fn uncut_stone_is_lit_like_the_polished_surface() {
        assert!((VeeFixture::upright_stroke(20.0).light(4, 20) - 1.0).abs() < 1e-6);
    }
}
