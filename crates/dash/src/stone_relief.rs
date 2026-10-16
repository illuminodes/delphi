use crate::{BoxBlur, DistanceField, FumePolygon, ValueNoise};

#[derive(Debug, Clone, PartialEq)]
pub struct StoneRelief {
    width: usize,
    height: usize,
    scale: f64,
    rise: Vec<f32>,
    cover: Vec<f32>,
    hollow: Vec<f32>,
}

impl StoneRelief {
    const GAP: f64 = 1.4;
    const SHOULDER: f64 = 0.12;
    const SHOULDER_MIN: f64 = 4.0;
    const SHOULDER_MAX: f64 = 12.0;
    const ROUND: f64 = 0.6;
    const CHIP: f64 = 1.1;
    const CHIP_SCALE: f64 = 7.0;
    const SPALL: f64 = 4.0;
    const SPALL_SCALE: f64 = 30.0;
    const SPALL_FROM: f64 = 0.45;
    const BULGE: f64 = 0.9;
    const RIPPLE: f64 = 1.6;
    const RIPPLE_SCALE: f64 = 22.0;
    const HOLLOW_REACH: f64 = 6.0;

    #[must_use]
    pub fn carve(outline: &FumePolygon, size: (usize, usize), scale: f64, seed: u64) -> Self {
        let (width, height) = size;
        let css = |index: usize| (((index % width) as f64 + 0.5) / scale, ((index / width) as f64 + 0.5) / scale);
        let field = DistanceField::new(width, height, |index| {
            let (x, y) = css(index);
            outline.contains(x, y)
        });
        let deepest = (0..width * height).map(|index| field.at(index % width, index / width)).fold(0.0, f64::max) / scale;
        let shoulder = (deepest * Self::SHOULDER * 2.0).clamp(Self::SHOULDER_MIN, Self::SHOULDER_MAX);
        let (chips, spalls, ripples) = (ValueNoise::new(seed), ValueNoise::new(seed ^ 0x5A11), ValueNoise::new(seed ^ 0xB0B));
        let mut rise = vec![0.0_f32; width * height];
        let mut cover = vec![0.0_f32; width * height];
        for index in 0..width * height {
            let (x, y) = css(index);
            let inward = field.at(index % width, index / width) / scale;
            if inward <= 0.0 {
                continue;
            }
            let spall = ((spalls.fractal(x / Self::SPALL_SCALE, y / Self::SPALL_SCALE, 2) - Self::SPALL_FROM) / (1.0 - Self::SPALL_FROM)).max(0.0);
            let edge = inward - Self::GAP + chips.fractal(x / Self::CHIP_SCALE, y / Self::CHIP_SCALE, 3) * Self::CHIP - spall * Self::SPALL;
            cover[index] = (edge * scale + 0.5).clamp(0.0, 1.0) as f32;
            if edge <= 0.0 {
                continue;
            }
            let across = (edge / shoulder).min(1.0);
            let round = (1.0 - (1.0 - across).powi(2)).sqrt() * shoulder * Self::ROUND;
            let bulge = (inward / deepest.max(1.0)).min(1.0) * shoulder * Self::BULGE;
            let ripple = ripples.fractal(x / Self::RIPPLE_SCALE, y / Self::RIPPLE_SCALE, 3) * Self::RIPPLE * across;
            rise[index] = (round + bulge + ripple) as f32;
        }
        let reach = (Self::HOLLOW_REACH * scale).round().max(1.0) as usize;
        let around = BoxBlur::new(width, height, reach, 2).blur(&rise);
        let hollow = rise.iter().zip(&around).map(|(here, around)| (around - here).max(0.0)).collect();
        Self { width, height, scale, rise, cover, hollow }
    }

    #[must_use]
    pub const fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    #[must_use]
    pub fn cover(&self, index: usize) -> f64 {
        self.cover.get(index).map_or(0.0, |cover| f64::from(*cover))
    }

    #[must_use]
    pub fn rise(&self, index: usize) -> f64 {
        self.rise.get(index).map_or(0.0, |rise| f64::from(*rise))
    }

    #[must_use]
    pub fn hollow(&self, index: usize) -> f64 {
        self.hollow.get(index).map_or(0.0, |hollow| f64::from(*hollow))
    }

    #[must_use]
    pub fn normal(&self, index: usize) -> (f64, f64, f64) {
        let (column, row) = (index % self.width, index / self.width);
        let at = |column: usize, row: usize| self.rise(row * self.width + column);
        let (left, right) = (column.saturating_sub(1), (column + 1).min(self.width - 1));
        let (up, down) = (row.saturating_sub(1), (row + 1).min(self.height - 1));
        let across = (at(right, row) - at(left, row)) * self.scale / (right - left).max(1) as f64;
        let along = (at(column, down) - at(column, up)) * self.scale / (down - up).max(1) as f64;
        let length = (across * across + along * along + 1.0).sqrt();
        (-across / length, -along / length, 1.0 / length)
    }
}

#[cfg(test)]
mod tests {
    use super::StoneRelief;
    use crate::FumePolygon;

    fn relief() -> StoneRelief {
        let outline = FumePolygon::new(vec![(4.0, 4.0), (196.0, 4.0), (196.0, 116.0), (4.0, 116.0)]);
        StoneRelief::carve(&outline, (200, 120), 1.0, 5)
    }

    fn row_rise(relief: &StoneRelief, row: usize) -> Vec<f64> {
        (0..200).map(|column| relief.rise(row * 200 + column)).collect()
    }

    #[test]
    fn the_stone_rises_from_its_edge_to_a_raised_middle() {
        let relief = relief();
        let row = row_rise(&relief, 60);

        assert!(row[1].abs() < 1e-9);
        assert!(row[100] > row[12] + 3.0, "{} {}", row[100], row[12]);
    }

    #[test]
    fn the_edge_is_broken_rather_than_ruled_straight() {
        let relief = relief();
        let first_solid: Vec<usize> = (20..180).map(|column| (0..60).find(|row| relief.cover(row * 200 + column) > 0.5).unwrap_or(60)).collect();
        let (low, high) = (first_solid.iter().min().copied().unwrap_or(0), first_solid.iter().max().copied().unwrap_or(0));

        assert!(high - low >= 3, "{low} {high}");
        assert!(low >= 4);
    }

    #[test]
    fn a_shoulder_leans_outward_and_the_middle_faces_the_viewer() {
        let relief = relief();
        let top_shoulder = (8..20).map(|row| relief.normal(row * 200 + 100).1).fold(0.0, f64::min);

        assert!(top_shoulder < -0.3, "{top_shoulder}");
        assert!(relief.normal(60 * 200 + 100).2 > 0.8);
    }

    #[test]
    fn dips_in_the_face_are_marked_as_hollows() {
        let relief = relief();
        let hollows = (0..200 * 120).filter(|index| relief.hollow(*index) > 0.2).count();

        assert!(hollows > 50, "{hollows}");
        assert!(relief.hollow(60 * 200 + 1).abs() < 1e-9 || relief.cover(60 * 200 + 1) < 0.5);
    }

    #[test]
    fn the_same_seed_carves_the_same_stone() {
        let outline = FumePolygon::new(vec![(4.0, 4.0), (96.0, 4.0), (96.0, 56.0), (4.0, 56.0)]);

        assert_eq!(StoneRelief::carve(&outline, (100, 60), 1.0, 3), StoneRelief::carve(&outline, (100, 60), 1.0, 3));
    }
}
