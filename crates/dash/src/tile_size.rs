use crate::SplitAxis;

/// A size in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileSize {
    pub width: f64,
    pub height: f64,
}

impl TileSize {
    pub const ZERO: Self = Self { width: 0.0, height: 0.0 };

    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub const fn along(self, axis: SplitAxis) -> f64 {
        match axis {
            SplitAxis::Row => self.width,
            SplitAxis::Column => self.height,
        }
    }

    /// The smallest area holding `self` and `other` side by side along `axis`.
    #[must_use]
    pub fn beside(self, other: Self, axis: SplitAxis) -> Self {
        match axis {
            SplitAxis::Row => Self::new(self.width + other.width, self.height.max(other.height)),
            SplitAxis::Column => Self::new(self.width.max(other.width), self.height + other.height),
        }
    }

    /// Width over height, or `None` before the size is known.
    #[must_use]
    pub fn aspect(self) -> Option<f64> {
        (self.width > 0.0 && self.height > 0.0).then(|| self.width / self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::TileSize;
    use crate::SplitAxis;

    #[test]
    fn side_by_side_adds_along_the_axis_and_keeps_the_larger_across() {
        let (a, b) = (TileSize::new(100.0, 50.0), TileSize::new(30.0, 80.0));

        assert_eq!(a.beside(b, SplitAxis::Row), TileSize::new(130.0, 80.0));
        assert_eq!(a.beside(b, SplitAxis::Column), TileSize::new(100.0, 130.0));
        assert!((a.along(SplitAxis::Column) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn an_unmeasured_size_has_no_aspect() {
        assert_eq!(TileSize::ZERO.aspect(), None);
        assert_eq!(TileSize::new(200.0, 100.0).aspect(), Some(2.0));
    }
}
