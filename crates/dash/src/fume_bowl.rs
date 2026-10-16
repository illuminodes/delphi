use std::f64::consts::PI;

use crate::FumePolygon;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FumeBowl {
    pub centre: f64,
    pub rim: f64,
    pub half_width: f64,
    pub depth: f64,
    pub base: f64,
}

impl FumeBowl {
    pub const PICTURE: &'static str = "fumes/bronze-tripod.png";
    const STEPS: usize = 32;
    const LIP: f64 = 0.07;
    const ASPECT: f64 = 2145.0 / 3111.0;
    const BASIN_SHARE: f64 = 0.372;
    const BASIN_CENTRE: f64 = 0.491;
    const BASIN_FLOOR: f64 = 0.063;
    const MOUTH: f64 = 0.009;

    #[must_use]
    pub fn standing(centre: f64, base: f64, height: f64) -> Self {
        let top = base - height;
        let rim = top + height * Self::MOUTH;
        Self { centre, rim, half_width: height * Self::ASPECT * Self::BASIN_SHARE * 0.5, depth: height * (Self::BASIN_FLOOR - Self::MOUTH), base }
    }

    #[must_use]
    pub fn picture(&self) -> (f64, f64, f64, f64) {
        let height = (self.rim - self.base) / (Self::MOUTH - 1.0);
        let width = height * Self::ASPECT;
        (self.centre - width * Self::BASIN_CENTRE, self.base - height, width, height)
    }

    #[must_use]
    pub fn reach(height: f64) -> f64 {
        height * Self::ASPECT * Self::BASIN_CENTRE.max(1.0 - Self::BASIN_CENTRE)
    }

    #[must_use]
    pub const fn floor(&self) -> f64 {
        self.rim + self.depth
    }

    #[must_use]
    pub fn lip_height(&self) -> f64 {
        self.half_width * Self::LIP
    }

    #[must_use]
    pub fn body(&self) -> FumePolygon {
        FumePolygon::new(
            (0..=Self::STEPS)
                .map(|step| {
                    let angle = PI * step as f64 / Self::STEPS as f64;
                    (self.centre + self.half_width * angle.cos(), self.rim + self.depth * angle.sin().powf(0.8))
                })
                .collect(),
        )
    }

    #[must_use]
    pub fn mouth(&self) -> FumePolygon {
        FumePolygon::new(
            (0..Self::STEPS * 2)
                .map(|step| {
                    let angle = PI * step as f64 / Self::STEPS as f64;
                    (self.centre + self.half_width * angle.cos(), self.rim + self.lip_height() * angle.sin())
                })
                .collect(),
        )
    }

    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.body().contains(x, y) || self.mouth().contains(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::FumeBowl;

    fn bowl() -> FumeBowl {
        FumeBowl::standing(300.0, 400.0, 300.0)
    }

    #[test]
    fn the_bowl_opens_upward_with_its_belly_below_the_rim() {
        let bowl = bowl();
        let body = bowl.body();

        assert!(body.contains(300.0, bowl.rim + bowl.depth * 0.5));
        assert!(!body.contains(300.0, bowl.rim - 2.0));
        assert!(!body.contains(bowl.centre + bowl.half_width * 0.95, bowl.floor() - 1.0));
        assert!((body.bottom() - bowl.floor()).abs() < 1e-9);
    }

    #[test]
    fn the_mouth_is_a_flat_ellipse_across_the_rim() {
        let bowl = bowl();
        let mouth = bowl.mouth();

        assert!(mouth.contains(300.0, bowl.rim) && mouth.contains(300.0 + bowl.half_width * 0.9, bowl.rim));
        assert!(!mouth.contains(300.0, bowl.rim - bowl.lip_height() - 1.0));
    }

    #[test]
    fn the_picture_stands_on_the_base_with_its_basin_over_the_centre() {
        let bowl = bowl();
        let (left, top, width, height) = bowl.picture();

        assert!((top + height - 400.0).abs() < 1e-9 && (height - 300.0).abs() < 1e-9);
        assert!((top - (bowl.rim - height * FumeBowl::MOUTH)).abs() < 1e-9);
        assert!((left + width * FumeBowl::BASIN_CENTRE - 300.0).abs() < 1e-9);
        assert!((bowl.half_width * 2.0 - width * FumeBowl::BASIN_SHARE).abs() < 1e-9);
        assert!(left + width <= 300.0 + FumeBowl::reach(height) + 1e-9 && left >= 300.0 - FumeBowl::reach(height) - 1e-9);
    }
}
