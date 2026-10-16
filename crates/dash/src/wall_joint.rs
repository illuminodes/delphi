#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallJoint {
    pub bottom: (f64, f64),
    pub knee: (f64, f64),
    pub top: (f64, f64),
}

impl WallJoint {
    #[must_use]
    pub const fn new(bottom: (f64, f64), knee: (f64, f64), top: (f64, f64)) -> Self {
        Self { bottom, knee, top }
    }

    #[must_use]
    pub fn straight(bottom: (f64, f64), top: (f64, f64)) -> Self {
        Self::new(bottom, ((bottom.0 + top.0) * 0.5, (bottom.1 + top.1) * 0.5), top)
    }

    #[must_use]
    pub const fn shifted(self, dx: f64, dy: f64) -> Self {
        Self::new((self.bottom.0 + dx, self.bottom.1 + dy), (self.knee.0 + dx, self.knee.1 + dy), (self.top.0 + dx, self.top.1 + dy))
    }

    #[must_use]
    pub fn leftmost(&self) -> f64 {
        self.bottom.0.min(self.knee.0).min(self.top.0)
    }

    #[must_use]
    pub fn rightmost(&self) -> f64 {
        self.bottom.0.max(self.knee.0).max(self.top.0)
    }
}

#[cfg(test)]
mod tests {
    use super::WallJoint;

    #[test]
    fn a_straight_joint_bends_nowhere() {
        let joint = WallJoint::straight((100.0, 200.0), (120.0, 100.0));

        assert_eq!(joint.knee, (110.0, 150.0));
    }

    #[test]
    fn the_extremes_include_the_knee() {
        let joint = WallJoint::new((100.0, 200.0), (90.0, 150.0), (120.0, 100.0));

        assert!((joint.leftmost() - 90.0).abs() < 1e-9 && (joint.rightmost() - 120.0).abs() < 1e-9);
        assert_eq!(joint.shifted(1.0, 2.0).knee, (91.0, 152.0));
    }
}
