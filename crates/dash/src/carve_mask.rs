use crate::{CarveCut, CarveFill, CarveGroove, ValueNoise};

#[derive(Debug, Clone, PartialEq)]
pub struct CarveMask {
    width: usize,
    height: usize,
    scale: f64,
    cut: Vec<f32>,
    gilt: Vec<f32>,
    incised: Vec<f32>,
    grooved: Vec<f32>,
}

impl CarveMask {
    const SOFT_EDGE: f64 = 0.6;
    const SWELL: f64 = 0.22;
    const SWELL_SCALE: f64 = 11.0;
    const DRIFT: f64 = 0.35;
    const DRIFT_SCALE: f64 = 23.0;
    const HAND: ValueNoise = ValueNoise::new(0x5EED_C415);

    #[must_use]
    pub fn new(width: usize, height: usize, scale: f64) -> Self {
        Self { width, height, scale, cut: vec![0.0; width * height], gilt: vec![0.0; width * height], incised: vec![0.0; width * height], grooved: vec![0.0; width * height] }
    }

    #[must_use]
    pub fn from_rgba(width: usize, height: usize, scale: f64, pixels: &[u8]) -> Self {
        let channel = |offset: usize| {
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .take(width * height)
                .map(|pixel| f32::from(pixel[offset]) * f32::from(pixel[3]) / (255.0 * 255.0))
                .collect()
        };
        let mut mask = Self { width, height, scale, cut: channel(0), gilt: channel(1), incised: channel(2), grooved: vec![0.0; width * height] };
        for layer in [&mut mask.cut, &mut mask.gilt, &mut mask.incised] {
            layer.resize(width * height, 0.0);
        }
        mask
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
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    #[must_use]
    pub fn cut_at(&self, column: usize, row: usize) -> f64 {
        self.cut.get(row * self.width + column).map_or(0.0, |cut| f64::from(*cut))
    }

    #[must_use]
    pub fn gilt_at(&self, column: usize, row: usize) -> f64 {
        self.gilt.get(row * self.width + column).map_or(0.0, |gilt| f64::from(*gilt))
    }

    #[must_use]
    pub fn incised_at(&self, column: usize, row: usize) -> f64 {
        self.incised.get(row * self.width + column).map_or(0.0, |incised| f64::from(*incised))
    }

    #[must_use]
    pub fn grooved_at(&self, column: usize, row: usize) -> f64 {
        self.grooved.get(row * self.width + column).map_or(0.0, |grooved| f64::from(*grooved))
    }

    #[must_use]
    pub fn cuts(&self) -> &[f32] {
        &self.cut
    }

    pub fn press(&mut self, column: usize, row: usize, cover: f64, depth: f64, fill: CarveFill, cut_as: CarveCut) {
        if column >= self.width || row >= self.height {
            return;
        }
        let cut = (cover.clamp(0.0, 1.0) * depth) as f32;
        let index = row * self.width + column;
        self.cut[index] = self.cut[index].max(cut);
        if fill == CarveFill::Gilt {
            self.gilt[index] = self.gilt[index].max(cut);
        }
        if cut_as == CarveCut::Incised {
            self.incised[index] = self.incised[index].max(cut);
        }
    }

    pub fn thicken_relief(&mut self, reach: f64) {
        let radius = reach * self.scale;
        let span = radius.ceil() as isize;
        let offsets: Vec<(isize, isize)> = (-span..=span)
            .flat_map(|dy| (-span..=span).map(move |dx| (dx, dy)))
            .filter(|(dx, dy)| ((dx * dx + dy * dy) as f64).sqrt() <= radius)
            .collect();
        let (cut, gilt) = (self.cut.clone(), self.gilt.clone());
        for row in 0..self.height {
            for column in 0..self.width {
                let index = row * self.width + column;
                if self.incised[index] > 0.0 {
                    continue;
                }
                for (dx, dy) in &offsets {
                    let (x, y) = (column as isize + dx, row as isize + dy);
                    if x < 0 || y < 0 || x >= self.width as isize || y >= self.height as isize {
                        continue;
                    }
                    let near = y as usize * self.width + x as usize;
                    if self.incised[near] > 0.0 {
                        continue;
                    }
                    self.cut[index] = self.cut[index].max(cut[near]);
                    self.gilt[index] = self.gilt[index].max(gilt[near]);
                }
            }
        }
    }

    pub fn stamp_groove(&mut self, groove: &CarveGroove) {
        let points = &groove.points;
        let closing = groove.closed.then(|| (points[points.len() - 1], points[0]));
        let segments = points.windows(2).map(|pair| (pair[0], pair[1])).chain(closing);
        for (from, to) in segments {
            self.stamp_segment(from, to, groove.width / 2.0, groove.depth, groove.fill);
        }
    }

    fn stamp_segment(&mut self, from: (f64, f64), to: (f64, f64), half: f64, depth: f64, fill: CarveFill) {
        let (ax, ay, bx, by) = (from.0 * self.scale, from.1 * self.scale, to.0 * self.scale, to.1 * self.scale);
        let half = half * self.scale;
        let reach = half * (1.0 + Self::SWELL + Self::DRIFT) + Self::SOFT_EDGE + 1.0;
        let columns = Self::span(ax.min(bx) - reach, ax.max(bx) + reach, self.width);
        let rows = Self::span(ay.min(by) - reach, ay.max(by) + reach, self.height);
        let (dx, dy) = (bx - ax, by - ay);
        let length_squared = (dx * dx + dy * dy).max(f64::EPSILON);
        for row in rows {
            let py = row as f64 + 0.5;
            for column in columns.clone() {
                let px = column as f64 + 0.5;
                let t = (((px - ax) * dx + (py - ay) * dy) / length_squared).clamp(0.0, 1.0);
                let (cx, cy) = ((ax + dx * t) / self.scale, (ay + dy * t) / self.scale);
                let swell = 1.0 + Self::SWELL * Self::HAND.fractal(cx / Self::SWELL_SCALE, cy / Self::SWELL_SCALE, 2);
                let drift = Self::DRIFT * half * Self::HAND.sample(cx / Self::DRIFT_SCALE - 41.0, cy / Self::DRIFT_SCALE + 17.0);
                let reach_here = half * swell;
                let (ox, oy) = (px - ax - dx * t, py - ay - dy * t);
                let side = (dx * (py - ay) - dy * (px - ax)).signum();
                let distance = (side * ox.hypot(oy) - drift).abs();
                let cover = (Self::smoothstep(reach_here + Self::SOFT_EDGE, reach_here - Self::SOFT_EDGE, distance) * half.min(1.0) * depth) as f32;
                let index = row * self.width + column;
                self.cut[index] = self.cut[index].max(cover);
                self.grooved[index] = self.grooved[index].max(cover);
                if fill == CarveFill::Gilt {
                    self.gilt[index] = self.gilt[index].max(cover);
                }
            }
        }
    }

    fn span(low: f64, high: f64, limit: usize) -> std::ops::Range<usize> {
        let start = low.floor().max(0.0) as usize;
        let end = (high.ceil().max(0.0) as usize).min(limit);
        start.min(end)..end
    }

    fn smoothstep(low: f64, high: f64, value: f64) -> f64 {
        let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::CarveMask;
    use crate::CarveGroove;

    #[test]
    fn the_mask_canvas_red_is_the_cut_green_the_gilt_and_blue_the_incision() {
        let pixels = [255, 0, 0, 255, 128, 128, 0, 255, 0, 0, 0, 255, 255, 0, 255, 51];
        let mask = CarveMask::from_rgba(4, 1, 1.0, &pixels);

        assert!(mask.incised_at(0, 0) < 1e-6);
        assert!((mask.incised_at(3, 0) - 0.2).abs() < 1e-6);

        assert!((mask.cut_at(0, 0) - 1.0).abs() < 1e-6);
        assert!(mask.gilt_at(0, 0) < 1e-6);
        assert!((mask.gilt_at(1, 0) - 128.0 / 255.0).abs() < 1e-6);
        assert!(mask.cut_at(2, 0) < 1e-6);
        assert!((mask.cut_at(3, 0) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn a_groove_cuts_along_its_line_and_nowhere_else() {
        let mut mask = CarveMask::new(60, 40, 1.0);
        mask.stamp_groove(&CarveGroove::line((10.0, 20.0), (50.0, 20.0), 3.0));

        assert!(mask.cut_at(30, 19) > 0.9 || mask.cut_at(30, 20) > 0.9);
        assert!(mask.cut_at(30, 30) < 1e-6);
        assert!(mask.gilt_at(30, 20) < 1e-6);
        assert!(mask.grooved_at(30, 19).max(mask.grooved_at(30, 20)) > 0.9);
    }

    #[test]
    fn a_shallow_gilt_groove_cuts_less_deep_and_marks_the_gilt() {
        let mut mask = CarveMask::new(60, 40, 2.0);
        mask.stamp_groove(&CarveGroove::line((5.0, 10.0), (25.0, 10.0), 3.0).depth(0.4).gilt());
        let (cut, gilt) = (mask.cut_at(30, 20).max(mask.cut_at(30, 19)), mask.gilt_at(30, 20).max(mask.gilt_at(30, 19)));

        assert!((cut - 0.4).abs() < 0.05, "{cut}");
        assert!((gilt - cut).abs() < 1e-6);
    }

    #[test]
    fn pressing_keeps_the_deepest_cut_and_ignores_pixels_off_the_mask() {
        let mut mask = CarveMask::new(4, 4, 1.0);
        mask.press(1, 1, 0.8, 1.0, crate::CarveFill::Gilt, crate::CarveCut::Relief);
        mask.press(1, 1, 0.3, 1.0, crate::CarveFill::Plain, crate::CarveCut::Incised);
        mask.press(9, 9, 1.0, 1.0, crate::CarveFill::Plain, crate::CarveCut::Incised);

        assert!((mask.cut_at(1, 1) - 0.8).abs() < 1e-6);
        assert!((mask.gilt_at(1, 1) - 0.8).abs() < 1e-6);
        assert!((mask.incised_at(1, 1) - 0.3).abs() < 1e-6);
        assert!(mask.grooved_at(1, 1) < 1e-6);
    }

    #[test]
    fn thickening_widens_relief_letters_and_leaves_incised_ones_alone() {
        let mut mask = CarveMask::new(30, 10, 2.0);
        mask.press(5, 5, 1.0, 1.0, crate::CarveFill::Gilt, crate::CarveCut::Relief);
        mask.press(20, 5, 1.0, 1.0, crate::CarveFill::Plain, crate::CarveCut::Incised);
        mask.thicken_relief(1.0);

        assert!((mask.cut_at(7, 5) - 1.0).abs() < 1e-6);
        assert!((mask.gilt_at(5, 7) - 1.0).abs() < 1e-6);
        assert!(mask.cut_at(8, 5) < 1e-6);
        assert!(mask.cut_at(22, 5) < 1e-6);
    }

    #[test]
    fn a_ring_is_cut_all_the_way_round() {
        let mut mask = CarveMask::new(40, 40, 1.0);
        mask.stamp_groove(&CarveGroove::ring((20.0, 20.0), 10.0, 2.0));

        for (column, row) in [(30, 20), (10, 20), (20, 30), (20, 10)] {
            assert!(mask.cut_at(column, row).max(mask.cut_at(column - 1, row - 1)) > 0.5, "({column}, {row})");
        }
        assert!(mask.cut_at(20, 20) < 1e-6);
    }
}
