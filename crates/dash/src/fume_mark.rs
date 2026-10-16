use crate::{FumePolygon, GasCloud, HexColor, VesselMask};

#[derive(Debug, Clone, PartialEq)]
pub enum FumeMark {
    Glow { x: f64, y: f64, radius: f64, colour: HexColor, alpha: f64 },
    Fill { shape: FumePolygon, colour: HexColor, alpha: f64, below: Option<f64> },
    Gas { cloud: GasCloud, mask: VesselMask },
    Picture { source: &'static str, left: f64, top: f64, width: f64, height: f64, alpha: f64 },
}

impl FumeMark {
    #[must_use]
    pub fn coverage(&self, x: f64, y: f64) -> f64 {
        match self {
            Self::Glow { x: centre_x, y: centre_y, radius, alpha, .. } => {
                let reach = (x - centre_x).hypot(y - centre_y) / radius;
                if reach >= 1.0 { 0.0 } else { alpha * (1.0 - reach * reach).powi(2) }
            }
            Self::Fill { shape, alpha, below, .. } => {
                let under = below.is_none_or(|line| y >= line);
                if under && shape.contains(x, y) { *alpha } else { 0.0 }
            }
            Self::Picture { .. } => 0.0,
            Self::Gas { cloud, .. } => cloud.density_at(x, y),
        }
    }

    #[must_use]
    pub fn paint(&self, under: [f64; 3], x: f64, y: f64) -> [f64; 3] {
        let coverage = self.coverage(x, y);
        if coverage <= 0.0 {
            return under;
        }
        let ink = self.colour().channels().map(f64::from);
        match self {
            Self::Glow { .. } | Self::Gas { .. } => [0, 1, 2].map(|channel| (under[channel] + ink[channel] * coverage).min(255.0)),
            Self::Fill { .. } | Self::Picture { .. } => [0, 1, 2].map(|channel| under[channel] + (ink[channel] - under[channel]) * coverage),
        }
    }

    #[must_use]
    pub const fn colour(&self) -> HexColor {
        match self {
            Self::Glow { colour, .. } | Self::Fill { colour, .. } => *colour,
            Self::Gas { cloud, .. } => cloud.colour,
            Self::Picture { .. } => HexColor::rgb([0, 0, 0]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FumeMark;
    use crate::{FumePolygon, HexColor};

    const INK: HexColor = HexColor::rgb([200, 100, 50]);

    fn square() -> FumePolygon {
        FumePolygon::new(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])
    }

    #[test]
    fn a_glow_is_strongest_at_its_centre_and_gone_at_its_rim() {
        let glow = FumeMark::Glow { x: 0.0, y: 0.0, radius: 10.0, colour: INK, alpha: 0.8 };

        assert!((glow.coverage(0.0, 0.0) - 0.8).abs() < 1e-9);
        assert!(glow.coverage(5.0, 0.0) < 0.8 && glow.coverage(5.0, 0.0) > 0.0);
        assert!(glow.coverage(10.0, 0.0).abs() < 1e-9);
    }

    #[test]
    fn a_glow_adds_light_and_a_fill_covers_what_is_under_it() {
        let glow = FumeMark::Glow { x: 0.0, y: 0.0, radius: 10.0, colour: INK, alpha: 1.0 };
        let fill = FumeMark::Fill { shape: square(), colour: INK, alpha: 1.0, below: None };

        assert_eq!(glow.paint([100.0, 200.0, 0.0], 0.0, 0.0), [255.0, 255.0, 50.0]);
        assert_eq!(fill.paint([1.0, 2.0, 3.0], 5.0, 5.0), [200.0, 100.0, 50.0]);
        assert_eq!(fill.paint([1.0, 2.0, 3.0], 50.0, 5.0), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_fill_with_a_line_covers_only_below_it() {
        let fill = FumeMark::Fill { shape: square(), colour: INK, alpha: 1.0, below: Some(6.0) };

        assert!(fill.coverage(5.0, 3.0).abs() < 1e-9);
        assert!((fill.coverage(5.0, 8.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_picture_leaves_its_pixels_to_the_renderer() {
        let picture = FumeMark::Picture { source: "jar.png", left: 0.0, top: 0.0, width: 10.0, height: 10.0, alpha: 0.5 };

        assert_eq!(picture.paint([1.0, 2.0, 3.0], 5.0, 5.0), [1.0, 2.0, 3.0]);
    }
}
