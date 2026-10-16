use crate::{CarveMask, Carving, EtchRecipe, FumePolygon, StoneEtch, StoneMottle, StoneRelief, StoneShadow, WallLight, WallStone};

#[derive(Debug, Clone, PartialEq)]
pub struct StoneFace {
    outline: FumePolygon,
    origin: (f64, f64),
    size: (usize, usize),
    scale: f64,
    tone: f64,
    tint: [f64; 3],
    light: WallLight,
    seed: u64,
}

impl StoneFace {
    const HOLLOW_DARK: f64 = 0.09;
    const HOLLOW_MOST: f64 = 0.4;
    const RIM_DARK: f64 = 0.45;
    const RIM_WIDTH: f64 = 1.8;

    #[must_use]
    pub fn new(stone: &WallStone, scale: f64, tone: f64, tint: [f64; 3], light: WallLight, seed: u64) -> Self {
        let bounds = stone.bounds();
        let pad = StoneShadow::reach();
        let origin = (bounds.left - 1.0, bounds.top - 1.0);
        let size = (((bounds.width() + 1.0 + pad) * scale).ceil() as usize, ((bounds.height() + 1.0 + pad) * scale).ceil() as usize);
        let outline = FumePolygon::new(stone.outline().points().iter().map(|(x, y)| (x - origin.0, y - origin.1)).collect());
        Self { outline, origin, size, scale, tone, tint, light, seed }
    }

    #[must_use]
    pub const fn origin(&self) -> (f64, f64) {
        self.origin
    }

    #[must_use]
    pub const fn pixel_size(&self) -> (usize, usize) {
        self.size
    }

    #[must_use]
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    #[must_use]
    pub fn local(&self, carvings: &[Carving]) -> Vec<Carving> {
        carvings.iter().map(|carving| carving.shifted(-self.origin.0, -self.origin.1)).collect()
    }

    #[must_use]
    pub fn relief(&self) -> StoneRelief {
        StoneRelief::carve(&self.outline, self.size, self.scale, self.seed)
    }

    pub fn dress(&self, pixels: &mut [u8], mask: Option<&CarveMask>, recipe: EtchRecipe) {
        let relief = self.relief();
        let mottle = StoneMottle::new(self.tint, self.seed);
        for index in 0..self.size.0 * self.size.1 {
            let cover = relief.cover(index);
            if cover <= 0.0 {
                continue;
            }
            let (x, y) = ((index % self.size.0) as f64 / self.scale + self.origin.0, (index / self.size.0) as f64 / self.scale + self.origin.1);
            let shine = self.light.shine(relief.normal(index), x, y);
            let hollow = (relief.hollow(index) * Self::HOLLOW_DARK).min(Self::HOLLOW_MOST);
            let rim = (1.0 - relief.rise(index) / Self::RIM_WIDTH).clamp(0.0, 1.0) * Self::RIM_DARK;
            let shade = self.tone * (1.0 - hollow) * (1.0 - rim);
            let colour = mottle.at(x, y);
            if let Some(pixel) = pixels.get_mut(index * 4..index * 4 + 3) {
                for ((channel, tint), lit) in pixel.iter_mut().zip(colour).zip(shine) {
                    *channel = (f64::from(*channel) * shade * tint * lit).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        if let Some(mask) = mask {
            StoneEtch::new(recipe, self.seed).apply(pixels, mask);
        }
        let shadow = StoneShadow::cast(&relief, self.scale);
        for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let stone = relief.cover(index);
            let under = shadow.at(index) * (1.0 - stone);
            let alpha = stone + under;
            if alpha <= 0.0 {
                *pixel = [0; 4];
                continue;
            }
            for channel in &mut pixel[..3] {
                *channel = (f64::from(*channel) * stone / alpha).round() as u8;
            }
            pixel[3] = (alpha * 255.0).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StoneFace;
    use crate::{CarveFont, CarveText, Carving, Limestone, WallLight, WallMason};

    fn face() -> StoneFace {
        let stone = WallMason::new(1000.0, 100.0, 3).lay(0, &[260.0]).remove(0).shifted(50.0, 400.0);
        StoneFace::new(&stone, 2.0, 1.0, [1.0; 3], WallLight::new((0.0, 0.0), 1600.0), 9)
    }

    fn dressed(face: &StoneFace) -> Vec<u8> {
        let (width, height) = face.pixel_size();
        let mut pixels = vec![160; width * height * 4];
        face.dress(&mut pixels, None, Limestone::ETCH);
        pixels
    }

    fn pixel(face: &StoneFace, pixels: &[u8], x: f64, y: f64) -> [u8; 4] {
        let (column, row) = ((x * face.scale()) as usize, (y * face.scale()) as usize);
        let at = (row * face.pixel_size().0 + column) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    fn column_average(face: &StoneFace, pixels: &[u8], x: f64, from: f64, to: f64) -> f64 {
        let rows: Vec<f64> = (0..20).map(|step| from + (to - from) * f64::from(step) / 19.0).collect();
        rows.iter().map(|y| f64::from(pixel(face, pixels, x, *y)[0])).sum::<f64>() / rows.len() as f64
    }

    #[test]
    fn the_stone_is_opaque_inside_and_clear_in_the_joints_around_it() {
        let face = face();
        let pixels = dressed(&face);
        let (width, height) = face.pixel_size();

        assert_eq!(pixel(&face, &pixels, width as f64 / 4.0, height as f64 / 4.0)[3], 255);
        assert_eq!(pixels[3], 0);
        let corner = (width * height - 1) * 4;
        assert_eq!(pixels[corner + 3], 0);
    }

    #[test]
    fn the_upper_edge_catches_the_light_and_the_lower_edge_falls_into_shade() {
        let face = face();
        let pixels = dressed(&face);
        let middle = face.pixel_size().0 as f64 / 4.0;
        let solid: Vec<f64> = (0..face.pixel_size().1).map(|row| row as f64 / face.scale()).filter(|y| pixel(&face, &pixels, middle, *y)[3] == 255).collect();
        let (top, bottom) = (solid[0], solid[solid.len() - 1]);
        let middle_y = (top + bottom) / 2.0;
        let upper = column_average(&face, &pixels, middle, top + 4.0, top + 9.0);
        let centre = column_average(&face, &pixels, middle, middle_y - 3.0, middle_y + 3.0);
        let lower = column_average(&face, &pixels, middle, bottom - 9.0, bottom - 4.0);

        assert!(upper > centre, "{upper} {centre}");
        assert!(lower < centre - 15.0, "{lower} {centre}");
    }

    #[test]
    fn carvings_are_moved_into_the_stone_own_frame() {
        let face = face();
        let text: Carving = CarveText::new("A", CarveFont::Display, 10.0, face.origin().0 + 20.0, face.origin().1 + 30.0).into();
        let Carving::Text(local) = &face.local(&[text])[0] else { panic!("text") };

        assert!((local.x - 20.0).abs() < 1e-9 && (local.y - 30.0).abs() < 1e-9);
    }
}
