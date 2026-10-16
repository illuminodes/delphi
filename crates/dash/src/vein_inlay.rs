use crate::{StoneRecipe, StoneRgb, ValueNoise, VeinPoint};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VeinInlay {
    recipe: StoneRecipe,
    noise: ValueNoise,
}

impl VeinInlay {
    const PINCH_SCALE: f64 = 55.0;
    const PINCH_LOW: f64 = 0.45;
    const PINCH_RANGE: f64 = 1.0;
    const RAGGED_SCALE: f64 = 7.0;
    const RAGGED: f64 = 0.9;
    const EDGE: f64 = 0.55;
    const RIM_WIDTH: f64 = 1.1;
    const SEEP: f64 = 0.18;
    const GRAIN: f64 = 0.09;

    #[must_use]
    pub const fn new(recipe: StoneRecipe, noise: ValueNoise) -> Self {
        Self { recipe, noise }
    }

    #[must_use]
    pub fn half_width(&self, point: VeinPoint) -> f64 {
        let pinch = 0.5 + 0.5 * self.noise.fractal(point.along / Self::PINCH_SCALE + 211.0, 3.7, 3);
        let ragged = Self::RAGGED * self.noise.fractal(point.along / Self::RAGGED_SCALE, point.offset.signum() * 17.0, 2);
        (self.recipe.vein_width * (Self::PINCH_LOW + Self::PINCH_RANGE * pinch) + ragged).max(0.0) * point.strength
    }

    #[must_use]
    pub fn over(&self, ground: StoneRgb, point: VeinPoint) -> StoneRgb {
        let edge = self.half_width(point);
        let distance = point.offset.abs();
        let inside = Self::smoothstep(edge + Self::EDGE, edge - Self::EDGE, distance) * edge.min(1.0);
        let outside = distance - edge;
        let rim = point.strength * self.recipe.rim_strength * (-(outside / Self::RIM_WIDTH).powi(2)).exp();
        let core = StoneRgb::from(self.recipe.vein_core);
        let grain = self.noise.fractal(point.along / 9.0 + 77.0, point.offset / 2.5, 3);
        let depth = if edge > 0.0 { (distance / edge).min(1.0).powi(3) } else { 1.0 };
        let fill = core
            .lit(1.0 + Self::GRAIN * grain)
            .mix(ground, Self::SEEP * (0.5 + 0.5 * grain))
            .mix(StoneRgb::from(self.recipe.vein_rim), 0.3 * depth);
        ground.mix(StoneRgb::from(self.recipe.vein_rim), rim * (1.0 - inside)).mix(fill, inside)
    }

    fn smoothstep(low: f64, high: f64, value: f64) -> f64 {
        let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::VeinInlay;
    use crate::{HexColor, StoneRecipe, StoneRgb, ValueNoise, VeinPoint};

    struct InlayFixture;

    impl InlayFixture {
        fn inlay() -> VeinInlay {
            let recipe = StoneRecipe {
                base: HexColor::rgb([120, 90, 60]),
                bands: &[],
                band_spacing: 10.0,
                band_softness: 0.3,
                clouding: 0.0,
                cloudiness: 0.0,
                turbulence: 0.0,
                branch_colour: HexColor::rgb([255, 255, 255]),
                branch_count: 0,
                branch_width: 1.0,
                vein_core: HexColor::rgb([250, 245, 235]),
                vein_rim: HexColor::rgb([30, 15, 5]),
                vein_width: 4.0,
                rim_strength: 0.8,
            };
            VeinInlay::new(recipe, ValueNoise::new(2))
        }

        fn ground() -> StoneRgb {
            StoneRgb::new([120.0, 90.0, 60.0])
        }

        fn at(offset: f64, along: f64) -> VeinPoint {
            VeinPoint { offset, along, strength: 1.0 }
        }
    }

    #[test]
    fn the_middle_of_the_vein_is_the_vein_colour() {
        let centre = InlayFixture::inlay().over(InlayFixture::ground(), InlayFixture::at(0.0, 30.0));

        assert!(centre.lightness() > 0.8, "{centre:?}");
    }

    #[test]
    fn just_outside_the_edge_is_darker_than_the_stone() {
        let inlay = InlayFixture::inlay();
        let point = InlayFixture::at(0.0, 30.0);
        let edge = inlay.half_width(point);
        let rim = inlay.over(InlayFixture::ground(), InlayFixture::at(edge + 0.8, 30.0));

        assert!(rim.lightness() < InlayFixture::ground().lightness() - 0.05, "{rim:?}");
    }

    #[test]
    fn the_edge_is_crisp() {
        let inlay = InlayFixture::inlay();
        let edge = inlay.half_width(InlayFixture::at(3.0, 30.0));
        let inside = inlay.over(InlayFixture::ground(), InlayFixture::at(edge - 1.0, 30.0));
        let outside = inlay.over(InlayFixture::ground(), InlayFixture::at(edge + 1.0, 30.0));

        assert!(inside.lightness() - outside.lightness() > 0.35, "{inside:?} {outside:?}");
    }

    #[test]
    fn far_from_the_vein_the_stone_shows_untouched() {
        let far = InlayFixture::inlay().over(InlayFixture::ground(), InlayFixture::at(15.0, 30.0));

        assert_eq!(far.rgba(), InlayFixture::ground().rgba());
    }

    #[test]
    fn the_vein_pinches_and_swells_along_its_length() {
        let inlay = InlayFixture::inlay();
        let widths: Vec<f64> = (0..400).map(|along| inlay.half_width(InlayFixture::at(0.5, f64::from(along) * 2.0))).collect();
        let (low, high) = widths.iter().fold((f64::MAX, f64::MIN), |(low, high), width| (low.min(*width), high.max(*width)));

        assert!(low > 0.8 && high > low * 1.8, "{low} {high}");
    }

    #[test]
    fn the_vein_vanishes_where_the_line_has_tapered_away() {
        let faded = VeinPoint { strength: 0.0, ..InlayFixture::at(0.0, 30.0) };

        assert_eq!(InlayFixture::inlay().over(InlayFixture::ground(), faded).rgba(), InlayFixture::ground().rgba());
    }
}
