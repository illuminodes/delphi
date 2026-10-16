#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VesselMask {
    pub source: &'static str,
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl VesselMask {
    #[must_use]
    pub fn place(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (across, down) = ((x - self.left) / self.width, (y - self.top) / self.height);
        ((0.0..1.0).contains(&across) && (0.0..1.0).contains(&down)).then_some((across, down))
    }
}

#[cfg(test)]
mod tests {
    use super::VesselMask;

    const MASK: VesselMask = VesselMask { source: "mask.png", left: 100.0, top: 50.0, width: 40.0, height: 80.0 };

    #[test]
    fn a_point_on_the_jar_is_placed_as_a_fraction_of_the_mask() {
        assert_eq!(MASK.place(100.0, 50.0), Some((0.0, 0.0)));
        assert_eq!(MASK.place(120.0, 110.0), Some((0.5, 0.75)));
    }

    #[test]
    fn a_point_beside_the_jar_falls_off_the_mask() {
        assert_eq!(MASK.place(99.0, 60.0), None);
        assert_eq!(MASK.place(140.0, 60.0), None);
        assert_eq!(MASK.place(110.0, 130.0), None);
    }
}
