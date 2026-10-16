#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallLight {
    pub sun: (f64, f64),
    pub reach: f64,
}

impl WallLight {
    const DIRECTION: (f64, f64, f64) = (-0.62, -0.46, 0.636);
    const KEY: [f64; 3] = [1.12, 0.98, 0.8];
    const FILL: [f64; 3] = [0.5, 0.56, 0.68];
    const NEAR: f64 = 1.08;
    const FAR: f64 = 0.62;

    #[must_use]
    pub const fn new(sun: (f64, f64), reach: f64) -> Self {
        Self { sun, reach }
    }

    #[must_use]
    pub fn falloff(&self, x: f64, y: f64) -> f64 {
        let along = ((x - self.sun.0).hypot(y - self.sun.1) / self.reach.max(1.0)).clamp(0.0, 1.0);
        Self::NEAR + (Self::FAR - Self::NEAR) * along * along.sqrt()
    }

    #[must_use]
    pub fn shine(&self, normal: (f64, f64, f64), x: f64, y: f64) -> [f64; 3] {
        let (lx, ly, lz) = Self::DIRECTION;
        let facing = (normal.0 * lx + normal.1 * ly + normal.2 * lz).max(0.0) / lz;
        let sky = (0.5 + 0.5 * (-normal.1).max(-1.0)).clamp(0.0, 1.0) * normal.2.max(0.0);
        let falloff = self.falloff(x, y);
        [0, 1, 2].map(|channel| Self::KEY[channel] * facing * falloff * 0.78 + Self::FILL[channel] * sky * 0.42)
    }
}

#[cfg(test)]
mod tests {
    use super::WallLight;

    fn light() -> WallLight {
        WallLight::new((0.0, -100.0), 1600.0)
    }

    fn brightness(colour: [f64; 3]) -> f64 {
        colour.iter().sum::<f64>() / 3.0
    }

    #[test]
    fn stone_near_the_sun_is_brighter_than_stone_far_from_it() {
        let flat = (0.0, 0.0, 1.0);

        assert!(brightness(light().shine(flat, 300.0, 200.0)) > brightness(light().shine(flat, 1400.0, 900.0)) * 1.3);
    }

    #[test]
    fn a_face_turned_toward_the_sun_is_warm_and_one_turned_away_is_cool_and_dark() {
        let toward = light().shine((-0.6, -0.4, 0.69), 500.0, 400.0);
        let away = light().shine((0.6, 0.4, 0.69), 500.0, 400.0);

        assert!(toward[0] > toward[2] * 1.2);
        assert!(away[2] > away[0] && brightness(away) < brightness(toward) * 0.5);
    }

    #[test]
    fn a_flat_face_near_the_sun_is_lit_about_as_the_photo_was() {
        let flat = brightness(light().shine((0.0, 0.0, 1.0), 200.0, 100.0));

        assert!((0.85..=1.2).contains(&flat), "{flat}");
    }
}
