use crate::{BoxBlur, CarveMask, DistanceField};

#[derive(Debug, Clone, PartialEq)]
pub struct CarveRelief {
    width: usize,
    height: usize,
    scale: f64,
    depth: Vec<f32>,
}

impl CarveRelief {
    const INSIDE: f32 = 0.2;
    const EDGE: f64 = 0.5;
    const SMOOTHING: usize = 1;

    #[must_use]
    pub fn new(mask: &CarveMask, slope: f64, floor: f64) -> Self {
        let (width, height, scale) = (mask.width(), mask.height(), mask.scale());
        let cuts = mask.cuts();
        let field = DistanceField::new(width, height, |index| cuts[index] >= Self::INSIDE);
        let raw: Vec<f32> = cuts
            .iter()
            .enumerate()
            .map(|(index, cut)| {
                let reach = (field.at(index % width, index / width) - Self::EDGE).max(0.0) / scale;
                ((reach * slope).min(floor) * f64::from(*cut).min(1.0)) as f32
            })
            .collect();
        let depth = BoxBlur::new(width, height, Self::SMOOTHING, 1).blur(&raw);
        Self { width, height, scale, depth }
    }

    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    #[must_use]
    pub fn depths(&self) -> &[f32] {
        &self.depth
    }

    #[must_use]
    pub fn at(&self, column: usize, row: usize) -> f64 {
        self.depth.get(row * self.width + column).map_or(0.0, |depth| f64::from(*depth))
    }

    #[must_use]
    pub fn normals(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height * 4);
        for row in 0..self.height {
            for column in 0..self.width {
                let (across, down) = self.gradient(column, row);
                let length = (across * across + down * down + 1.0).sqrt();
                out.extend([across / length, -down / length, 1.0 / length].map(Self::encode));
                out.push(u8::MAX);
            }
        }
        out
    }

    fn gradient(&self, column: usize, row: usize) -> (f64, f64) {
        let (left, right) = (column.saturating_sub(1), (column + 1).min(self.width - 1));
        let (up, down) = (row.saturating_sub(1), (row + 1).min(self.height - 1));
        let across = (self.at(right, row) - self.at(left, row)) * self.scale / (right - left).max(1) as f64;
        let downward = (self.at(column, down) - self.at(column, up)) * self.scale / (down - up).max(1) as f64;
        (across, downward)
    }

    fn encode(axis: f64) -> u8 {
        ((axis * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::CarveRelief;
    use crate::{CarveGroove, CarveMask};

    struct ReliefFixture;

    impl ReliefFixture {
        fn groove(from: (f64, f64), to: (f64, f64), width: f64, scale: f64) -> CarveRelief {
            let size = (60.0 * scale) as usize;
            let mut mask = CarveMask::new(size, size, scale);
            mask.stamp_groove(&CarveGroove::line(from, to, width));
            CarveRelief::new(&mask, 1.0, 4.0)
        }

        fn normal(relief: &CarveRelief, column: usize, row: usize) -> [u8; 3] {
            let normals = relief.normals();
            let index = (row * relief.width() + column) * 4;
            [normals[index], normals[index + 1], normals[index + 2]]
        }
    }

    #[test]
    fn untouched_stone_stays_at_the_surface_and_faces_straight_up() {
        let relief = ReliefFixture::groove((30.0, 5.0), (30.0, 55.0), 6.0, 1.0);

        assert!(relief.at(5, 30).abs() < 1e-9);
        assert_eq!(ReliefFixture::normal(&relief, 5, 30), [128, 128, 255]);
    }

    #[test]
    fn a_cut_deepens_towards_its_middle_until_it_reaches_the_floor() {
        let relief = ReliefFixture::groove((30.0, 5.0), (30.0, 55.0), 20.0, 1.0);
        let profile: Vec<f64> = (20..=30).map(|column| relief.at(column, 30)).collect();

        assert!(profile.windows(2).all(|pair| pair[1] >= pair[0] - 1e-6), "{profile:?}");
        assert!((relief.at(30, 30) - 4.0).abs() < 1e-6, "{}", relief.at(30, 30));
        assert!(relief.at(21, 30) + 1.0 < relief.at(30, 30), "{}", relief.at(21, 30));
    }

    #[test]
    fn a_cut_is_as_deep_in_css_pixels_at_double_density() {
        let single = ReliefFixture::groove((30.0, 5.0), (30.0, 55.0), 6.0, 1.0).at(30, 30);
        let double = ReliefFixture::groove((30.0, 5.0), (30.0, 55.0), 6.0, 2.0).at(60, 60);

        assert!((single - double).abs() < 0.4, "{single} vs {double}");
    }

    #[test]
    fn the_walls_of_an_upright_groove_face_each_other() {
        let relief = ReliefFixture::groove((30.0, 5.0), (30.0, 55.0), 10.0, 1.0);
        let ([left, ..], [right, ..]) = (ReliefFixture::normal(&relief, 27, 30), ReliefFixture::normal(&relief, 33, 30));

        assert!(left > 160, "left wall {left}");
        assert!(right < 96, "right wall {right}");
    }

    #[test]
    fn the_upper_wall_of_a_level_groove_faces_down_the_slab() {
        let relief = ReliefFixture::groove((5.0, 30.0), (55.0, 30.0), 10.0, 1.0);
        let ([_, upper, _], [_, lower, _]) = (ReliefFixture::normal(&relief, 30, 27), ReliefFixture::normal(&relief, 30, 33));

        assert!(upper < 96, "upper wall {upper}");
        assert!(lower > 160, "lower wall {lower}");
    }

    #[test]
    fn a_shallow_groove_is_cut_less_deep() {
        let mut mask = CarveMask::new(60, 60, 1.0);
        mask.stamp_groove(&CarveGroove::line((30.0, 5.0), (30.0, 55.0), 20.0).depth(0.4));
        let relief = CarveRelief::new(&mask, 1.0, 4.0);

        assert!((relief.at(30, 30) - 1.6).abs() < 0.05, "{}", relief.at(30, 30));
    }
}
