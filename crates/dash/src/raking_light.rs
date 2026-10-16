#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RakingLight {
    elevation: f64,
}

impl RakingLight {
    const REST: f64 = -0.6;
    const SWING: f64 = 0.45;
    const LIFT: f64 = 1.0;

    #[must_use]
    pub const fn new(elevation_degrees: f64) -> Self {
        Self { elevation: elevation_degrees.to_radians() }
    }

    #[must_use]
    pub fn toward(&self, pointer: Option<(f64, f64)>) -> [f64; 3] {
        let across = pointer.map_or(Self::REST, |(u, _)| Self::REST + (u.clamp(0.0, 1.0) - 0.5) * 2.0 * Self::SWING);
        let length = across.hypot(Self::LIFT);
        let flat = self.elevation.cos();
        [flat * across / length, flat * Self::LIFT / length, self.elevation.sin()]
    }
}

#[cfg(test)]
mod tests {
    use super::RakingLight;

    #[test]
    fn at_rest_the_light_comes_from_the_upper_left() {
        let [across, up, out] = RakingLight::new(32.0).toward(None);

        assert!(across < 0.0 && up > 0.0 && out > 0.0);
    }

    #[test]
    fn the_light_swings_across_with_the_pointer() {
        let light = RakingLight::new(32.0);

        assert!(light.toward(Some((0.0, 0.5)))[0] < light.toward(Some((1.0, 0.5)))[0]);
    }

    #[test]
    fn the_light_never_comes_from_below_the_slab_so_cuts_never_read_as_raised() {
        let light = RakingLight::new(32.0);
        for pointer in [None, Some((0.0, 1.0)), Some((1.0, 1.0)), Some((0.5, 0.0)), Some((1.0, 0.0))] {
            let [across, up, _] = light.toward(pointer);
            assert!(up > 0.0 && across.atan2(up).abs() < 50_f64.to_radians(), "{pointer:?}: across {across}, up {up}");
        }
    }

    #[test]
    fn the_light_always_rakes_at_the_same_height_above_the_stone() {
        let light = RakingLight::new(32.0);
        for pointer in [None, Some((0.0, 0.0)), Some((0.8, 0.2)), Some((1.0, 1.0))] {
            let [across, up, out] = light.toward(pointer);
            assert!(((across * across + up * up + out * out).sqrt() - 1.0).abs() < 1e-9, "{pointer:?}");
            assert!((out - 32_f64.to_radians().sin()).abs() < 1e-9, "{pointer:?}");
        }
    }
}
